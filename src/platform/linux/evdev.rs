//! Reading the keyboard directly, because Wayland will not describe it.
//!
//! The trigger key is a question about a keyboard that is not typing into us:
//! the user is selecting text in someone else's window. X11 answers that
//! question — the modifier mask rides along with every pointer query — and
//! Wayland refuses to, deliberately and in every compositor, since an
//! interface for "what is the keyboard doing right now" is an interface for
//! writing a keylogger.
//!
//! What is left is the layer underneath both of them. `/dev/input/event*` is
//! the kernel's own view of the hardware, and it says which keys are down
//! without any compositor's cooperation. The same property that makes Wayland
//! refuse applies here, so this reads as little as it possibly can:
//!
//!   * only devices that carry a Shift key, which rules out mice, lid
//!     switches, power buttons and the rest of the event nodes;
//!   * only the eight modifier keycodes, and only as a bitmask of what is
//!     currently held. Every other keycode is dropped inside the read loop —
//!     nothing else is stored, counted or forwarded anywhere.
//!
//! The one exception is the left mouse button, read from devices that have no
//! keyboard, so the clipboard route in [`super::borrow`] can tell when a drag
//! ended. It is the button's state and nothing else — not the pointer's path.
//!
//! It needs permission the desktop does not hand out by default: the device
//! nodes belong to the `input` group. Without membership this module reports
//! that it is unavailable and the gate falls back to translating every
//! selection, which is the same place a session without it was already in.

use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::config::TriggerKey;

/// Which modifiers are held, as one bit per [`Mod`].
static HELD: AtomicU32 = AtomicU32::new(0);

/// Whether the reader got far enough to be believed. Until a device is open,
/// an empty [`HELD`] means "we do not know", not "nothing is pressed".
static RUNNING: AtomicBool = AtomicBool::new(false);

/// The shared left-button reader's state, kept apart from the keyboard's so a
/// session that can read the mouse but not the keyboard (or the reverse) is
/// believed about the one it can. `POINTER_RUNNING` is the same "we do not
/// know yet" guard [`RUNNING`] is for the keyboard.
static POINTER_RUNNING: AtomicBool = AtomicBool::new(false);
static BUTTON_DOWN: AtomicBool = AtomicBool::new(false);

/// The kernel keycodes for the keys this cares about. Left and right halves
/// are the same modifier as far as a gate is concerned.
mod keycode {
    pub const LEFTCTRL: u16 = 29;
    pub const LEFTSHIFT: u16 = 42;
    pub const RIGHTSHIFT: u16 = 54;
    pub const LEFTALT: u16 = 56;
    pub const RIGHTCTRL: u16 = 97;
    pub const RIGHTALT: u16 = 100;
    pub const LEFTMETA: u16 = 125;
    pub const RIGHTMETA: u16 = 126;
    /// Not a modifier: read only by [`super::start_pointer`], from devices
    /// that have no keyboard at all.
    pub const BTN_LEFT: u16 = 272;
}

/// The bit a keycode sets, or `None` for every key that is not a modifier —
/// which is where the rest of the keyboard is discarded.
fn bit(code: u16) -> Option<u32> {
    Some(match code {
        keycode::LEFTSHIFT | keycode::RIGHTSHIFT => 1 << 0,
        keycode::LEFTCTRL | keycode::RIGHTCTRL => 1 << 1,
        keycode::LEFTALT | keycode::RIGHTALT => 1 << 2,
        keycode::LEFTMETA | keycode::RIGHTMETA => 1 << 3,
        _ => return None,
    })
}

fn wanted_bit(key: TriggerKey) -> Option<u32> {
    Some(match key {
        TriggerKey::Always => return None,
        TriggerKey::Shift => 1 << 0,
        TriggerKey::Ctrl => 1 << 1,
        TriggerKey::Alt => 1 << 2,
        TriggerKey::Super => 1 << 3,
    })
}

/// Whether the trigger key is held down right now.
///
/// `None` when there is no reader — no permission, no keyboard, or it was
/// never started — so the caller can tell "not held" apart from "no idea".
pub fn held(key: TriggerKey) -> Option<bool> {
    if !RUNNING.load(Ordering::Relaxed) {
        return None;
    }
    let wanted = wanted_bit(key)?;
    Some(HELD.load(Ordering::Relaxed) & wanted != 0)
}

/// Whether the keyboard can be read at all on this machine.
pub fn available() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

/// Starts a shared left-button reader once, so [`button_down`] can answer.
///
/// Separate from [`ensure_started`]: the button is wanted on the selection
/// path even where the keyboard is not (a compositor that reports modifiers
/// but not the mouse), and it is not wanted where nothing waits on a drag.
/// Idempotent, and silent when the mouse cannot be read — the caller falls
/// back to the settle window, exactly as it does without this reader at all.
pub fn ensure_pointer_started() {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        match start_pointer(|down| BUTTON_DOWN.store(down, Ordering::Relaxed)) {
            Ok(()) => POINTER_RUNNING.store(true, Ordering::Relaxed),
            Err(reason) => crate::trace!("evdev: not reading the mouse button — {reason}"),
        }
    });
}

/// Whether the left mouse button is being read, so its state can be trusted.
pub fn pointer_available() -> bool {
    POINTER_RUNNING.load(Ordering::Relaxed)
}

/// Whether the left button is down right now, or `None` when it is not read.
pub fn button_down() -> Option<bool> {
    POINTER_RUNNING
        .load(Ordering::Relaxed)
        .then(|| BUTTON_DOWN.load(Ordering::Relaxed))
}

/// Why [`start`] did not get a reader, once it has been tried.
///
/// `None` both before the attempt and after a successful one — the interface
/// only asks this once it already knows the gate is not in force.
static START_ERROR: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

/// Starts the reader once, whatever happens, and remembers why if it could
/// not. Safe to call from anywhere; only the first call does anything.
pub fn ensure_started() {
    START_ERROR.get_or_init(|| match start() {
        Ok(()) => None,
        Err(reason) => {
            crate::trace!("evdev: not reading the keyboard — {reason}");
            Some(reason)
        }
    });
}

pub fn start_error() -> Option<String> {
    START_ERROR.get().cloned().flatten()
}

/// Opens every keyboard we are allowed to and watches it on a thread of its
/// own.
///
/// Returns the reason it could not, which the interface shows: this is the one
/// failure here a user can actually fix, and it is fixed by joining a group
/// rather than by anything inside the app.
pub fn start() -> Result<(), String> {
    let devices = keyboards();
    if devices.is_empty() {
        return Err(match std::fs::read_dir("/dev/input") {
            // The nodes are there and unreadable, which is the ordinary case
            // and the one worth explaining.
            Ok(_) => "no readable keyboard in /dev/input — add your user to the \
                      'input' group and log in again"
                .to_string(),
            Err(err) => format!("/dev/input cannot be listed: {err}"),
        });
    }

    std::thread::Builder::new()
        .name("keyboard-modifiers".into())
        .spawn(move || {
            // Set inside the thread rather than before the spawn: a failed
            // spawn must leave RUNNING false, so that `held()` answers
            // "no idea" (fail-open) instead of "not held" (every gated
            // selection silently dropped) for the life of the process.
            RUNNING.store(true, Ordering::Relaxed);
            read_loop(devices, "keyboard", keyboards, |present| {
                // With no keyboard attached the answer is "no idea", not "not
                // held" -- the gate fails open rather than dropping every
                // selection until one is plugged back in.
                RUNNING.store(present, Ordering::Relaxed);
                if !present {
                    HELD.store(0, Ordering::Relaxed);
                }
            }, |code, down| {
                // Everything that is not a modifier leaves no trace: no
                // branch below stores it.
                let Some(bit) = bit(code) else {
                    return;
                };
                if down {
                    HELD.fetch_or(bit, Ordering::Relaxed);
                } else {
                    HELD.fetch_and(!bit, Ordering::Relaxed);
                }
            });
            // The reader itself stopped; "no idea" again, not "not held".
            RUNNING.store(false, Ordering::Relaxed);
            HELD.store(0, Ordering::Relaxed);
        })
        .map_err(|err| format!("could not start the keyboard reader: {err}"))?;
    crate::trace!("evdev: watching the keyboard for the trigger key");
    Ok(())
}

/// The event nodes that are both readable by us and pass `wanted`.
///
/// Opened non-blocking: the read loop drains each device until it would
/// block, so no keyboard's events wait on another keyboard's silence.
fn devices(wanted: impl Fn(RawFd) -> bool) -> Vec<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    let Ok(entries) = std::fs::read_dir("/dev/input") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("event"))
        {
            continue;
        }
        let Ok(file) = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
        else {
            continue;
        };
        if wanted(file.as_raw_fd()) {
            found.push(file);
        }
    }
    found
}

/// The event nodes that are keyboards.
fn keyboards() -> Vec<std::fs::File> {
    devices(|fd| has_key(fd, keycode::LEFTSHIFT))
}

/// The event nodes that are pointers: a left button and no Shift key, so a
/// keyboard with a built-in trackpoint is read once, as a keyboard, and not
/// again here.
fn pointers() -> Vec<std::fs::File> {
    devices(|fd| has_key(fd, keycode::BTN_LEFT) && !has_key(fd, keycode::LEFTSHIFT))
}

/// Whether this device can send `code`, which is how a keyboard is told from
/// the mice, switches and buttons that share the same directory.
fn has_key(fd: RawFd, code: u16) -> bool {
    const EV_KEY: u32 = 1;
    // The key bitmap, long enough for the mouse buttons at 0x110.
    const BITS: usize = 768;
    let mut map = [0u8; BITS / 8];

    // EVIOCGBIT(EV_KEY, len): _IOC(_IOC_READ, 'E', 0x20 + EV_KEY, len).
    let request: libc::c_ulong = (2 << 30)
        | ((map.len() as libc::c_ulong) << 16)
        | ((b'E' as libc::c_ulong) << 8)
        | (0x20 + EV_KEY as libc::c_ulong);

    // SAFETY: the buffer outlives the call and is exactly the length encoded
    // in the request, which is what the driver writes into.
    if unsafe { libc::ioctl(fd, request, map.as_mut_ptr()) } < 0 {
        return false;
    }
    let code = code as usize;
    map[code / 8] & (1 << (code % 8)) != 0
}

/// One `input_event` as the kernel writes it. Declared here rather than pulled
/// from a crate: it is four fields, and its layout is part of the kernel ABI.
#[repr(C)]
#[derive(Clone, Copy)]
struct InputEvent {
    _time: libc::timeval,
    kind: u16,
    code: u16,
    value: i32,
}

/// Watches the mice for the left button, calling `on_left` with `true` when it
/// goes down and `false` when it comes up.
///
/// The same restraint as the keyboard: one button, and nothing about where
/// the pointer went. Tap-to-click on a touchpad is synthesized above the
/// kernel and never appears here; a physical click does.
pub fn start_pointer(on_left: impl Fn(bool) + Send + 'static) -> Result<(), String> {
    let devices = pointers();
    if devices.is_empty() {
        return Err("no readable mouse in /dev/input".to_string());
    }
    std::thread::Builder::new()
        .name("pointer-button".into())
        .spawn(move || {
            read_loop(devices, "pointer", pointers, |_| {}, |code, down| {
                if code == keycode::BTN_LEFT {
                    on_left(down);
                }
            })
        })
        .map_err(|err| format!("could not start the pointer reader: {err}"))?;
    crate::trace!("evdev: watching the left mouse button");
    Ok(())
}

/// How often the readers look for devices that were not there before.
///
/// A Bluetooth mouse that reconnects after the laptop sleeps comes back as a
/// new event node, and the old one is gone for good; without a rescan the
/// reader goes deaf to that mouse until the app is restarted. Opening a
/// couple of dozen nodes every few seconds costs nothing measurable.
const RESCAN: std::time::Duration = std::time::Duration::from_secs(2);

/// Which device an open file is, so a rescan can tell a node already being
/// read from a new one that happens to reuse its name.
fn identity(file: &std::fs::File) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    file.metadata().ok().map(|m| (m.dev(), m.ino()))
}

/// Reads key events from `devices`, and from any that `rescan` finds later,
/// handing each one to `on_key` as a code and whether it is down. Autorepeat
/// counts as down. `on_present` is told whenever the set goes from empty to
/// not, or back.
fn read_loop(
    devices: Vec<std::fs::File>,
    what: &str,
    rescan: fn() -> Vec<std::fs::File>,
    mut on_present: impl FnMut(bool),
    mut on_key: impl FnMut(u16, bool),
) {
    const EV_KEY: u16 = 1;
    /// A key that is down, or one being auto-repeated; 0 is a release.
    const PRESSED: i32 = 1;
    const REPEATED: i32 = 2;

    let mut files: Vec<std::fs::File> = Vec::new();
    let mut fds: Vec<libc::pollfd> = Vec::new();
    let mut ids: Vec<Option<(u64, u64)>> = Vec::new();

    let adopt = |found: Vec<std::fs::File>,
                     files: &mut Vec<std::fs::File>,
                     fds: &mut Vec<libc::pollfd>,
                     ids: &mut Vec<Option<(u64, u64)>>| {
        for file in found {
            let id = identity(&file);
            if id.is_some() && ids.contains(&id) {
                continue; // already reading it; this copy is simply closed
            }
            crate::trace!("evdev: reading {what} device {}", file.as_raw_fd());
            fds.push(libc::pollfd {
                fd: file.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            });
            ids.push(id);
            files.push(file);
        }
    };
    adopt(devices, &mut files, &mut fds, &mut ids);

    let mut event = InputEvent {
        _time: libc::timeval {
            tv_sec: 0,
            tv_usec: 0,
        },
        kind: 0,
        code: 0,
        value: 0,
    };
    let size = std::mem::size_of::<InputEvent>();
    let mut last_scan = std::time::Instant::now();

    loop {
        if last_scan.elapsed() >= RESCAN {
            last_scan = std::time::Instant::now();
            let was_empty = files.is_empty();
            adopt(rescan(), &mut files, &mut fds, &mut ids);
            if was_empty && !files.is_empty() {
                on_present(true);
            }
        }

        // SAFETY: the slice is live for the call and its length is what is
        // passed; poll writes only into `revents`. With no devices at all it
        // is simply a sleep until the next rescan.
        let ready = unsafe {
            libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, RESCAN.as_millis() as i32)
        };
        if ready < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            // A poll that fails for any other reason will keep failing; going
            // quiet is better than spinning, and the gate then behaves as it
            // does on a session with no reader at all.
            crate::trace!("evdev: poll failed ({err}); stopping the {what} reader");
            return;
        }
        if ready == 0 {
            continue;
        }

        // Devices that reported an error, hangup or invalid fd — an unplug
        // reports POLLHUP without POLLIN, so skipping them and re-polling
        // would spin hot forever. They are collected and closed after the
        // pass, which is also what keeps poll from returning immediately
        // ever after.
        let mut gone: Vec<usize> = Vec::new();

        for (index, pollfd) in fds.iter().enumerate() {
            if pollfd.revents & libc::POLLIN == 0 {
                if pollfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                    crate::trace!("evdev: {what} device {} went away", pollfd.fd);
                    gone.push(index);
                }
                continue;
            }
            loop {
                // SAFETY: reading `size` bytes into a struct of exactly that
                // size, which is the record length this device writes.
                let got =
                    unsafe { libc::read(pollfd.fd, (&raw mut event).cast::<libc::c_void>(), size) };
                if got < 0 {
                    let err = std::io::Error::last_os_error();
                    match err.kind() {
                        // Drained: this device has nothing more right now.
                        // Back to poll, so the other keyboards get their
                        // turn — these fds are non-blocking precisely so
                        // this returns instead of parking on whichever
                        // device spoke last.
                        std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::Interrupted => break,
                        _ => {
                            crate::trace!("evdev: read failed ({err}); dropping the device");
                            gone.push(index);
                            break;
                        }
                    }
                }
                if got != size as isize {
                    break;
                }
                if event.kind != EV_KEY {
                    continue;
                }
                on_key(event.code, event.value == PRESSED || event.value == REPEATED);
            }
        }

        if !gone.is_empty() {
            gone.dedup();
            for index in gone.into_iter().rev() {
                fds.remove(index);
                ids.remove(index);
                // Dropping the file closes the node, so a device that comes
                // back is opened fresh rather than leaking the old handle.
                files.remove(index);
            }
            if files.is_empty() {
                crate::trace!("evdev: no {what} devices left; waiting for one to appear");
                on_present(false);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_modifiers_are_recognised() {
        // A, Space, Escape, F1 — the keys a translator has no business
        // knowing about.
        for code in [30u16, 57, 1, 59] {
            assert_eq!(bit(code), None, "keycode {code} must be dropped");
        }
        assert_eq!(bit(keycode::LEFTSHIFT), bit(keycode::RIGHTSHIFT));
        assert_eq!(bit(keycode::LEFTMETA), wanted_bit(TriggerKey::Super));
    }

    #[test]
    fn an_unstarted_reader_says_it_does_not_know() {
        // Nothing has been started in a unit test, and "no idea" has to be
        // distinguishable from "not held" — the gate treats them differently.
        assert_eq!(held(TriggerKey::Shift), None);
        assert!(!available());
    }
}
