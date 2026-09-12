//! A way for a second copy of the binary to reach the one already running.
//!
//! This exists for the sessions where the trigger key cannot be watched. A
//! compositor will not say what the keyboard is doing, but every compositor
//! will run a command on a key combination — that is what a keybinding *is* —
//! so the key press arrives as a process rather than as an event. The second
//! process says "translate the selection" down a socket and exits; the first
//! one does the work.
//!
//! On Windows it carries a second message for a second reason. Nothing there
//! stops a user launching the app again while it is running, and two copies
//! would mean two tray icons and two translators racing for the same
//! selection, so the new process asks the old one to show its window and then
//! gets out of the way.
//!
//! The transport is whatever the system addresses one instance with: a Unix
//! socket in the runtime directory, or a named pipe. Both are per-user, both
//! disappear with the process, and neither needs polling on either side.
//! Nothing on them is privileged — they carry one of two fixed words and no
//! payload, so the worst a local process can do is ask for a bubble.

/// The messages. Kept to fixed words so the listener never has to parse
/// anything a caller controls.
const TRANSLATE: &str = "translate-selection";
#[cfg(target_os = "windows")]
const OPEN: &str = "open-window";

/// How much of a caller's message is ever read. The sender is not necessarily
/// ours: a process that opens the socket and then writes forever must not be
/// able to grow the listener's memory.
const MAX_MESSAGE: u64 = 64;

#[cfg(unix)]
pub use unix::{listen, request_translate};

#[cfg(target_os = "windows")]
pub use windows::{listen, request_open, request_translate};

#[cfg(unix)]
mod unix {
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;

    use super::{MAX_MESSAGE, TRANSLATE};

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
                    if Read::by_ref(&mut stream)
                        .take(MAX_MESSAGE)
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
}

#[cfg(target_os = "windows")]
mod windows {
    use std::io::Write;
    use std::time::{Duration, Instant};

    use ::windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_BUSY, HANDLE};
    use ::windows::Win32::Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND, ReadFile,
    };
    use ::windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
        PIPE_TYPE_BYTE, PIPE_WAIT, PeekNamedPipe, WaitNamedPipeW,
    };
    use ::windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use ::windows::core::PCWSTR;

    use super::{MAX_MESSAGE, OPEN, TRANSLATE};

    /// Where the running instance listens.
    ///
    /// The user's name and the session are both in it. Pipe names are machine
    /// wide, and two people signed in at once — or one person on the console
    /// and again over remote desktop — are running two desktops that must not
    /// hand each other their selections.
    fn pipe_name() -> String {
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".to_string());
        let user: String = user
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        let mut session = 0u32;
        // SAFETY: both arguments are plain values; the call cannot fail in a
        // way that matters, and a zero session is a usable name either way.
        let _ = unsafe { ProcessIdToSessionId(std::process::id(), &mut session) };
        format!(r"\\.\pipe\bubbleTranslate-{session}-{user}")
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Starts listening, calling `on_translate` for every translate request
    /// that arrives. A request to open the window is answered here, because
    /// the shell already knows how to ask the interface for that.
    ///
    /// There is no stale pipe to clear: a named pipe exists only while the
    /// process holding it does. `FILE_FLAG_FIRST_PIPE_INSTANCE` is what makes
    /// this safe against two copies starting at the same moment — the second
    /// one is refused by the kernel rather than by a check it could lose a race
    /// on.
    pub fn listen(on_translate: impl Fn() + Send + 'static) -> std::io::Result<()> {
        let name = pipe_name();
        let pipe = create(&name)?;

        std::thread::Builder::new()
            .name("hotkey-pipe".into())
            .spawn(move || {
                let pipe = pipe;
                loop {
                    // A client that connected and vanished before this call
                    // reports "already connected", which is a served request,
                    // not an error.
                    let _ = unsafe { ConnectNamedPipe(pipe.0, None) };
                    let message = read_message(pipe.0);
                    // Always, including when nothing was read. The pipe has one
                    // instance, so a connection left open is the whole service
                    // gone: every later caller is told the pipe is busy, and a
                    // second copy of the app, finding nobody to defer to,
                    // starts as a rival with its own tray icon and its own
                    // input hooks.
                    let _ = unsafe { DisconnectNamedPipe(pipe.0) };

                    match message.trim() {
                        TRANSLATE => {
                            crate::trace!("ipc: asked to translate the selection");
                            on_translate();
                        }
                        OPEN => {
                            crate::trace!("ipc: asked to show the window");
                            crate::shell::request_open();
                        }
                        "" => {}
                        other => crate::trace!("ipc: ignoring {other:?}"),
                    }
                }
            })?;
        crate::trace!("ipc: listening on {name}");
        Ok(())
    }

    fn create(name: &str) -> std::io::Result<Pipe> {
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(wide(name).as_ptr()),
                PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                // One instance, because there is one running copy to talk to,
                // and requests are a keystroke apart at worst.
                1,
                0,
                MAX_MESSAGE as u32,
                0,
                None,
            )
        };
        if handle.is_invalid() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                "another bubbleTranslate is already listening",
            ));
        }
        Ok(Pipe(handle))
    }

    /// How long a connected caller has to say what it wants.
    ///
    /// Generous for a caller that writes nineteen bytes the instant it
    /// connects, which is all any of ours do, and short enough that one that
    /// never writes at all costs a pause rather than the service.
    const SPEAK_UP: Duration = Duration::from_secs(2);

    /// Reads one request, or gives up.
    ///
    /// The wait is a poll rather than a plain `ReadFile` because a read on a
    /// pipe that is not overlapped cannot be given a deadline, and without one
    /// a caller that connects and then says nothing blocks this thread for as
    /// long as it lives. That is not hypothetical politeness: it happened, and
    /// what it looked like from outside was the app quietly losing its ability
    /// to notice a second launch.
    fn read_message(pipe: HANDLE) -> String {
        let deadline = Instant::now() + SPEAK_UP;
        loop {
            let mut available = 0u32;
            // A caller that has gone away fails this outright, which is the
            // other way out of the loop and the common one: it connected,
            // wrote, and closed before we looked.
            if unsafe { PeekNamedPipe(pipe, None, 0, None, Some(&mut available), None) }.is_err() {
                break;
            }
            if available > 0 {
                let mut buffer = [0u8; MAX_MESSAGE as usize];
                let mut read = 0u32;
                if unsafe { ReadFile(pipe, Some(&mut buffer), Some(&mut read), None) }.is_err() {
                    break;
                }
                return String::from_utf8_lossy(&buffer[..read as usize]).into_owned();
            }
            if Instant::now() >= deadline {
                crate::trace!("ipc: a caller connected and said nothing");
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        String::new()
    }

    /// Owns the pipe handle for as long as the listener thread runs, which is
    /// for as long as the process does.
    struct Pipe(HANDLE);

    // SAFETY: a pipe handle is just a kernel handle; nothing about it is bound
    // to the thread that created it, and only the listener thread uses it.
    unsafe impl Send for Pipe {}

    impl Drop for Pipe {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }

    /// Asks the running instance to translate what is selected right now.
    ///
    /// Returns whether anyone was there to ask. This is the whole of the
    /// `--translate-selection` command, which a keybinding runs.
    pub fn request_translate() -> bool {
        send(TRANSLATE)
    }

    /// Asks the running instance to show its window, and says whether there
    /// was one to ask. This is how launching the app a second time brings the
    /// first copy forward instead of starting a rival translator.
    pub fn request_open() -> bool {
        send(OPEN)
    }

    fn send(message: &str) -> bool {
        let name = pipe_name();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            // The ordinary file API speaks to a pipe perfectly well, and
            // opening one that nobody is serving fails immediately — which is
            // exactly the question being asked.
            match std::fs::OpenOptions::new().write(true).open(&name) {
                Ok(mut pipe) => return pipe.write_all(message.as_bytes()).is_ok(),
                // "Busy" is not "absent". The listener serves one caller at a
                // time, so a pipe that exists and is mid-conversation answers
                // this way, and reading it as "nothing is running" is how a
                // second copy of the app talks itself into starting up
                // alongside the first. Wait for the instance to come free.
                Err(err) if err.raw_os_error() == Some(ERROR_PIPE_BUSY.0 as i32) => {
                    if Instant::now() >= deadline {
                        crate::trace!("ipc: the listener stayed busy");
                        return false;
                    }
                    let wide = wide(&name);
                    // The documented way to queue for a busy pipe; it returns
                    // as soon as an instance frees up.
                    let _ = unsafe { WaitNamedPipeW(PCWSTR(wide.as_ptr()), 500) };
                }
                Err(_) => return false,
            }
        }
    }
}
