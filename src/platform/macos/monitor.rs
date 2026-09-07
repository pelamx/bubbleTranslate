//! Watches for the gestures that finish a text selection.
//!
//! A `CGEventTap` on its own run-loop thread sees mouse and key events
//! system-wide. Rather than polling the focused element (expensive, and it
//! fires mid-drag), the tap waits for the moment a selection is *completed*:
//! a mouse-up that ended a drag or a multi-click, or a key-up from a
//! shift-navigation.
//!
//! The gesture filter matters more than it looks. Capture can fall back to
//! synthesizing Cmd+C, so triggering on every mouse-up would fire a copy on
//! every single click anywhere in the OS. Only drags past a few points and
//! double/triple clicks get through.

use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use core_foundation::base::TCFType;
use core_foundation::mach_port::CFMachPortRef;
use core_foundation::runloop::CFRunLoop;
use core_foundation_sys::runloop::kCFRunLoopCommonModes;
use core_graphics::event::{
    CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, CallbackResult, EventField,
};

use crate::platform::Trigger;
use crate::platform::capture;

/// Minimum drag distance, in points, before a mouse-up counts as a selection
/// rather than a click.
const DRAG_THRESHOLD: f64 = 6.0;

/// Keys that extend a selection when Shift is held.
const NAVIGATION_KEYS: [i64; 8] = [
    123, // left
    124, // right
    125, // down
    126, // up
    115, // home
    119, // end
    116, // page up
    121, // page down
];
const KEYCODE_A: i64 = 0;

// The tap's own port, so its callback can switch it back on. Only ever
// touched from the monitor thread, which is where both the tap and the
// callback live.
thread_local! {
    static TAP_PORT: Cell<CFMachPortRef> = const { Cell::new(std::ptr::null_mut()) };
}

unsafe extern "C" {
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
}

/// Set while the pointer is inside the bubble, so interacting with our own
/// window never kicks off another capture of the app behind it.
static PAUSED: AtomicBool = AtomicBool::new(false);

/// No second source here: on macOS the clipboard is a way of *reading* a
/// selection, not a way of noticing one, and that is already what
/// `clipboard_fallback` controls.
pub fn set_watch_clipboard(_watch: bool) {}

pub fn set_paused(paused: bool) {
    PAUSED.store(paused, Ordering::Relaxed);
}

/// Starts the tap on a dedicated thread and returns immediately.
///
/// Returns an error only if the tap could not be created, which in practice
/// means Accessibility permission has not been granted yet.
pub fn spawn(on_trigger: impl Fn(Trigger) + Send + 'static) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("selection-monitor".into())
        .spawn(move || run(on_trigger))
        .map(|_| ())
}

fn run(on_trigger: impl Fn(Trigger) + Send + 'static) {
    // Where the current drag started, and what the pasteboard looked like then.
    // `None` between gestures.
    let press: Mutex<Option<((f64, f64), isize)>> = Mutex::new(None);

    let callback = move |_proxy: _,
                         event_type: CGEventType,
                         event: &core_graphics::event::CGEvent| {
        // macOS switches off a tap whose callback took too long, and then
        // simply stops delivering events — no error, no further callbacks
        // beyond this one notification. Catching it and switching the tap back
        // on is the difference between a hiccup and selection detection being
        // dead for the rest of the session.
        if matches!(
            event_type,
            CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
        ) {
            crate::trace!("tap disabled by system ({event_type:?}) — re-enabling");
            TAP_PORT.with(|port| {
                let port = port.get();
                if !port.is_null() {
                    unsafe { CGEventTapEnable(port, true) };
                }
            });
            return CallbackResult::Keep;
        }

        // Never react to the Cmd+C we post ourselves, or the gesture would
        // feed itself.
        if capture::is_synthesizing() || capture::is_marked_synthetic(event) {
            return CallbackResult::Keep;
        }
        if PAUSED.load(Ordering::Relaxed) {
            crate::trace!("event ignored: pointer is over the bubble");
            return CallbackResult::Keep;
        }

        match event_type {
            CGEventType::LeftMouseDown => {
                let p = event.location();
                crate::trace!("mouse-down at ({:.0}, {:.0})", p.x, p.y);
                // Sampled here because a copy-on-select interface writes the
                // pasteboard at mouse-up; by the time the capture runs there is
                // no longer a "before" to compare against.
                *press.lock().unwrap() = Some(((p.x, p.y), capture::pasteboard_change_count()));
            }
            CGEventType::LeftMouseUp => {
                let p = event.location();
                let pressed = press.lock().unwrap().take();
                let dragged = pressed
                    .map(|((x, y), _)| {
                        ((p.x - x).powi(2) + (p.y - y).powi(2)).sqrt() > DRAG_THRESHOLD
                    })
                    .unwrap_or(false);
                // Click state 2 is a double-click (word), 3 a triple-click
                // (paragraph); both select without any drag.
                let multi_click =
                    event.get_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE) >= 2;

                crate::trace!(
                    "mouse-up  drag={} multi_click={} -> {}",
                    if dragged { "yes" } else { "no " },
                    multi_click,
                    if dragged || multi_click {
                        "TRIGGER"
                    } else {
                        "ignored"
                    },
                );
                if dragged || multi_click {
                    on_trigger(Trigger {
                        at: Some((p.x, p.y)),
                        clipboard_before: pressed.map(|(_, count)| count),
                    });
                }
            }
            CGEventType::KeyUp => {
                let flags = event.get_flags();
                let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
                let shift_select = flags.contains(CGEventFlags::CGEventFlagShift)
                    && NAVIGATION_KEYS.contains(&keycode);
                let select_all =
                    flags.contains(CGEventFlags::CGEventFlagCommand) && keycode == KEYCODE_A;

                if shift_select || select_all {
                    crate::trace!("key-up    keycode={keycode} -> TRIGGER");
                    let p = event.location();
                    on_trigger(Trigger {
                        at: Some((p.x, p.y)),
                        clipboard_before: None,
                    });
                }
            }
            _ => {}
        }

        CallbackResult::Keep
    };

    crate::trace!("installing event tap...");
    // Built by hand rather than with `with_enabled`, which keeps the tap to
    // itself: the callback needs the port to re-enable the tap above.
    //
    // SAFETY: `new_unchecked` requires the callback to run only on the thread
    // the tap is installed on, and the tap to outlive its use. Both hold — the
    // run loop below is this thread's, and `tap` owns the callback until this
    // function returns, which only happens once that run loop stops.
    let tap = unsafe {
        CGEventTap::new_unchecked(
            CGEventTapLocation::Session,
            CGEventTapPlacement::HeadInsertEventTap,
            // Listen-only: we never modify or swallow the user's events, which
            // also means a slow callback can never stall their typing.
            CGEventTapOptions::ListenOnly,
            vec![
                CGEventType::LeftMouseDown,
                CGEventType::LeftMouseUp,
                CGEventType::KeyUp,
            ],
            callback,
        )
    };

    let Ok(tap) = tap else {
        eprintln!(
            "bubbleTranslate: could not install the event tap — grant Accessibility \
             permission in System Settings and restart."
        );
        return;
    };

    let Ok(source) = tap.mach_port().create_runloop_source(0) else {
        eprintln!("bubbleTranslate: could not attach the event tap to the run loop.");
        return;
    };
    CFRunLoop::get_current().add_source(&source, unsafe { kCFRunLoopCommonModes });
    TAP_PORT.with(|port| port.set(tap.mach_port().as_concrete_TypeRef()));
    tap.enable();
    crate::trace!("event tap installed");
    CFRunLoop::run_current();
}
