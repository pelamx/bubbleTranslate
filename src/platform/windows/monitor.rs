//! Watches for the gestures that finish a text selection.
//!
//! Two low-level hooks — `WH_MOUSE_LL` and `WH_KEYBOARD_LL` — on a thread of
//! their own see mouse and key events system-wide. Rather than polling the
//! focused element (expensive, and it fires mid-drag), the hooks wait for the
//! moment a selection is *completed*: a mouse-up that ended a drag or a
//! multi-click, or a key-up from a shift-navigation.
//!
//! The gesture filter matters more than it looks. Capture can fall back to
//! synthesizing Ctrl+C, so triggering on every mouse-up would fire a copy on
//! every single click anywhere in Windows. Only drags past a few pixels and
//! double or triple clicks get through.
//!
//! Two things here are Windows' own. A low-level hook is not given
//! double-click events — the double-click is synthesized later, by the window
//! that receives the clicks, and never reaches this layer — so the clicks are
//! counted here against the user's own double-click time. And a hook whose
//! callback dwells longer than `LowLevelHooksTimeout` is quietly skipped for
//! that event, which is why nothing below does any work beyond a comparison
//! and a send.

use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetDoubleClickTime, VIRTUAL_KEY, VK_A, VK_C, VK_CONTROL, VK_DOWN, VK_END,
    VK_HOME, VK_LEFT, VK_LWIN, VK_MENU, VK_NEXT, VK_PRIOR, VK_RIGHT, VK_RWIN, VK_SHIFT, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetCursorPos, GetMessageW, HHOOK, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT,
    SetWindowsHookExW, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_SYSKEYUP,
};

use crate::config::TriggerKey;
use crate::platform::Trigger;
use crate::platform::capture;

/// Minimum drag distance, in pixels at 100% scaling, before a mouse-up counts
/// as a selection rather than a click. Scaled to the display below, because a
/// hook reports raw pixels and six of them on a 200% laptop panel is half the
/// gesture it is on a 100% monitor.
const DRAG_THRESHOLD: f64 = 6.0;

/// How far apart two clicks may land and still be one double-click. Windows
/// has a system metric for this, but only for its own hit-testing; a selection
/// gesture is forgiving enough that a fixed few pixels is plenty.
const DOUBLE_CLICK_SLOP: f64 = 6.0;

/// Keys that extend a selection when Shift is held.
const NAVIGATION_KEYS: [VIRTUAL_KEY; 8] = [
    VK_LEFT, VK_RIGHT, VK_UP, VK_DOWN, VK_HOME, VK_END, VK_PRIOR, VK_NEXT,
];

/// Set while the pointer is inside the bubble, so interacting with our own
/// window never kicks off another capture of the app behind it.
static PAUSED: AtomicBool = AtomicBool::new(false);

/// Whether a copy should be treated like a selection.
static WATCH_CLIPBOARD: AtomicBool = AtomicBool::new(false);

/// Which key has to be held for a selection to be worth a bubble.
///
/// An atomic because the hook callbacks read it, and a hook callback must not
/// touch the config mutex — the UI holds that while it draws, and a hook that
/// waits on anything is a hook Windows stops calling.
static TRIGGER_KEY: AtomicU8 = AtomicU8::new(0);

/// What the clipboard looked like when the user's own Ctrl+C went down,
/// sampled before the application had a chance to answer it.
static CLIPBOARD_BEFORE_COPY: AtomicIsize = AtomicIsize::new(0);

/// Where the callbacks send what they see.
///
/// A static because a hook procedure is a bare function pointer with nowhere
/// to put a closure, and there is exactly one monitor per process. The mutex
/// is what makes a `Send` closure — the sending half of a channel, as it
/// happens — legal to keep in a static at all; nothing else ever locks it, so
/// the hook never waits here.
static ON_TRIGGER: OnceLock<Mutex<TriggerSink>> = OnceLock::new();

type TriggerSink = Box<dyn Fn(Trigger) + Send>;

/// The press that a drag is being measured from, and what the clipboard looked
/// like when it began. `None` between gestures.
///
/// The modifiers are part of it because the trigger key is often released in
/// the same moment the button comes up, and the press is when the user's
/// intent was unambiguous.
static PRESS: Mutex<Option<Press>> = Mutex::new(None);

#[derive(Clone, Copy)]
struct Press {
    at: (f64, f64),
    clipboard: isize,
    keyed: bool,
}

/// The run of clicks in progress, for spotting a double or triple click that
/// Windows does not report at this level.
static CLICKS: Mutex<Option<Clicks>> = Mutex::new(None);

#[derive(Clone, Copy)]
struct Clicks {
    at: (f64, f64),
    last: Instant,
    count: u32,
}

pub fn set_paused(paused: bool) {
    PAUSED.store(paused, Ordering::Relaxed);
}

pub fn set_watch_clipboard(watch: bool) {
    WATCH_CLIPBOARD.store(watch, Ordering::Relaxed);
}

fn encode(key: TriggerKey) -> u8 {
    match key {
        TriggerKey::Always => 0,
        TriggerKey::Shift => 1,
        TriggerKey::Ctrl => 2,
        TriggerKey::Alt => 3,
        TriggerKey::Super => 4,
    }
}

pub fn set_trigger_key(key: TriggerKey) {
    TRIGGER_KEY.store(encode(key), Ordering::Relaxed);
}

fn trigger_key() -> TriggerKey {
    match TRIGGER_KEY.load(Ordering::Relaxed) {
        1 => TriggerKey::Shift,
        2 => TriggerKey::Ctrl,
        3 => TriggerKey::Alt,
        4 => TriggerKey::Super,
        _ => TriggerKey::Always,
    }
}

/// Never blocked here, so there is nothing to explain: the hooks see the whole
/// keyboard, so the gate is exact — there is nothing to poll and nothing to
/// miss, unlike a Wayland session where no protocol reports the keyboard to an
/// unfocused client.
pub fn trigger_key_blocked() -> Option<String> {
    None
}

/// Whether a key is physically down at this instant.
fn held(key: VIRTUAL_KEY) -> bool {
    // The high bit is the current state; the low bit is a "pressed since last
    // asked" latch that must not be read as held.
    (unsafe { GetAsyncKeyState(key.0 as i32) } as u16 & 0x8000) != 0
}

/// Whether the key the user chose is held right now. Always true when they
/// chose none.
fn satisfied() -> bool {
    match trigger_key() {
        TriggerKey::Always => true,
        TriggerKey::Shift => held(VK_SHIFT),
        TriggerKey::Ctrl => held(VK_CONTROL),
        TriggerKey::Alt => held(VK_MENU),
        TriggerKey::Super => held(VK_LWIN) || held(VK_RWIN),
    }
}

/// The drag threshold in the pixels the hooks actually report.
fn drag_threshold() -> f64 {
    static SCALED: OnceLock<f64> = OnceLock::new();
    *SCALED.get_or_init(|| {
        let dpi = unsafe { GetDpiForSystem() } as f64;
        if dpi > 0.0 {
            DRAG_THRESHOLD * dpi / 96.0
        } else {
            DRAG_THRESHOLD
        }
    })
}

fn fire(trigger: Trigger) {
    if let Some(on_trigger) = ON_TRIGGER.get()
        && let Ok(on_trigger) = on_trigger.lock()
    {
        on_trigger(trigger);
    }
}

/// Whether this event is one of ours.
///
/// The marker and the flag, and deliberately not the "injected" bit every
/// synthesized event carries: a great many people type through something that
/// injects — a key remapper, a keyboard's own utility, a remote desktop
/// session, an on-screen keyboard — and ignoring all of it would leave the
/// translator silently dead for them. Only our own copy has to be filtered
/// out, and it says so in two ways.
fn is_ours(extra: usize) -> bool {
    capture::is_synthesizing() || extra == capture::SYNTHETIC_MARKER
}

/// Starts the hooks on a dedicated thread and returns immediately.
///
/// The thread is the important part: a low-level hook is delivered to the
/// thread that installed it, by way of that thread's message queue, so it
/// needs a message loop of its own and must not be the UI's.
pub fn spawn(on_trigger: impl Fn(Trigger) + Send + 'static) -> std::io::Result<()> {
    if ON_TRIGGER.set(Mutex::new(Box::new(on_trigger))).is_err() {
        return Err(std::io::Error::other(
            "the selection monitor is already running",
        ));
    }
    std::thread::Builder::new()
        .name("selection-monitor".into())
        .spawn(run)
        .map(|_| ())
}

fn run() {
    // SAFETY: both hooks are global (thread id 0), which for the low-level
    // hooks needs no module handle of its own — Windows calls back into this
    // process rather than injecting a DLL anywhere.
    let mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), None, 0) };
    let keyboard = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), None, 0) };

    match (&mouse, &keyboard) {
        (Ok(_), Ok(_)) => crate::trace!("input hooks installed"),
        _ => {
            eprintln!(
                "bubbleTranslate: could not watch the keyboard and mouse — selections \
                 will not be noticed. Another program may have taken the hook."
            );
            return;
        }
    }

    // The hooks are only ever called while this thread is pumping messages,
    // and this loop never ends: the process exits and Windows removes them.
    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, Some(HWND::default()), 0, 0) }.as_bool() {}
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // Negative codes are not ours to look at; pass them straight on.
    if code < 0 {
        return unsafe { CallNextHookEx(Some(HHOOK::default()), code, wparam, lparam) };
    }
    let event = unsafe { *(lparam.0 as *const MSLLHOOKSTRUCT) };

    // Never react to the Ctrl+C we post ourselves, or the gesture would feed
    // itself.
    if is_ours(event.dwExtraInfo) || PAUSED.load(Ordering::Relaxed) {
        return unsafe { CallNextHookEx(Some(HHOOK::default()), code, wparam, lparam) };
    }

    let at = (event.pt.x as f64, event.pt.y as f64);
    match wparam.0 as u32 {
        WM_LBUTTONDOWN => {
            crate::trace!("mouse-down at ({:.0}, {:.0})", at.0, at.1);
            // The clipboard is sampled here because a copy-on-select interface
            // writes it at mouse-up; by the time the capture runs there is no
            // longer a "before" to compare against.
            *PRESS.lock().unwrap() = Some(Press {
                at,
                clipboard: capture::clipboard_sequence(),
                keyed: satisfied(),
            });
        }
        WM_LBUTTONUP => {
            let press = PRESS.lock().unwrap().take();
            let dragged = press
                .map(|press| distance(press.at, at) > drag_threshold())
                .unwrap_or(false);
            let multi_click = count_click(at) >= 2;

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
            // Either end of the gesture counts: the key may be taken before the
            // button goes down or let go before it comes up, and both are the
            // same request.
            let keyed = satisfied() || press.is_some_and(|press| press.keyed);
            if !keyed {
                crate::trace!(
                    "mouse-up  {} was not held -> ignored",
                    trigger_key().label()
                );
            } else if dragged || multi_click {
                fire(Trigger {
                    at: Some(at),
                    clipboard_before: press.map(|press| press.clipboard),
                });
            }
        }
        _ => {}
    }

    unsafe { CallNextHookEx(Some(HHOOK::default()), code, wparam, lparam) }
}

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
}

/// Counts this release into the run of clicks at this spot, and says how many
/// it is now up to.
///
/// Windows only hands a double-click to the window being clicked, and only
/// after deciding one happened, so a hook has to keep the tally itself. The
/// user's own double-click time is what it is kept against, because that is
/// the speed they have already told the system they click at.
fn count_click(at: (f64, f64)) -> u32 {
    let window = Duration::from_millis(unsafe { GetDoubleClickTime() } as u64);
    let mut clicks = CLICKS.lock().unwrap();
    let count = match *clicks {
        Some(previous)
            if previous.last.elapsed() <= window
                && distance(previous.at, at) <= DOUBLE_CLICK_SLOP =>
        {
            previous.count + 1
        }
        _ => 1,
    };
    *clicks = Some(Clicks {
        at,
        last: Instant::now(),
        count,
    });
    count
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return unsafe { CallNextHookEx(Some(HHOOK::default()), code, wparam, lparam) };
    }
    let event = unsafe { *(lparam.0 as *const KBDLLHOOKSTRUCT) };

    if is_ours(event.dwExtraInfo) || PAUSED.load(Ordering::Relaxed) {
        return unsafe { CallNextHookEx(Some(HHOOK::default()), code, wparam, lparam) };
    }

    let key = VIRTUAL_KEY(event.vkCode as u16);
    match wparam.0 as u32 {
        WM_KEYDOWN if key == VK_C && held(VK_CONTROL) => {
            // Sampled before the application has seen the copy, so that what
            // lands on the clipboard afterwards is recognisably new.
            CLIPBOARD_BEFORE_COPY.store(capture::clipboard_sequence(), Ordering::Relaxed);
        }
        WM_KEYUP | WM_SYSKEYUP => {
            let shift_select = held(VK_SHIFT) && NAVIGATION_KEYS.contains(&key);
            let select_all = held(VK_CONTROL) && key == VK_A;
            // A copy is the one gesture that gets through from an application
            // drawing its own text, which is why it can stand in for a
            // selection — but only when the user asked for that.
            let copied = key == VK_C && held(VK_CONTROL) && WATCH_CLIPBOARD.load(Ordering::Relaxed);

            if (shift_select || select_all || copied) && satisfied() {
                crate::trace!("key-up    vk={} -> TRIGGER", event.vkCode);
                fire(Trigger {
                    at: cursor(),
                    // Only the copy has a "before" worth comparing against: the
                    // others never touched the clipboard, and offering a stale
                    // number would invite the capture to translate it.
                    clipboard_before: copied.then(|| CLIPBOARD_BEFORE_COPY.load(Ordering::Relaxed)),
                });
            }
        }
        _ => {}
    }

    unsafe { CallNextHookEx(Some(HHOOK::default()), code, wparam, lparam) }
}

/// Where the pointer is, in the pixels the hooks report.
fn cursor() -> Option<(f64, f64)> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.ok()?;
    Some((point.x as f64, point.y as f64))
}
