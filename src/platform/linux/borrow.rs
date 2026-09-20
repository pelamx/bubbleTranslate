//! Borrowing the clipboard, for pages that publish no selection.
//!
//! Some web pages draw their own text and their own highlight: Google Drive's
//! PDF preview is the one that prompted this. Selecting text there changes
//! nothing the desktop can see — the primary selection stays exactly as it
//! was — and the page only hands the text over when it is copied.
//!
//! So when a drag made with the trigger key held ends in a browser and no
//! selection appears, Ctrl+C is sent on the user's behalf, the text is read
//! off the clipboard as it arrives, and the clipboard the user had is put
//! back. Every condition is there to keep that from happening anywhere a
//! synthesized copy would do harm:
//!
//!   * only for the trigger key, and only when something positively says it
//!     was held — "no idea" is not enough to send a keystroke;
//!   * only after a drag, or a double click — the two ways a word gets
//!     selected, and both of them deliberate. A single click is neither;
//!   * only in a browser: Ctrl+C in a terminal interrupts what is running;
//!   * only when the page published nothing, so every ordinary selection
//!     still goes the ordinary way and nothing is copied at all;
//!   * never over a clipboard holding something that is not text, which could
//!     not be restored.
//!
//! Hyprland only, because it is the compositor that will both say which
//! window is focused and send a chord with an exact modifier mask.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::config::TriggerKey;

use super::{compositor, cursor, evdev, monitor, wayland};

/// How long after the button comes up a page has to publish a selection of
/// its own before it is taken not to. Chromium publishes on mouse-up, well
/// inside this.
const PUBLISH_GRACE: Duration = Duration::from_millis(250);

/// How long the copy has to arrive. A page that copies nothing never answers,
/// and the next ordinary copy must not be mistaken for this one.
const COPY_BUDGET: Duration = Duration::from_millis(800);

/// How long after the copy arrives the echoes of it — and of the restore —
/// are ignored.
const ECHO_WINDOW: Duration = Duration::from_millis(700);

/// Pointer movement below this, in logical pixels, is a click.
const MIN_DRAG: f64 = 6.0;

/// How soon after a release the next press is the same gesture continuing —
/// a double click selecting a word, or a triple click selecting a line. The
/// pointer does not move at all for those, so the drag test below can never
/// see them, and a word is exactly what someone reaching for a translator
/// selects most often.
const MULTI_CLICK: Duration = Duration::from_millis(400);

/// The window classes a copy may be sent to, as lowercase substrings.
const BROWSERS: [&str; 9] = [
    "chrome", "chromium", "brave", "microsoft-edge", "vivaldi", "helium", "firefox", "zen",
    "librewolf",
];

struct Pending {
    until: Instant,
    /// The clipboard to put back; `None` when it was empty.
    saved: Option<String>,
}

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);
static QUIET_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);

/// Starts watching the mouse, where this session can do the rest.
pub fn start() {
    if !compositor::can_answer() {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel::<bool>();
    if let Err(reason) = evdev::start_pointer(move |down| {
        let _ = tx.send(down);
    }) {
        crate::trace!("borrow: off — {reason}");
        return;
    }
    // Its own thread, so the compositor round trips below never stall the
    // device reader.
    let _ = std::thread::Builder::new()
        .name("clipboard-borrow".into())
        .spawn(move || {
            let mut press: Option<(u64, Option<(f64, f64)>, bool)> = None;
            let mut released: Option<Instant> = None;
            for down in rx {
                if down {
                    let multi = continues_a_click(released);
                    press = Some((wayland::primary_generation(), cursor::position(), multi));
                } else if let Some((generation, from, multi)) = press.take() {
                    released = Some(Instant::now());
                    consider(generation, from, multi);
                }
            }
        });
}

/// Whether a press this soon after the last release is the same gesture
/// continuing rather than a new one starting.
fn continues_a_click(released: Option<Instant>) -> bool {
    released.is_some_and(|at| at.elapsed() < MULTI_CLICK)
}

fn consider(generation: u64, from: Option<(f64, f64)>, multi_click: bool) {
    let key = monitor::trigger_key();
    if key == TriggerKey::Always || monitor::key_held(key) != Some(true) {
        return;
    }
    // A double click has no distance to measure; it is the gesture itself
    // that says the user picked a word out.
    if !multi_click {
        let (Some(from), Some(to)) = (from, cursor::position()) else {
            return;
        };
        if (to.0 - from.0).hypot(to.1 - from.1) < MIN_DRAG {
            return;
        }
    }
    let Some(class) = compositor::focused_class() else {
        return;
    };
    if !is_browser(&class) {
        return;
    }
    std::thread::sleep(PUBLISH_GRACE);
    if wayland::primary_generation() != generation {
        return;
    }
    let Ok(saved) = wayland::read_clipboard() else {
        crate::trace!("borrow    clipboard holds something that is not text; leaving it");
        return;
    };
    *PENDING.lock().unwrap() = Some(Pending {
        until: Instant::now() + COPY_BUDGET,
        saved,
    });
    if compositor::send_copy() {
        crate::trace!("borrow    no selection published in {class}; copying");
    } else {
        PENDING.lock().unwrap().take();
    }
}

fn is_browser(class: &str) -> bool {
    BROWSERS.iter().any(|b| class.contains(b))
}

/// Claims the copy in flight, if there is one, returning the clipboard to
/// restore once its text has been read.
pub(super) fn take_pending() -> Option<Option<String>> {
    let pending = PENDING.lock().unwrap().take()?;
    if Instant::now() > pending.until {
        return None;
    }
    *QUIET_UNTIL.lock().unwrap() = Some(Instant::now() + ECHO_WINDOW);
    Some(pending.saved)
}

/// Whether selection events are ours rather than the user's right now: a
/// copy is in flight, or has just landed and is being restored.
pub(super) fn suppressing() -> bool {
    let now = Instant::now();
    if PENDING
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|p| now <= p.until)
    {
        return true;
    }
    QUIET_UNTIL.lock().unwrap().is_some_and(|t| now <= t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_browsers_are_sent_a_copy() {
        for class in ["google-chrome", "chromium", "brave-browser", "firefox", "zen"] {
            assert!(is_browser(class), "{class}");
        }
        for class in ["alacritty", "com.mitchellh.ghostty", "code", "kitty", "foot"] {
            assert!(!is_browser(class), "{class}");
        }
    }

    #[test]
    fn a_quick_second_press_is_the_same_gesture() {
        assert!(continues_a_click(Some(Instant::now())));
        assert!(!continues_a_click(Some(Instant::now() - MULTI_CLICK)));
        // The very first click of a session has nothing to continue.
        assert!(!continues_a_click(None));
    }

    #[test]
    fn an_expired_copy_is_not_claimed() {
        *PENDING.lock().unwrap() = Some(Pending {
            until: Instant::now() - Duration::from_millis(1),
            saved: None,
        });
        assert!(take_pending().is_none());
    }
}
