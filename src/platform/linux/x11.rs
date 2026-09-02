//! Reading selections on an X11 session, and asking X where the pointer is.
//!
//! X11 has had a primary selection since long before clipboards were a
//! convention: selecting text *is* putting it there, and any client may read
//! it. That makes this the easy case — no permission, no synthesized copy, no
//! focus requirement. The XFixes extension supplies the other half by
//! reporting when the selection changes hands, so nothing has to be polled.
//!
//! This path is only taken on a real X11 session. Under XWayland it would
//! compile and connect and then never work: the compositor guards the bridged
//! selection behind keyboard focus, so `convert_selection` comes back refused
//! for a background app. [`super::Backend`] makes that choice, not this file.

use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xfixes;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, KeyButMask, WindowClass,
};
use x11rb::rust_connection::RustConnection;

/// How long to wait for the selection's owner to answer a conversion request.
///
/// Generous because the cost of waiting is paid on the watcher thread while
/// the cost of giving up early is a selection that silently vanishes — the
/// same trade the macOS clipboard path makes.
const CONVERT_TIMEOUT: Duration = Duration::from_millis(1000);

struct Atoms {
    primary: u32,
    clipboard: u32,
    utf8: u32,
    incr: u32,
    /// The property the selection's owner writes its answer into. Private to
    /// our own window, so no other client can collide with it.
    dest: u32,
}

impl Atoms {
    fn intern(conn: &RustConnection) -> Result<Self, String> {
        let atom = |name: &str| -> Result<u32, String> {
            conn.intern_atom(false, name.as_bytes())
                .map_err(|e| e.to_string())?
                .reply()
                .map(|r| r.atom)
                .map_err(|e| e.to_string())
        };
        Ok(Self {
            primary: atom("PRIMARY")?,
            clipboard: atom("CLIPBOARD")?,
            utf8: atom("UTF8_STRING")?,
            incr: atom("INCR")?,
            dest: atom("BUBBLETRANSLATE_SELECTION")?,
        })
    }
}

/// Checks that this display can report selection changes, without committing
/// to a watcher.
pub fn probe() -> Result<(), String> {
    let (conn, _) = x11rb::connect(None).map_err(|err| format!("no X11 display: {err}"))?;
    xfixes::query_version(&conn, 5, 0)
        .map_err(|err| format!("XFixes is missing: {err}"))?
        .reply()
        .map_err(|err| format!("XFixes is missing: {err}"))?;
    Ok(())
}

/// Watches the primary selection until the process ends, calling `on_change`
/// with the text every time it moves to a new owner.
///
/// Blocks, so it wants a thread of its own. Two connections: one parked in
/// `wait_for_event` for XFixes notifications, one for the request/reply
/// conversation that reads the text. Splitting them keeps a conversion from
/// having to step over selection events that arrive mid-read.
pub fn watch(mut on_change: impl FnMut(String) + Send + 'static) -> Result<(), String> {
    let (notify_conn, screen_num) = x11rb::connect(None).map_err(|e| e.to_string())?;
    let (read_conn, _) = x11rb::connect(None).map_err(|e| e.to_string())?;

    xfixes::query_version(&notify_conn, 5, 0)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| format!("XFixes is missing: {e}"))?;

    let atoms = Atoms::intern(&read_conn)?;
    let notify_atoms = Atoms::intern(&notify_conn)?;
    let notify_window = hidden_window(&notify_conn, screen_num)?;
    let read_window = hidden_window(&read_conn, screen_num)?;

    // Owner changes only. Ownership ending is not interesting: an empty
    // selection is nothing to translate.
    //
    // The clipboard is subscribed to as well but acted on only when asked for;
    // subscribing to both up front avoids having to tear the watch down and
    // rebuild it when the setting changes.
    for selection in [notify_atoms.primary, notify_atoms.clipboard] {
        xfixes::select_selection_input(
            &notify_conn,
            notify_window,
            selection,
            xfixes::SelectionEventMask::SET_SELECTION_OWNER,
        )
        .map_err(|e| e.to_string())?;
    }
    notify_conn.flush().map_err(|e| e.to_string())?;
    crate::trace!("x11: watching the primary selection");

    loop {
        let event = notify_conn.wait_for_event().map_err(|e| e.to_string())?;
        let Event::XfixesSelectionNotify(notify) = event else {
            continue;
        };
        if notify.selection == notify_atoms.clipboard && !super::monitor::watching_clipboard() {
            continue;
        }
        // Which selection changed is carried on the event, and it is read back
        // on the other connection under the same atom.
        let selection = if notify.selection == notify_atoms.clipboard {
            atoms.clipboard
        } else {
            atoms.primary
        };
        // The owner has only just claimed the selection; asking it to convert
        // immediately is normal and it will answer when ready.
        if let Some(text) = read_selection(&read_conn, read_window, &atoms, selection) {
            on_change(text);
        }
    }
}

/// Blocks while a mouse button is held down, so a selection being dragged out
/// is not acted on until the user lets go.
///
/// X11 reports the selection the moment it changes, and a toolkit that updates
/// the primary selection as a drag grows sends one change per word. Waiting
/// out a quiet period would guess at the end of that gesture; the button mask
/// says it outright, which is the same thing the macOS tap keys off.
///
/// Best effort in both directions: it gives up after `timeout` so a wedged
/// button — or one held for a reason of its own, a game or a scrollbar — can
/// never stop the bubble for good, and it returns immediately if X will not
/// answer, leaving [`crate::engine`]'s settle window as the only filter.
pub fn wait_while_dragging(timeout: Duration) {
    /// Fast enough to feel like the bubble follows the mouse-up, cheap enough
    /// to be nothing next to the round trip a translation costs.
    const POLL: Duration = Duration::from_millis(15);

    // The wheel is deliberately not in here: it presses and releases in the
    // same instant, so waiting on it would be waiting on nothing.
    let buttons = KeyButMask::BUTTON1 | KeyButMask::BUTTON2 | KeyButMask::BUTTON3;

    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return;
    };
    let Some(root) = conn.setup().roots.get(screen_num).map(|screen| screen.root) else {
        return;
    };

    let deadline = Instant::now() + timeout;
    let mut waited = false;
    while Instant::now() < deadline {
        let held = conn
            .query_pointer(root)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| reply.mask.intersects(buttons));
        match held {
            Some(true) => {
                waited = true;
                std::thread::sleep(POLL);
            }
            // Either the button is up or X would not say; both mean stop
            // waiting, and only the first is worth a line.
            _ => {
                if waited {
                    crate::trace!("x11: the drag ended; taking the selection");
                }
                return;
            }
        }
    }
    crate::trace!("x11: a button is still down after {timeout:?}; taking the selection anyway");
}

/// Where the pointer is, in root-window coordinates.
///
/// Reliable on a real X11 session. Under XWayland it is not — the answer there
/// is whatever the compositor last let XWayland see, which is stale or centred
/// whenever the pointer is over a Wayland window — so [`super::cursor`] only
/// calls this on an X11 session.
pub fn pointer() -> Option<(f64, f64)> {
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen_num)?.root;
    let reply = conn.query_pointer(root).ok()?.reply().ok()?;
    reply
        .same_screen
        .then(|| (f64::from(reply.root_x), f64::from(reply.root_y)))
}

/// The screen's size in the same pixels [`pointer`] answers in.
pub fn screen_size() -> Option<(f64, f64)> {
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let screen = conn.setup().roots.get(screen_num)?;
    Some((
        f64::from(screen.width_in_pixels),
        f64::from(screen.height_in_pixels),
    ))
}

/// An unmapped 1×1 window, which is all a client needs to receive selection
/// events and hold the property a conversion is written into.
fn hidden_window(conn: &RustConnection, screen_num: usize) -> Result<u32, String> {
    let screen = conn
        .setup()
        .roots
        .get(screen_num)
        .ok_or("no such X11 screen")?;
    let window = conn.generate_id().map_err(|e| e.to_string())?;
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        window,
        screen.root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )
    .map_err(|e| e.to_string())?;
    conn.flush().map_err(|e| e.to_string())?;
    Ok(window)
}

/// Asks the selection's owner for UTF-8 text and waits for it.
///
/// X11 selections are a conversation, not a store: the request goes out, the
/// owning client writes the answer into a property on *our* window, and a
/// `SelectionNotify` says it is there.
fn read_selection(
    conn: &RustConnection,
    window: u32,
    atoms: &Atoms,
    selection: u32,
) -> Option<String> {
    conn.convert_selection(
        window,
        selection,
        atoms.utf8,
        atoms.dest,
        x11rb::CURRENT_TIME,
    )
    .ok()?;
    conn.flush().ok()?;

    let deadline = Instant::now() + CONVERT_TIMEOUT;
    while Instant::now() < deadline {
        match conn.poll_for_event().ok()? {
            Some(Event::SelectionNotify(notify)) => {
                if notify.property == x11rb::NONE {
                    // The owner has no text form of its selection — an image,
                    // say, or a client that only speaks Latin-1.
                    crate::trace!("x11: the selection owner refused UTF8_STRING");
                    return None;
                }
                let reply = conn
                    .get_property(true, window, notify.property, AtomEnum::ANY, 0, u32::MAX)
                    .ok()?
                    .reply()
                    .ok()?;
                if reply.type_ == atoms.incr {
                    // Incremental transfer, meaning a selection far larger than
                    // anything worth translating. Reading it would take a whole
                    // second conversation; declining costs nothing real.
                    crate::trace!("x11: the selection is too large to read in one piece");
                    return None;
                }
                return String::from_utf8(reply.value).ok();
            }
            Some(_) => continue,
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    crate::trace!("x11: the selection owner never answered");
    None
}
