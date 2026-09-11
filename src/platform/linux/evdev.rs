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

    RUNNING.store(true, Ordering::Relaxed);
    std::thread::Builder::new()
        .name("keyboard-modifiers".into())
        .spawn(move || read_loop(devices))
        .map_err(|err| format!("could not start the keyboard reader: {err}"))?;
    crate::trace!("evdev: watching the keyboard for the trigger key");
    Ok(())
}

/// The event nodes that are both readable by us and actually keyboards.
fn keyboards() -> Vec<std::fs::File> {
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
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        if has_shift(file.as_raw_fd()) {
            crate::trace!("evdev: reading {}", path.display());
            found.push(file);
        }
    }
    found
}

/// Whether this device has a left Shift key, which is how a keyboard is told
/// from the mice, switches and buttons that share the same directory.
fn has_shift(fd: RawFd) -> bool {
    const EV_KEY: u32 = 1;
    // The key bitmap, long enough for the modifier range this asks about.
    const BITS: usize = 256;
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
    let code = keycode::LEFTSHIFT as usize;
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

fn read_loop(devices: Vec<std::fs::File>) {
    const EV_KEY: u16 = 1;
    /// A key that is down, or one being auto-repeated; 0 is a release.
    const PRESSED: i32 = 1;
    const REPEATED: i32 = 2;

    let mut fds: Vec<libc::pollfd> = devices
        .iter()
        .map(|file| libc::pollfd {
            fd: file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();

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

    loop {
        // SAFETY: the slice is live for the call and its length is what is
        // passed; poll writes only into `revents`.
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
        if ready < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            // A poll that fails for any other reason will keep failing; going
            // quiet is better than spinning, and the gate then behaves as it
            // does on a session with no reader at all.
            crate::trace!("evdev: poll failed ({err}); stopping the keyboard reader");
            RUNNING.store(false, Ordering::Relaxed);
            HELD.store(0, Ordering::Relaxed);
            return;
        }

        for pollfd in &fds {
            if pollfd.revents & libc::POLLIN == 0 {
                continue;
            }
            loop {
                // SAFETY: reading `size` bytes into a struct of exactly that
                // size, which is the record length this device writes.
                let got =
                    unsafe { libc::read(pollfd.fd, (&raw mut event).cast::<libc::c_void>(), size) };
                if got != size as isize {
                    break;
                }
                if event.kind != EV_KEY {
                    continue;
                }
                // Everything that is not a modifier leaves no trace: no
                // branch below stores it, and `event` is overwritten by the
                // next record.
                let Some(bit) = bit(event.code) else {
                    continue;
                };
                let down = event.value == PRESSED || event.value == REPEATED;
                if down {
                    HELD.fetch_or(bit, Ordering::Relaxed);
                } else {
                    HELD.fetch_and(!bit, Ordering::Relaxed);
                }
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
