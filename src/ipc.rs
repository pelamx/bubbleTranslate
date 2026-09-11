//! A way for a second copy of the binary to reach the one already running.
//!
//! This exists for the sessions where the trigger key cannot be watched. A
//! compositor will not say what the keyboard is doing, but every compositor
//! will run a command on a key combination — that is what a keybinding *is* —
//! so the key press arrives as a process rather than as an event. The second
//! process says "translate the selection" down a socket and exits; the first
//! one does the work.
//!
//! A Unix socket in the runtime directory rather than a signal or a file: it
//! is addressed to one instance, it disappears with the session, and it needs
//! no polling on either side. Nothing on it is privileged — it carries one
//! fixed word and no payload, so the worst a local process can do with it is
//! ask for a bubble.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

/// The only message. Kept a single fixed word so the listener never has to
/// parse anything a caller controls.
const TRANSLATE: &str = "translate-selection";

/// Where the running instance listens.
///
/// `XDG_RUNTIME_DIR` first: it is per-user, per-session, and cleaned up on
/// logout, which is exactly the lifetime this wants. The fallback is the
/// temporary directory with the user id in the name, so two users on one
/// machine cannot collide.
pub fn socket_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("bubbleTranslate.sock");
    }
    // SAFETY: getuid takes no arguments, touches nothing, and cannot fail.
    let uid = unsafe { libc::getuid() };
    std::env::temp_dir().join(format!("bubbleTranslate-{uid}.sock"))
}

/// Starts listening, calling `on_translate` for every request that arrives.
///
/// A stale socket from an instance that was killed rather than closed would
/// make binding fail forever, so one is cleared out of the way first — but
/// only after checking that nobody is answering on it, which is what keeps
/// this from stealing the socket of a copy that is running perfectly well.
pub fn listen(on_translate: impl Fn() + Send + 'static) -> std::io::Result<()> {
    let path = socket_path();
    if path.exists() {
        if UnixStream::connect(&path).is_ok() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                "another bubbleTranslate is already listening",
            ));
        }
        let _ = std::fs::remove_file(&path);
    }

    let listener = UnixListener::bind(&path)?;
    std::thread::Builder::new()
        .name("hotkey-socket".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut message = String::new();
                // Bounded, because the sender is not necessarily ours: a
                // process that opens the socket and then writes forever must
                // not be able to grow this thread's memory.
                if Read::by_ref(&mut stream)
                    .take(64)
                    .read_to_string(&mut message)
                    .is_err()
                {
                    continue;
                }
                if message.trim() == TRANSLATE {
                    crate::trace!("ipc: asked to translate the selection");
                    on_translate();
                }
            }
        })?;
    crate::trace!("ipc: listening on {}", path.display());
    Ok(())
}

/// Asks the running instance to translate what is selected right now.
///
/// Returns whether anyone was there to ask. This is the whole of the
/// `--translate-selection` command, which a keybinding runs.
pub fn request_translate() -> bool {
    let Ok(mut stream) = UnixStream::connect(socket_path()) else {
        return false;
    };
    stream.write_all(TRANSLATE.as_bytes()).is_ok()
}
