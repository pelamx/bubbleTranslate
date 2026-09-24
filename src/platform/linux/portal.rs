//! Taking the picture through the desktop's screenshot portal.
//!
//! GNOME and KDE implement no protocol that lets an application copy the
//! screen, on purpose: on those desktops a screenshot is something the
//! desktop takes and hands over, through `org.freedesktop.portal.Screenshot`
//! on the session bus. It is the same route every sandboxed application uses,
//! and it is there on every desktop that ships `xdg-desktop-portal` — which
//! is every mainstream one.
//!
//! Asked first without interaction: the desktop takes the whole screen
//! quietly, and the user draws the rectangle on our own overlay, the same as
//! everywhere else. A desktop that refuses that — a permission it wants to
//! ask for, or no support for it — is asked again interactively, and then its
//! own screenshot interface does the choosing; what comes back is already the
//! region.
//!
//! The portal hands the picture over as a PNG file, which GNOME writes into
//! the user's Pictures folder. It is read and deleted at once: it was taken
//! for us, not by the user, and leaving a stray screenshot behind every time
//! the key is pressed would be littering in someone's own files.

use std::collections::HashMap;
use std::time::Duration;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use super::ocr::Frame;

/// How long the interactive screenshot may stay open. It is the user choosing
/// a region in the desktop's own interface, so this is generous.
const INTERACTIVE_PATIENCE: Duration = Duration::from_secs(180);

/// How long a quiet screenshot may take. Usually a moment — but the first
/// time, the desktop may ask the user whether to allow it, and that answer
/// is theirs to take their time over.
const QUIET_PATIENCE: Duration = Duration::from_secs(60);

/// What the portal gave back.
pub enum Picture {
    /// The whole desktop, for our overlay to choose from.
    Desktop(Frame),
    /// A region the user already chose in the desktop's own interface.
    Chosen(Frame),
}

/// Takes a picture through the portal.
pub fn grab() -> Result<Picture, String> {
    let conn = Connection::session().map_err(|err| format!("no session bus: {err}"))?;
    match screenshot(&conn, false, QUIET_PATIENCE) {
        Ok(frame) => Ok(Picture::Desktop(frame)),
        Err(Refusal::Cancelled) => Err("the screenshot was cancelled".into()),
        Err(Refusal::Failed(reason)) => {
            crate::trace!(
                "portal    a quiet screenshot was refused ({reason}); asking interactively"
            );
            match screenshot(&conn, true, INTERACTIVE_PATIENCE) {
                Ok(frame) => Ok(Picture::Chosen(frame)),
                Err(Refusal::Cancelled) => Err("the screenshot was cancelled".into()),
                Err(Refusal::Failed(reason)) => Err(reason),
            }
        }
    }
}

enum Refusal {
    /// The user said no, or closed the desktop's screenshot interface.
    Cancelled,
    Failed(String),
}

fn screenshot(conn: &Connection, interactive: bool, patience: Duration) -> Result<Frame, Refusal> {
    let failed = |what: &str, err: zbus::Error| Refusal::Failed(format!("{what}: {err}"));

    // The request object's path is known before the call is made, and the
    // answer is subscribed to first: a desktop that answers at once would
    // otherwise answer before anyone was listening.
    let token = format!(
        "bubbleTranslate_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default()
    );
    let sender = conn
        .unique_name()
        .map(|name| name.trim_start_matches(':').replace('.', "_"))
        .ok_or_else(|| Refusal::Failed("the session bus gave us no name".into()))?;
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");

    let request = Proxy::new(
        conn,
        "org.freedesktop.portal.Desktop",
        path.as_str(),
        "org.freedesktop.portal.Request",
    )
    .map_err(|err| failed("no request object", err))?;
    let responses = request
        .receive_signal("Response")
        .map_err(|err| failed("could not listen for the answer", err))?;

    let portal = Proxy::new(
        conn,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Screenshot",
    )
    .map_err(|err| failed("no screenshot portal", err))?;
    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    options.insert("interactive", Value::from(interactive));
    let _: OwnedObjectPath = portal
        .call("Screenshot", &("", options))
        .map_err(|err| failed("the screenshot portal refused the call", err))?;

    // The signal iterator blocks with no deadline of its own, so it is
    // drained on a thread and waited on here with one.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        if let Some(message) = responses.into_iter().next() {
            let _ = tx.send(message);
        }
    });
    let message = rx
        .recv_timeout(patience)
        .map_err(|_| Refusal::Failed("the desktop never answered".into()))?;

    let (code, results): (u32, HashMap<String, OwnedValue>) = message
        .body()
        .deserialize()
        .map_err(|err| failed("an answer that could not be read", err))?;
    match code {
        0 => {}
        1 => return Err(Refusal::Cancelled),
        other => return Err(Refusal::Failed(format!("the desktop answered {other}"))),
    }

    let uri = results
        .get("uri")
        .and_then(|value| String::try_from(value.clone()).ok())
        .ok_or_else(|| Refusal::Failed("the answer carried no picture".into()))?;
    let path =
        file_path(&uri).ok_or_else(|| Refusal::Failed(format!("not a local file: {uri}")))?;

    let frame = read_png(&path);
    let _ = std::fs::remove_file(&path);
    frame.map_err(Refusal::Failed)
}

/// The local path a `file://` URI names, with its escapes undone.
fn file_path(uri: &str) -> Option<std::path::PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let decoded = urlencoding::decode(rest).ok()?;
    Some(std::path::PathBuf::from(decoded.into_owned()))
}

/// Reads a PNG into the four-bytes-a-pixel, blue-first layout [`Frame`] uses.
fn read_png(path: &std::path::Path) -> Result<Frame, String> {
    let file =
        std::fs::File::open(path).map_err(|err| format!("could not open the picture: {err}"))?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    // Palette and 16-bit pictures come out as plain 8-bit colour.
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .map_err(|err| format!("not a picture this can read: {err}"))?;
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or("the picture is too large")?
    ];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|err| format!("the picture could not be decoded: {err}"))?;
    let pixels = &buffer[..info.buffer_size()];

    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        other => return Err(format!("an unexpected picture format: {other:?}")),
    };
    let mut bgrx = Vec::with_capacity(info.width as usize * info.height as usize * 4);
    for row in pixels
        .chunks_exact(info.line_size)
        .take(info.height as usize)
    {
        for pixel in row.chunks_exact(channels).take(info.width as usize) {
            let (r, g, b) = if channels >= 3 {
                (pixel[0], pixel[1], pixel[2])
            } else {
                (pixel[0], pixel[0], pixel[0])
            };
            bgrx.extend_from_slice(&[b, g, r, 0]);
        }
    }
    Ok(Frame {
        width: info.width,
        height: info.height,
        bgrx,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_uri_becomes_its_path() {
        assert_eq!(
            file_path("file:///home/a/Pictures/Screenshot%20from%202026.png"),
            Some("/home/a/Pictures/Screenshot from 2026.png".into())
        );
        assert_eq!(file_path("https://example.com/x.png"), None);
    }
}
