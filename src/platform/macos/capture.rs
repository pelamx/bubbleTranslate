//! Reading whatever text is selected in whatever app is frontmost.
//!
//! Two strategies, tried in order:
//!
//! 1. **Accessibility API** — ask the focused UI element for `AXSelectedText`.
//!    Clean and instant, and it never touches the pasteboard. Native Cocoa
//!    apps, Safari and most text fields answer this.
//! 2. **Copy-on-select** — some interfaces put the selection on the pasteboard
//!    themselves as the gesture ends, and leave nothing selected behind: a
//!    terminal set to copy on select, and any full-screen TUI that takes the
//!    mouse over from the terminal it runs in. Cmd+C would come back empty
//!    there, but the text is already on the pasteboard.
//! 3. **Synthetic Cmd+C** — post a command-C keystroke and watch the general
//!    pasteboard's change count. Terminal.app, most PDF viewers and Electron
//!    apps expose nothing over AX but all copy just fine, so this is what
//!    makes "anywhere" actually mean anywhere. The previous pasteboard text is
//!    put back afterwards.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, EventField};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::NSString;

use crate::platform::{Capture, CaptureSource, Readiness};

type AXUIElementRef = *mut c_void;
const AX_SUCCESS: i32 = 0;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout: f32) -> i32;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
}

/// Virtual keycode for "c" on any layout (keycodes are physical, not logical).
const KEYCODE_C: u16 = 8;

/// Stamped into every event we synthesize so the selection monitor can tell
/// our own Cmd+C apart from the user's keystrokes.
pub const SYNTHETIC_MARKER: i64 = 0x6274_6473; // "btds"

/// Belt-and-braces companion to the marker: the tap ignores everything while
/// this is set, which also covers events the marker cannot reach (the
/// flags-changed events implied by holding Command).
static SYNTHESIZING: AtomicBool = AtomicBool::new(false);

pub fn is_synthesizing() -> bool {
    SYNTHESIZING.load(Ordering::SeqCst)
}

/// Reports whether the user has ticked this binary in System Settings →
/// Privacy & Security → Accessibility, popping the system's "open Settings?"
/// dialog if they have not. Without the permission both capture strategies
/// silently return nothing: AX refuses to answer and posted events are
/// dropped.
pub fn readiness() -> Readiness {
    let trusted = unsafe {
        let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
        let value = core_foundation::boolean::CFBoolean::true_value();
        let options = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);
        AXIsProcessTrustedWithOptions(options.as_CFTypeRef() as *const c_void)
    };
    if trusted {
        return Readiness::ready();
    }
    Readiness::blocked(
        "Accessibility permission is off — selections cannot be read. Enable \
         bubbleTranslate in System Settings › Privacy & Security › Accessibility, \
         then restart.",
        "Selections cannot be read. Enable bubbleTranslate in System Settings › \
         Privacy & Security › Accessibility, then restart the app — the event tap \
         is installed at startup.",
    )
}

/// Grabs the current selection, or `None` when there isn't one.
///
/// `allow_clipboard` gates the Cmd+C strategy: with it off, apps that don't
/// speak AX simply return nothing instead of having their pasteboard borrowed.
pub fn selected_text(allow_clipboard: bool, clipboard_before: Option<isize>) -> Option<Capture> {
    if let Some(text) = accessibility_selection() {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return Some(Capture {
                text: trimmed.to_string(),
                via: CaptureSource::Accessibility,
            });
        }
    }
    if !allow_clipboard {
        return None;
    }

    // Checked before Cmd+C rather than after: where it applies, Cmd+C has
    // nothing left to copy and would only spend the full budget failing, and
    // the keystroke would land in an interface that never asked for it.
    // Trusting the pasteboard only when it moved *during this gesture* is what
    // keeps an unrelated, older clipboard from being translated on a stray
    // double-click.
    if let Some(before) = clipboard_before {
        let pasteboard = NSPasteboard::generalPasteboard();
        if pasteboard.changeCount() != before {
            if let Some(text) = read_pasteboard_string(&pasteboard) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    crate::trace!("clipboard the app copied it itself during the gesture");
                    return Some(Capture {
                        text: trimmed.to_string(),
                        via: CaptureSource::Clipboard,
                    });
                }
            }
        }
    }

    clipboard_selection().map(|text| Capture {
        text,
        via: CaptureSource::Clipboard,
    })
}

/// The general pasteboard's change count, sampled when a gesture begins so the
/// capture above can tell a selection the app copied itself from the clipboard
/// the user already had.
pub fn pasteboard_change_count() -> isize {
    NSPasteboard::generalPasteboard().changeCount()
}

// -- Strategy 1: Accessibility ---------------------------------------------

fn accessibility_selection() -> Option<String> {
    unsafe {
        let system = AXUIElementCreateSystemWide();
        if system.is_null() {
            return None;
        }
        // An unresponsive app must not freeze the capture thread; bail out
        // fast and let the clipboard path take over.
        AXUIElementSetMessagingTimeout(system, 0.25);

        let focused = copy_attribute(system, "AXFocusedUIElement");
        CFRelease(system as CFTypeRef);
        let focused = focused?;

        let selected = copy_attribute(focused as AXUIElementRef, "AXSelectedText");
        CFRelease(focused);

        let selected = selected?;
        let text = cf_string_to_owned(selected as CFStringRef);
        CFRelease(selected);
        text
    }
}

/// Reads one AX attribute, returning a +1 reference the caller must release.
unsafe fn copy_attribute(element: AXUIElementRef, attribute: &str) -> Option<CFTypeRef> {
    if element.is_null() {
        return None;
    }
    let name = CFString::new(attribute);
    let mut value: CFTypeRef = std::ptr::null();
    let err =
        unsafe { AXUIElementCopyAttributeValue(element, name.as_concrete_TypeRef(), &mut value) };
    if err != AX_SUCCESS || value.is_null() {
        return None;
    }
    Some(value)
}

unsafe fn cf_string_to_owned(s: CFStringRef) -> Option<String> {
    if s.is_null() {
        return None;
    }
    // AXSelectedText is documented as a string, but a misbehaving app can hand
    // back another type; CFString::wrap_under_get_rule would then reinterpret
    // it. Going through to_string on a borrowed wrapper is safe enough here
    // because we only ever pass it values fetched from AXSelectedText.
    let cf = unsafe { CFString::wrap_under_get_rule(s) };
    Some(cf.to_string())
}

// -- Strategy 2: synthetic Cmd+C -------------------------------------------

fn clipboard_selection() -> Option<String> {
    let pasteboard = NSPasteboard::generalPasteboard();
    let before_count = pasteboard.changeCount();
    let previous = read_pasteboard_string(&pasteboard);

    SYNTHESIZING.store(true, Ordering::SeqCst);
    let started = Instant::now();
    let posted = post_command_c();
    // The flag stays up a moment past the post so the tap also drops the
    // trailing key-up and flags-changed events.
    let result = if posted {
        wait_for_pasteboard_change(&pasteboard, before_count, COPY_BUDGET)
    } else {
        None
    };
    crate::trace!(
        "clipboard posted={posted} changed={} after {}ms",
        result.is_some(),
        started.elapsed().as_millis(),
    );
    std::thread::sleep(Duration::from_millis(30));
    SYNTHESIZING.store(false, Ordering::SeqCst);

    let copied = result
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // Put the user's clipboard back regardless of the outcome. This only
    // restores text; a copied image or file reference is not preserved.
    if copied.is_some() {
        if let Some(previous) = previous {
            write_pasteboard_string(&pasteboard, &previous);
        }
    }

    copied
}

/// How long to wait for the target app to put the copy on the pasteboard.
///
/// Only ever reached when the copy produces nothing: the poll below returns the
/// moment the change count moves, so a responsive app is unaffected by how
/// generous this is. That asymmetry is why it is set well above any observed
/// copy latency — the cost of waiting too long is paid on a background thread,
/// while the cost of giving up too early is a selection that silently vanishes.
const COPY_BUDGET: Duration = Duration::from_millis(1200);

/// Virtual keycode for the left Command key.
const KEYCODE_COMMAND: u16 = 55;

fn post_command_c() -> bool {
    let Ok(source) = CGEventSource::new(CGEventSourceStateID::CombinedSessionState) else {
        return false;
    };

    // The Command key is pressed and released for real, as its own pair of
    // flagsChanged events, rather than only setting the modifier bit on the C
    // keypress. Native Cocoa apps accept the bit on its own, but Chromium —
    // and so every Electron app, VS Code's terminal included — tracks modifier
    // state from these events and treats a C with nothing but the bit set as a
    // plain "c" keystroke: no copy, and a stray character typed into whatever
    // was focused.
    let Ok(cmd_down) = CGEvent::new_keyboard_event(source.clone(), KEYCODE_COMMAND, true) else {
        return false;
    };
    let Ok(cmd_up) = CGEvent::new_keyboard_event(source.clone(), KEYCODE_COMMAND, false) else {
        return false;
    };
    let Ok(down) = CGEvent::new_keyboard_event(source.clone(), KEYCODE_C, true) else {
        return false;
    };
    let Ok(up) = CGEvent::new_keyboard_event(source, KEYCODE_C, false) else {
        return false;
    };

    cmd_down.set_type(CGEventType::FlagsChanged);
    cmd_up.set_type(CGEventType::FlagsChanged);
    cmd_down.set_flags(CGEventFlags::CGEventFlagCommand);
    // Releasing Command leaves no modifiers held.
    cmd_up.set_flags(CGEventFlags::CGEventFlagNull);
    down.set_flags(CGEventFlags::CGEventFlagCommand);
    up.set_flags(CGEventFlags::CGEventFlagCommand);

    for event in [&cmd_down, &down, &up, &cmd_up] {
        event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, SYNTHETIC_MARKER);
    }

    cmd_down.post(CGEventTapLocation::HID);
    // Let the modifier land before the key that depends on it; a chord posted
    // in one burst is read as an unmodified keypress by some apps.
    std::thread::sleep(Duration::from_millis(12));
    down.post(CGEventTapLocation::HID);
    // A gap between down and up; some apps ignore a zero-duration keypress.
    std::thread::sleep(Duration::from_millis(12));
    up.post(CGEventTapLocation::HID);
    std::thread::sleep(Duration::from_millis(12));
    cmd_up.post(CGEventTapLocation::HID);
    true
}

/// Polls the change count instead of sleeping a fixed interval, so a fast app
/// answers in ~20ms while a slow one still gets the full budget.
fn wait_for_pasteboard_change(
    pasteboard: &NSPasteboard,
    before: isize,
    budget: Duration,
) -> Option<String> {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
        if pasteboard.changeCount() != before {
            return read_pasteboard_string(pasteboard);
        }
    }
    None
}

fn read_pasteboard_string(pasteboard: &NSPasteboard) -> Option<String> {
    unsafe {
        pasteboard
            .stringForType(NSPasteboardTypeString)
            .map(|s| s.to_string())
    }
}

fn write_pasteboard_string(pasteboard: &NSPasteboard, value: &str) {
    unsafe {
        pasteboard.clearContents();
        let ns = NSString::from_str(value);
        pasteboard.setString_forType(&ns, NSPasteboardTypeString);
    }
}

/// Puts text on the general pasteboard, for the bubble's copy button.
///
/// The context goes unused here — AppKit owns the pasteboard directly — but it
/// is what the Linux side needs on an X11 session, so the signature is shared.
pub fn set_clipboard(_ctx: &eframe::egui::Context, text: &str) {
    let pasteboard = NSPasteboard::generalPasteboard();
    write_pasteboard_string(&pasteboard, text);
}

/// Whether an event came from our own synthetic copy.
pub fn is_marked_synthetic(event: &CGEvent) -> bool {
    event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA) == SYNTHETIC_MARKER
}
