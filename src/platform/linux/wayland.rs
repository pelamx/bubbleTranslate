//! Reading and writing selections on Wayland, over `wlr-data-control`.
//!
//! Wayland ties clipboard access to keyboard focus on purpose: an ordinary
//! client may only read the selection while it is the one being typed into.
//! That rules out `wl_data_device` for anything like this — a translator that
//! watches selections made in *other* applications is, by construction, never
//! the focused client.
//!
//! `wlr-data-control` is the protocol that exists for that case. It was
//! written for clipboard managers, and it hands over the selection regardless
//! of focus. wlroots compositors (Hyprland, sway, river, Wayfire), KWin and
//! COSMIC all implement it; GNOME does not, which is why [`super::Backend`]
//! has a case for having nowhere to go.
//!
//! Everything here runs on one connection owned by one thread, because a
//! data-control offer is only readable from the connection it arrived on.

use std::collections::HashMap;
use std::io::Read;
use std::os::fd::AsFd;

use wayland_client::backend::ObjectId;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::{self, ZwlrDataControlOfferV1},
    zwlr_data_control_source_v1::{self, ZwlrDataControlSourceV1},
};

/// Version 2 is the floor: `primary_selection` — the whole point of this file
/// — was added there. A compositor stuck on version 1 can only offer the
/// clipboard, which is not what selecting text fills in.
const REQUIRED_MANAGER_VERSION: u32 = 2;

/// What we ask for, best first. The first one an offer advertises wins.
const TEXT_MIMES: [&str; 5] = [
    "text/plain;charset=utf-8",
    "text/plain;charset=UTF-8",
    "UTF8_STRING",
    "text/plain",
    "STRING",
];

/// Checks that this compositor can serve selections without focus, without
/// committing to a watcher.
///
/// Returns the error text to show the user when it cannot, which is a real
/// possibility rather than an edge case: on GNOME it is always this path.
pub fn probe() -> Result<(), String> {
    let conn = Connection::connect_to_env()
        .map_err(|err| format!("could not connect to the Wayland compositor: {err}"))?;
    let mut queue = conn.new_event_queue();
    conn.display().get_registry(&queue.handle(), ());

    let mut globals = Globals::default();
    queue
        .roundtrip(&mut globals)
        .map_err(|err| format!("the Wayland compositor did not answer: {err}"))?;

    match globals.manager_version {
        None => Err("this compositor does not implement wlr-data-control".to_string()),
        Some(version) if version < REQUIRED_MANAGER_VERSION => Err(format!(
            "this compositor implements wlr-data-control v{version}, and reading \
             the primary selection needs v{REQUIRED_MANAGER_VERSION}"
        )),
        Some(_) if globals.seat_name.is_none() => {
            Err("this compositor advertised no seat".to_string())
        }
        Some(_) => Ok(()),
    }
}

/// Watches the primary selection until the process ends, calling `on_change`
/// with the text every time it is replaced.
///
/// Blocks, so it wants a thread of its own. The selection in place when the
/// watch starts is recorded but not reported: it is whatever the user was
/// doing before bubbleTranslate existed, and translating it unbidden at
/// startup would be a surprise.
pub fn watch(on_change: impl FnMut(String) + Send + 'static) -> Result<(), String> {
    let conn = Connection::connect_to_env()
        .map_err(|err| format!("could not connect to the Wayland compositor: {err}"))?;

    // An event queue is bound to one state type for its whole life, so the
    // watcher — not a scratch struct — is what the registry is bound through.
    let mut watcher = Watcher {
        globals: Globals::default(),
        offers: HashMap::new(),
        on_change: Box::new(on_change),
        primed: false,
        clipboard_primed: false,
    };

    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    queue
        .roundtrip(&mut watcher)
        .map_err(|err| format!("the Wayland compositor did not answer: {err}"))?;

    let (Some(manager), Some(seat)) = (
        watcher.globals.manager.clone(),
        watcher.globals.seat.clone(),
    ) else {
        return Err("this compositor does not implement wlr-data-control".to_string());
    };

    // The device is what turns the connection into a subscription: from here
    // the compositor pushes every selection change at us.
    let _device = manager.get_data_device(&seat, &qh, ());

    loop {
        queue
            .blocking_dispatch(&mut watcher)
            .map_err(|err| format!("the Wayland connection ended: {err}"))?;
    }
}

/// Claims the clipboard and serves it until something else claims it.
///
/// Spawns a thread with its own connection, the way `wl-copy` forks: whoever
/// owns a Wayland selection has to stay around to answer paste requests, since
/// the data is handed over on demand rather than stored by the compositor.
/// The thread ends when the compositor cancels the source, which is exactly
/// when the text stops being the clipboard's contents.
pub fn set_clipboard(text: String) {
    std::thread::Builder::new()
        .name("wayland-clipboard".into())
        .spawn(move || {
            if let Err(err) = serve_clipboard(text) {
                crate::trace!("clipboard: {err}");
            }
        })
        .ok();
}

fn serve_clipboard(text: String) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|err| err.to_string())?;
    let mut server = ClipboardSource {
        globals: Globals::default(),
        text,
        cancelled: false,
    };

    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut server).map_err(|e| e.to_string())?;

    let (Some(manager), Some(seat)) = (
        server.globals.manager.clone(),
        server.globals.seat.clone(),
    ) else {
        return Err("no data-control manager".to_string());
    };

    let device = manager.get_data_device(&seat, &qh, ());
    let source = manager.create_data_source(&qh, ());
    for mime in TEXT_MIMES {
        source.offer(mime.to_string());
    }
    device.set_selection(Some(&source));

    while !server.cancelled {
        queue
            .blocking_dispatch(&mut server)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// -- registry --------------------------------------------------------------

/// The two globals every path here needs. Kept separate from the states that
/// embed it so the registry is bound in exactly one place.
#[derive(Default)]
struct Globals {
    manager: Option<ZwlrDataControlManagerV1>,
    manager_version: Option<u32>,
    seat: Option<wl_seat::WlSeat>,
    seat_name: Option<u32>,
}

/// Lets the three states below share one registry implementation.
trait HasGlobals {
    fn globals(&mut self) -> &mut Globals;
}

impl HasGlobals for Globals {
    fn globals(&mut self) -> &mut Globals {
        self
    }
}

macro_rules! registry_dispatch {
    ($state:ty) => {
        impl Dispatch<wl_registry::WlRegistry, ()> for $state {
            fn event(
                state: &mut Self,
                registry: &wl_registry::WlRegistry,
                event: wl_registry::Event,
                _: &(),
                _: &Connection,
                qh: &QueueHandle<Self>,
            ) {
                let wl_registry::Event::Global {
                    name,
                    interface,
                    version,
                } = event
                else {
                    return;
                };
                let globals = state.globals();
                match interface.as_str() {
                    "zwlr_data_control_manager_v1" => {
                        globals.manager_version = Some(version);
                        let bind = version.min(REQUIRED_MANAGER_VERSION);
                        if bind >= REQUIRED_MANAGER_VERSION {
                            globals.manager = Some(registry.bind(name, bind, qh, ()));
                        }
                    }
                    // The first seat is the right one: data-control is
                    // per-seat, and a second seat is a multi-user setup that
                    // would want a second instance of the app anyway.
                    "wl_seat" if globals.seat.is_none() => {
                        globals.seat_name = Some(name);
                        globals.seat = Some(registry.bind(name, 1, qh, ()));
                    }
                    _ => {}
                }
            }
        }

        impl Dispatch<wl_seat::WlSeat, ()> for $state {
            fn event(
                _: &mut Self,
                _: &wl_seat::WlSeat,
                _: wl_seat::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }

        impl Dispatch<ZwlrDataControlManagerV1, ()> for $state {
            fn event(
                _: &mut Self,
                _: &ZwlrDataControlManagerV1,
                _: <ZwlrDataControlManagerV1 as Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    };
}

registry_dispatch!(Globals);
registry_dispatch!(Watcher);
registry_dispatch!(ClipboardSource);

// -- watching --------------------------------------------------------------

struct Watcher {
    globals: Globals,
    /// MIME types announced per offer. An offer arrives empty and is described
    /// by a burst of `offer` events before the selection event that uses it.
    offers: HashMap<ObjectId, Vec<String>>,
    on_change: Box<dyn FnMut(String) + Send>,
    /// False until the compositor has told us what the selection already was.
    primed: bool,
    /// The same, for the clipboard, which is announced separately.
    clipboard_primed: bool,
}

impl HasGlobals for Watcher {
    fn globals(&mut self) -> &mut Globals {
        &mut self.globals
    }
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for Watcher {
    fn event(
        state: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        conn: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_device_v1::Event::DataOffer { id } => {
                state.offers.insert(id.id(), Vec::new());
            }
            zwlr_data_control_device_v1::Event::PrimarySelection { id } => {
                let Some(offer) = id else {
                    // The selection was cleared; nothing to translate.
                    return;
                };
                let mimes = state.offers.remove(&offer.id()).unwrap_or_default();
                let text = receive(conn, &offer, &mimes);
                offer.destroy();

                // The first event describes the selection that already existed
                // when we connected. Recording it and staying quiet is what
                // keeps the app from popping a bubble the moment it starts.
                if !state.primed {
                    state.primed = true;
                    crate::trace!("wayland: primed with the existing selection");
                    return;
                }
                if let Some(text) = text {
                    (state.on_change)(text);
                }
            }
            zwlr_data_control_device_v1::Event::Selection { id } => {
                // The clipboard rather than the selection. Normally not our
                // business, but it is the only gesture that reaches the
                // desktop from an application that draws its own text and
                // publishes no selection, so it is watched on request.
                let Some(offer) = id else {
                    return;
                };
                let mimes = state.offers.remove(&offer.id()).unwrap_or_default();
                let wanted = super::monitor::watching_clipboard();
                // Reading it is what costs; the offer has to be destroyed
                // either way or it leaks until the device dies.
                let text = wanted.then(|| receive(conn, &offer, &mimes)).flatten();
                offer.destroy();

                if !state.clipboard_primed {
                    state.clipboard_primed = true;
                    return;
                }
                if let Some(text) = text {
                    (state.on_change)(text);
                }
            }
            zwlr_data_control_device_v1::Event::Finished => {
                crate::trace!("wayland: the compositor ended the data-control device");
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(Watcher, ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ZwlrDataControlOfferV1, ()> for Watcher {
    fn event(
        state: &mut Self,
        offer: &ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            state.offers.entry(offer.id()).or_default().push(mime_type);
        }
    }
}

/// Pulls the text out of an offer through a pipe.
///
/// The compositor stores nothing: it passes our write end to whichever client
/// owns the selection, and that client writes the bytes. Blocking on the read
/// is therefore waiting on another process, not on our own event loop — the
/// flush is what sets that in motion, so it has to come before the read.
fn receive(conn: &Connection, offer: &ZwlrDataControlOfferV1, mimes: &[String]) -> Option<String> {
    let mime = TEXT_MIMES
        .iter()
        .find(|wanted| mimes.iter().any(|m| m == *wanted))?;

    let (mut reader, writer) = std::io::pipe().ok()?;
    offer.receive(mime.to_string(), writer.as_fd());
    conn.flush().ok()?;
    // Our own copy of the write end has to go, or the read below never sees
    // end-of-file and hangs waiting on a pipe we are holding open ourselves.
    drop(writer);

    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).ok()?;
    String::from_utf8(buf).ok()
}

// -- owning the clipboard --------------------------------------------------

struct ClipboardSource {
    globals: Globals,
    text: String,
    cancelled: bool,
}

impl HasGlobals for ClipboardSource {
    fn globals(&mut self) -> &mut Globals {
        &mut self.globals
    }
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for ClipboardSource {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        _: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    wayland_client::event_created_child!(ClipboardSource, ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ZwlrDataControlOfferV1, ()> for ClipboardSource {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlOfferV1,
        _: zwlr_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlSourceV1, ()> for ClipboardSource {
    fn event(
        state: &mut Self,
        _: &ZwlrDataControlSourceV1,
        event: zwlr_data_control_source_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_source_v1::Event::Send { fd, .. } => {
                // A paste is in progress somewhere. Any MIME type we get asked
                // for is one we offered, and they are all this same text.
                let mut file = std::fs::File::from(fd);
                let _ = std::io::Write::write_all(&mut file, state.text.as_bytes());
            }
            zwlr_data_control_source_v1::Event::Cancelled => {
                // Someone else copied something; the text is no longer the
                // clipboard's, and this thread has nothing left to serve.
                state.cancelled = true;
            }
            _ => {}
        }
    }
}
