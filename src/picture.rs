//! Pictures of the screen, made ready for Claude to read.
//!
//! When the user has set Claude up to translate what is read off the screen
//! (see [`crate::config::Config::screen_goes_to_claude`]), the picture itself
//! goes to Claude Haiku with the text the computer's own reader made of it,
//! and Haiku reads it again: a misread letter is no longer something to guess
//! back from the words around it but something to look at. The computer's
//! reader still runs, because it is the one that knows where each paragraph
//! is, and it is what answers if Claude cannot be reached.
//!
//! The picture lives only as long as the request. Nothing here keeps it.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the platform readers should hand the picture up, set by the
/// engine before each request so the readers below the platform line need not
/// be given the config — the arrangement [`crate::vision`] has for its key.
static WANTED: AtomicBool = AtomicBool::new(false);

pub fn set_wanted(wanted: bool) {
    WANTED.store(wanted, Ordering::Relaxed);
}

pub fn wanted() -> bool {
    WANTED.load(Ordering::Relaxed)
}

/// The longest side a picture is sent at. Past it the model would scale the
/// picture down itself, and a picture made smaller here costs less to send
/// and is halved cleanly rather than resampled.
const MAX_EDGE: u32 = 2576;

/// The most PNG sent in one picture. The API takes five megabytes of base64,
/// which is a third bigger than the bytes it encodes.
const MAX_BYTES: usize = 3_500_000;

/// The picture as a grey PNG small enough to send: halved until it fits.
/// `bgrx` is four bytes a pixel, blue first, top row first.
///
/// Grey because a reader needs the letters and not their colour, and grey is
/// a third the size.
pub fn encode(width: u32, height: u32, bgrx: &[u8]) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || bgrx.len() < width as usize * height as usize * 4 {
        return None;
    }
    let grey: Vec<u8> = bgrx
        .chunks_exact(4)
        .take(width as usize * height as usize)
        .map(|p| luma(p[2], p[1], p[0]))
        .collect();
    fit(width, height, grey)
}

/// The same, for a picture that is already a PNG: the box macOS draws is
/// written to a file by the system's own screenshot tool.
pub fn encode_png(png: &[u8]) -> Option<Vec<u8>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    let bytes = &buffer[..info.buffer_size()];
    let grey: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => bytes
            .chunks_exact(4)
            .map(|p| luma(p[0], p[1], p[2]))
            .collect(),
        png::ColorType::Rgb => bytes
            .chunks_exact(3)
            .map(|p| luma(p[0], p[1], p[2]))
            .collect(),
        png::ColorType::GrayscaleAlpha => bytes.chunks_exact(2).map(|p| p[0]).collect(),
        png::ColorType::Grayscale => bytes.to_vec(),
        // Expanded to one of the above by the transformation.
        png::ColorType::Indexed => return None,
    };
    fit(info.width, info.height, grey)
}

fn luma(red: u8, green: u8, blue: u8) -> u8 {
    ((299 * u32::from(red) + 587 * u32::from(green) + 114 * u32::from(blue)) / 1000) as u8
}

fn fit(mut width: u32, mut height: u32, mut grey: Vec<u8>) -> Option<Vec<u8>> {
    while width.max(height) > MAX_EDGE {
        (width, height, grey) = halve(width, height, &grey);
    }
    loop {
        let png = grey_png(width, height, &grey)?;
        if png.len() <= MAX_BYTES || width.max(height) < 64 {
            return Some(png);
        }
        (width, height, grey) = halve(width, height, &grey);
    }
}

/// Half the size each way, each pixel the average of the four it replaces.
fn halve(width: u32, height: u32, grey: &[u8]) -> (u32, u32, Vec<u8>) {
    let (w, h) = ((width / 2).max(1), (height / 2).max(1));
    let at =
        |x: u32, y: u32| u32::from(grey[(y.min(height - 1) * width + x.min(width - 1)) as usize]);
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let sum = at(2 * x, 2 * y)
                + at(2 * x + 1, 2 * y)
                + at(2 * x, 2 * y + 1)
                + at(2 * x + 1, 2 * y + 1);
            out.push((sum / 4) as u8);
        }
    }
    (w, h, out)
}

fn grey_png(width: u32, height: u32, grey: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(grey).ok()?;
    writer.finish().ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size_of(png: &[u8]) -> (u32, u32) {
        let reader = png::Decoder::new(std::io::Cursor::new(png))
            .read_info()
            .unwrap();
        (reader.info().width, reader.info().height)
    }

    /// A picture that already fits is sent as it was taken.
    #[test]
    fn a_small_picture_keeps_its_size() {
        let png = encode(300, 120, &vec![200; 300 * 120 * 4]).unwrap();
        assert_eq!(size_of(&png), (300, 120));
    }

    /// A Retina screen is halved to fit, keeping its shape.
    #[test]
    fn a_large_picture_is_halved_until_it_fits() {
        let png = encode(3024, 1964, &vec![255; 3024 * 1964 * 4]).unwrap();
        assert_eq!(size_of(&png), (1512, 982));
    }

    /// A PNG goes through the same way as raw pixels.
    #[test]
    fn a_png_is_read_back_and_made_grey() {
        let rgba = encode(10, 4, &[90; 10 * 4 * 4]).unwrap();
        let again = encode_png(&rgba).unwrap();
        assert_eq!(size_of(&again), (10, 4));
    }

    #[test]
    fn a_short_buffer_is_refused() {
        assert!(encode(10, 10, &[0; 12]).is_none());
    }
}
