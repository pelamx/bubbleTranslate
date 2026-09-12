//! Reading whatever text is selected in whatever window is in front.
//!
//! Two strategies, tried in order, and they are the same two the macOS side
//! uses for the same reasons:
//!
//! 1. **UI Automation** — ask the focused element for its `TextPattern`
//!    selection. Instant, and it never touches the clipboard. Win32 edit
//!    controls, RichEdit, WPF, WinUI, Office, Firefox and Chromium-based
//!    browsers all answer this.
//! 2. **Copy-on-select** — some interfaces put the selection on the clipboard
//!    themselves as the gesture ends, and leave nothing selected behind. A
//!    synthetic Ctrl+C would come back empty there, but the text is already on
//!    the clipboard, so a clipboard that *moved during this gesture* is taken
//!    as the answer.
//! 3. **Synthetic Ctrl+C** — press the chord for real with `SendInput` and
//!    watch the clipboard sequence number. This is what makes "anywhere"
//!    actually mean anywhere: a PDF viewer, a Java application and most
//!    custom-drawn interfaces expose nothing over UI Automation but all copy
//!    just fine. The user's clipboard text is put back afterwards.
//!
//! One thing Windows refuses outright, and no strategy gets around it: a
//! process running elevated is walled off from one that is not. User Interface
//! Privilege Isolation drops both the automation call and the synthetic
//! keystroke, silently, in the same way. It cannot be detected in advance —
//! only a selection in an elevated window comes back empty — which is why
//! [`readiness`] does not try to predict it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
    IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationTextPattern, UIA_TextPatternId,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, MapVirtualKeyW, SendInput, VIRTUAL_KEY, VK_C, VK_CONTROL,
    VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};

use crate::platform::{Capture, CaptureSource, Readiness};

/// `CF_UNICODETEXT`. Spelled out rather than imported: the constant has moved
/// between modules across releases of the Windows bindings, and its value is
/// older than any of them.
const CF_UNICODETEXT: u32 = 13;

/// Stamped into every keystroke we synthesize, as `dwExtraInfo`, so the
/// selection monitor can tell our own Ctrl+C apart from the user's.
///
/// The low-level hooks also see an "injected" flag, and that alone would
/// nearly do — but injected is true of every automation tool on the machine,
/// and ignoring somebody else's macro is not the same question as ignoring our
/// own copy.
pub const SYNTHETIC_MARKER: usize = 0x6274_6473; // "btds"

/// Belt-and-braces companion to the marker: the hooks ignore everything while
/// this is set, which also covers the modifier releases below, whose extra
/// info a driver-level filter is free to drop.
static SYNTHESIZING: AtomicBool = AtomicBool::new(false);

pub fn is_synthesizing() -> bool {
    SYNTHESIZING.load(Ordering::SeqCst)
}

/// Nothing to ask for here.
///
/// Windows lets any process read another's selection within the same session
/// and integrity level — there is no permission switch, no prompt, and so
/// nothing this can report. The one case that fails is an elevated window, and
/// see the module note: it is not knowable until a capture comes back empty.
pub fn readiness() -> Readiness {
    Readiness::ready()
}

/// Grabs the current selection, or `None` when there isn't one.
///
/// `allow_clipboard` gates the Ctrl+C strategy: with it off, an application
/// that says nothing over UI Automation simply returns nothing rather than
/// having its clipboard borrowed.
pub fn selected_text(allow_clipboard: bool, clipboard_before: Option<isize>) -> Option<Capture> {
    if let Some(text) = automation_selection() {
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

    // Checked before Ctrl+C rather than after: where it applies, the copy has
    // nothing left to copy and would only spend the full budget failing, and
    // the keystroke would land in an interface that never asked for it.
    // Trusting the clipboard only when it moved *during this gesture* is what
    // keeps an unrelated, older clipboard from being translated on a stray
    // double-click.
    if let Some(before) = clipboard_before
        && clipboard_sequence() != before
        && let Some(text) = read_clipboard_string()
    {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            crate::trace!("clipboard the app copied it itself during the gesture");
            return Some(Capture {
                text: trimmed.to_string(),
                via: CaptureSource::Clipboard,
            });
        }
    }

    clipboard_selection().map(|text| Capture {
        text,
        via: CaptureSource::Clipboard,
    })
}

/// The clipboard's sequence number, sampled when a gesture begins so the
/// capture above can tell a selection the app copied itself from the clipboard
/// the user already had.
///
/// Cheap, and needs no handle: this is the one clipboard question Windows
/// answers without opening it, which matters because opening it fails whenever
/// another process is mid-copy.
pub fn clipboard_sequence() -> isize {
    unsafe { GetClipboardSequenceNumber() as isize }
}

// -- Strategy 1: UI Automation ---------------------------------------------

/// The automation client, built once per thread and kept.
///
/// Building one costs a COM activation, which is far from free on the path a
/// selection takes. The thread-local is also what keeps the object on the
/// apartment it was created in.
fn automation() -> Option<IUIAutomation> {
    thread_local! {
        static CLIENT: Option<IUIAutomation> = create_automation();
    }
    CLIENT.with(|client| client.clone())
}

fn create_automation() -> Option<IUIAutomation> {
    unsafe {
        // Multithreaded, because this runs on the engine thread, which has no
        // message loop: a single-threaded apartment without one deadlocks the
        // first time a call has to be marshalled. The returned status is
        // deliberately not checked — `S_FALSE` means the thread was already
        // initialised, which is a success, and a real failure shows up as an
        // error from the activation on the next line anyway.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        match CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) {
            Ok(client) => Some(client),
            Err(err) => {
                crate::trace!("ui automation unavailable: {err}");
                None
            }
        }
    }
}

/// Asks the focused element what it has selected.
///
/// A discontiguous selection — a column of a spreadsheet, several table cells
/// — arrives as several ranges, and joining them is what the application would
/// itself have put on the clipboard.
///
/// There is no timeout to set: a cross-process automation call carries one of
/// its own and fails with `UIA_E_TIMEOUT` rather than hanging, so an
/// unresponsive application costs a pause here and then falls through to the
/// clipboard.
fn automation_selection() -> Option<String> {
    let automation = automation()?;
    unsafe {
        let focused = match automation.GetFocusedElement() {
            Ok(focused) => focused,
            Err(err) => {
                crate::trace!("automation: nothing focused ({err})");
                return None;
            }
        };
        let pattern =
            match focused.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) {
                Ok(pattern) => pattern,
                Err(err) => {
                    crate::trace!("automation: the focused element has no text ({err})");
                    return None;
                }
            };
        let ranges = pattern.GetSelection().ok()?;
        let count = ranges.Length().ok()?;

        let mut parts = Vec::new();
        for i in 0..count {
            let Ok(range) = ranges.GetElement(i) else {
                continue;
            };
            // -1 asks for the whole range. A selection of an entire document
            // is bounded by `max_chars` further up; a second limit here would
            // only truncate in a place the user cannot see.
            if let Ok(text) = range.GetText(-1) {
                let text = text.to_string();
                if !text.trim().is_empty() {
                    parts.push(text);
                }
            }
        }
        (!parts.is_empty()).then(|| parts.join("\n"))
    }
}

// -- Strategy 2: synthetic Ctrl+C ------------------------------------------

/// How long to wait for the target app to put the copy on the clipboard.
///
/// Only ever reached when the copy produces nothing: the poll below returns
/// the moment the sequence number moves, so a responsive app is unaffected by
/// how generous this is. That asymmetry is why it is set well above any
/// observed copy latency — the cost of waiting too long is paid on a
/// background thread, while the cost of giving up too early is a selection
/// that silently vanishes.
const COPY_BUDGET: Duration = Duration::from_millis(1200);

fn clipboard_selection() -> Option<String> {
    let before = clipboard_sequence();
    let previous = read_clipboard_string();

    SYNTHESIZING.store(true, Ordering::SeqCst);
    let started = Instant::now();
    let posted = post_ctrl_c();
    let result = if posted {
        wait_for_clipboard_change(before, COPY_BUDGET)
    } else {
        None
    };
    crate::trace!(
        "clipboard posted={posted} changed={} after {}ms",
        result.is_some(),
        started.elapsed().as_millis(),
    );
    // The flag stays up a moment past the last keystroke so the hooks also
    // drop the trailing releases.
    std::thread::sleep(Duration::from_millis(30));
    SYNTHESIZING.store(false, Ordering::SeqCst);

    let copied = result
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // Put the user's clipboard back regardless of the outcome. This only
    // restores text; a copied image or file list is not preserved.
    if copied.is_some()
        && let Some(previous) = previous
    {
        write_clipboard_string(&previous);
    }

    copied
}

/// The modifiers that must not still be down when Ctrl+C is sent.
///
/// This is not tidiness. The trigger key is Shift by default and the user is
/// usually still holding it as the gesture ends, which would turn our copy
/// into Ctrl+Shift+C — "inspect element" in every Chromium browser, and
/// something else again in a dozen other applications. Alt and the Windows key
/// are released for the same reason.
const INTERFERING: [VIRTUAL_KEY; 4] = [VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN];

/// Whether a key is physically down right now.
fn held(key: VIRTUAL_KEY) -> bool {
    // The high bit is the current physical state; the low bit is a "pressed
    // since last asked" latch that must not be read as held.
    (unsafe { GetAsyncKeyState(key.0 as i32) } as u16 & 0x8000) != 0
}

fn post_ctrl_c() -> bool {
    // Release whatever the user is leaning on, press the chord, then press
    // back only what is still physically down — so the application underneath
    // ends the gesture believing exactly what the keyboard says.
    let interfering: Vec<VIRTUAL_KEY> = INTERFERING.into_iter().filter(|k| held(*k)).collect();
    for key in &interfering {
        if !send_key(*key, true) {
            return false;
        }
    }

    // One keystroke at a time, with a gap, rather than all four in a single
    // batch. A batch arrives with one timestamp, and an application that reads
    // its input asynchronously — which every WinUI and Electron interface does
    // — can see the C before it has processed the Control that qualifies it,
    // and copy nothing. The gap costs forty milliseconds on a path that
    // already allows more than a second for the answer.
    let chord = [
        (VK_CONTROL, false),
        (VK_C, false),
        (VK_C, true),
        (VK_CONTROL, true),
    ];
    for (key, up) in chord {
        if !send_key(key, up) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(12));
    }

    // Afterwards rather than with the chord: a press in the same breath would
    // arrive before the copy is processed and put the modifier back on it.
    for key in interfering {
        if held(key) {
            send_key(key, false);
        }
    }
    true
}

/// Presses or releases one key, as the keyboard would.
///
/// Both the virtual key and its scan code are sent, because the two halves of
/// the world read different ones: a Win32 application reads the virtual key,
/// while anything built on raw input or DirectInput — games, remote desktop
/// clients, a few terminals — reads the scan code and ignores an event that
/// arrives without one.
fn send_key(key: VIRTUAL_KEY, up: bool) -> bool {
    let scan = unsafe { MapVirtualKeyW(key.0 as u32, MAPVK_VK_TO_VSC) } as u16;
    let event = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: scan,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: SYNTHETIC_MARKER,
            },
        },
    };
    let sent = unsafe { SendInput(&[event], std::mem::size_of::<INPUT>() as i32) };
    if sent != 1 {
        crate::trace!("SendInput refused a keystroke (vk={})", key.0);
        return false;
    }
    true
}

/// Polls the sequence number instead of sleeping a fixed interval, so a fast
/// app answers in ~20ms while a slow one still gets the full budget.
fn wait_for_clipboard_change(before: isize, budget: Duration) -> Option<String> {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
        if clipboard_sequence() != before {
            return read_clipboard_string();
        }
    }
    None
}

// -- the clipboard itself ---------------------------------------------------

/// The clipboard, held open.
///
/// Only one process may have it at a time and every one of them takes it for a
/// moment on every copy, so a first refusal usually means "somebody is
/// mid-copy" — which is exactly the situation this is called in. Hence the
/// retry, and hence the guard: a clipboard left open locks it for every other
/// application on the desktop.
struct Clipboard;

impl Clipboard {
    fn open() -> Option<Self> {
        for _ in 0..25 {
            if unsafe { OpenClipboard(Some(HWND::default())) }.is_ok() {
                return Some(Self);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        crate::trace!("clipboard stayed locked by another process");
        None
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        let _ = unsafe { CloseClipboard() };
    }
}

fn read_clipboard_string() -> Option<String> {
    let _clipboard = Clipboard::open()?;
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT).is_err() {
            return None;
        }
        let handle = GetClipboardData(CF_UNICODETEXT).ok()?;
        let global = HGLOBAL(handle.0);
        let ptr = GlobalLock(global) as *const u16;
        if ptr.is_null() {
            return None;
        }
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
        let _ = GlobalUnlock(global);
        Some(text)
    }
}

fn write_clipboard_string(value: &str) {
    let mut utf16: Vec<u16> = value.encode_utf16().collect();
    utf16.push(0);
    let bytes = std::mem::size_of_val(utf16.as_slice());

    let Some(_clipboard) = Clipboard::open() else {
        return;
    };
    unsafe {
        // Moveable, because the clipboard takes ownership of the block and
        // frees it itself; a fixed one is rejected.
        let Ok(global) = GlobalAlloc(GMEM_MOVEABLE, bytes) else {
            return;
        };
        let ptr = GlobalLock(global) as *mut u16;
        if ptr.is_null() {
            return;
        }
        std::ptr::copy_nonoverlapping(utf16.as_ptr(), ptr, utf16.len());
        let _ = GlobalUnlock(global);

        if EmptyClipboard().is_err() {
            return;
        }
        // Ownership of the block passes to the clipboard here, and only if
        // this succeeds. The failing case leaks one string's worth of memory
        // and is not worth unwinding: it means the clipboard was taken from
        // under us between the two calls.
        if SetClipboardData(CF_UNICODETEXT, Some(HANDLE(global.0))).is_err() {
            crate::trace!("could not put text on the clipboard");
        }
    }
}

/// Puts text on the clipboard, for the bubble's copy button.
///
/// The context goes unused here — Win32 owns the clipboard directly — but it
/// is what the Linux side needs on an X11 session, so the signature is shared.
pub fn set_clipboard(_ctx: &eframe::egui::Context, text: &str) {
    write_clipboard_string(text);
}
