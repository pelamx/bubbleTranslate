//! Reading the screen with Google Cloud Vision, for those who ask for it.
//!
//! Off unless a key is entered in the settings. Without one the screen is
//! read on this computer, as it always was, and nothing about it leaves the
//! machine. With one, the picture of the screen — a box, or the whole of it —
//! is sent to Google to be read, which reads small and faint text, accented
//! letters and every script far better than a reader that has to be installed
//! language by language, and in a second rather than several.
//!
//! The key is the user's own, billed to their own account. It is never built
//! into the app: a key in a binary is a key anyone can take out of it.
//!
//! Anything that goes wrong — no network, a refused key, an answer that
//! cannot be read — is not an error the user sees: the screen is read on the
//! computer instead.

use std::sync::Mutex;
use std::time::Duration;

use base64::Engine as _;
use serde_json::Value;

use crate::page::{Rect, UNSCORED, Word};

/// The key from the settings, handed over by the engine before each request
/// so the readers below the platform line need not be given the config.
static KEY: Mutex<String> = Mutex::new(String::new());

/// The largest picture sent, in bytes of PNG. Vision takes requests up to
/// ten megabytes, and base64 makes a picture a third bigger again.
const MAX_PNG: usize = 7 * 1024 * 1024;

pub fn set_key(key: &str) {
    if let Ok(mut slot) = KEY.lock() {
        key.trim().clone_into(&mut slot);
    }
}

/// The key, when there is one.
pub fn key() -> Option<String> {
    KEY.lock()
        .ok()
        .map(|key| key.clone())
        .filter(|key| !key.is_empty())
}

/// Every word in the picture and where it is, line by line, in the picture's
/// own pixels. `bgrx` is four bytes a pixel, blue first.
pub fn read_words(
    key: &str,
    width: u32,
    height: u32,
    bgrx: &[u8],
) -> Result<Vec<Vec<Word>>, String> {
    let answer = annotate(key, width, height, bgrx)?;
    Ok(lines_of(&answer))
}

/// All the text in the picture, as Vision read it.
pub fn read_text(key: &str, width: u32, height: u32, bgrx: &[u8]) -> Result<String, String> {
    let answer = annotate(key, width, height, bgrx)?;
    Ok(answer
        .pointer("/responses/0/fullTextAnnotation/text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string())
}

fn annotate(key: &str, width: u32, height: u32, bgrx: &[u8]) -> Result<Value, String> {
    let started = std::time::Instant::now();
    let png = grey_png(width, height, bgrx)?;
    if png.len() > MAX_PNG {
        return Err(format!(
            "the picture is too big to send ({} bytes)",
            png.len()
        ));
    }
    let body = serde_json::json!({
        "requests": [{
            "image": { "content": base64::engine::general_purpose::STANDARD.encode(&png) },
            // The document reader rather than the plain one: it keeps
            // paragraphs and line breaks, which is what the page is rebuilt
            // from.
            "features": [{ "type": "DOCUMENT_TEXT_DETECTION" }],
        }]
    });
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .build(),
    );
    let response = agent
        .post("https://vision.googleapis.com/v1/images:annotate")
        // A header rather than `?key=`: the key stays out of anything that
        // logs addresses.
        .header("x-goog-api-key", key)
        .header("Content-Type", "application/json")
        .send(body.to_string().as_str())
        .map_err(|err| format!("could not reach Google Vision: {err}"))?;
    let status = response.status().as_u16();
    let text = response
        .into_body()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_string()
        .map_err(|err| format!("Google Vision's answer did not arrive whole: {err}"))?;
    let answer: Value = serde_json::from_str(&text)
        .map_err(|err| format!("Google Vision answered with something unreadable: {err}"))?;
    if status != 200 {
        let why = answer
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("no reason given");
        return Err(format!("Google Vision refused ({status}): {why}"));
    }
    if let Some(why) = answer
        .pointer("/responses/0/error/message")
        .and_then(Value::as_str)
    {
        return Err(format!("Google Vision could not read it: {why}"));
    }
    crate::trace!(
        "vision    read {} bytes of picture in {:?}",
        png.len(),
        started.elapsed()
    );
    Ok(answer)
}

/// The picture as an 8-bit grey PNG: a third of the size of colour, and all
/// a reader needs.
fn grey_png(width: u32, height: u32, bgrx: &[u8]) -> Result<Vec<u8>, String> {
    let grey: Vec<u8> = bgrx
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| {
            ((299 * u32::from(p[2]) + 587 * u32::from(p[1]) + 114 * u32::from(p[0])) / 1000) as u8
        })
        .collect();
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|err| err.to_string())?;
    writer
        .write_image_data(&grey)
        .map_err(|err| err.to_string())?;
    writer.finish().map_err(|err| err.to_string())?;
    Ok(out)
}

/// Vision's answer as lines of words.
///
/// It answers in pages, blocks, paragraphs, words and letters, and says
/// after each letter what came next: a space, or the end of the line. A line
/// is the words up to the end of one; a paragraph ending ends one too.
fn lines_of(answer: &Value) -> Vec<Vec<Word>> {
    let list = |value: &Value, key: &str| -> Vec<Value> {
        value
            .get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let mut lines = Vec::new();
    let Some(document) = answer.pointer("/responses/0/fullTextAnnotation") else {
        return lines;
    };
    for page in list(document, "pages") {
        for block in list(&page, "blocks") {
            for paragraph in list(&block, "paragraphs") {
                let mut line: Vec<Word> = Vec::new();
                for word in list(&paragraph, "words") {
                    let symbols = list(&word, "symbols");
                    let mut text: String = symbols
                        .iter()
                        .filter_map(|s| s.get("text").and_then(Value::as_str))
                        .collect();
                    let ending = symbols
                        .last()
                        .and_then(|s| s.pointer("/property/detectedBreak/type"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if ending == "HYPHEN" {
                        text.push('-');
                    }
                    if let Some(rect) = rect_of(&word) {
                        line.push(Word {
                            rect,
                            text,
                            confidence: word
                                .get("confidence")
                                .and_then(Value::as_f64)
                                .map_or(UNSCORED, |c| (c * 100.0) as f32),
                        });
                    }
                    if matches!(ending, "EOL_SURE_SPACE" | "LINE_BREAK" | "HYPHEN")
                        && !line.is_empty()
                    {
                        lines.push(std::mem::take(&mut line));
                    }
                }
                if !line.is_empty() {
                    lines.push(line);
                }
            }
        }
    }
    lines
}

/// The box around a word, from the corners Vision gives — which leave out a
/// coordinate that is zero.
fn rect_of(word: &Value) -> Option<Rect> {
    let corners = word.pointer("/boundingBox/vertices")?.as_array()?;
    let xs: Vec<i64> = corners
        .iter()
        .map(|v| v.get("x").and_then(Value::as_i64).unwrap_or(0))
        .collect();
    let ys: Vec<i64> = corners
        .iter()
        .map(|v| v.get("y").and_then(Value::as_i64).unwrap_or(0))
        .collect();
    let (x0, x1) = (*xs.iter().min()?, *xs.iter().max()?);
    let (y0, y1) = (*ys.iter().min()?, *ys.iter().max()?);
    Some(Rect {
        x: x0.max(0) as u32,
        y: y0.max(0) as u32,
        width: (x1 - x0).max(1) as u32,
        height: (y1 - y0).max(1) as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_becomes_lines_of_words() {
        let word = |text: &str, x: i64, ending: &str| {
            let mut symbols: Vec<Value> = text
                .chars()
                .map(|c| serde_json::json!({ "text": c.to_string() }))
                .collect();
            if let Some(last) = symbols.last_mut() {
                last["property"] = serde_json::json!({ "detectedBreak": { "type": ending } });
            }
            serde_json::json!({
                "boundingBox": { "vertices": [
                    { "x": x, "y": 10 }, { "x": x + 40, "y": 10 },
                    { "x": x + 40, "y": 30 }, { "x": x }
                ]},
                "symbols": symbols,
                "confidence": 0.98,
            })
        };
        let answer = serde_json::json!({ "responses": [{ "fullTextAnnotation": { "pages": [{
            "blocks": [{ "paragraphs": [{ "words": [
                word("Hello", 0, "SPACE"),
                word("wor", 50, "HYPHEN"),
                word("ld", 0, "LINE_BREAK"),
            ]}]}]
        }]}}]});
        let lines = lines_of(&answer);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].len(), 2);
        assert_eq!(lines[0][1].text, "wor-");
        assert_eq!(lines[1][0].text, "ld");
        // A corner left out is a coordinate of zero.
        assert_eq!(
            lines[0][0].rect,
            Rect {
                x: 0,
                y: 0,
                width: 40,
                height: 30
            }
        );
        assert!((lines[0][0].confidence - 98.0).abs() < 0.01);
    }

    #[test]
    fn a_grey_png_is_a_png() {
        let png = grey_png(2, 1, &[0, 0, 0, 0, 255, 255, 255, 0]).unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }

    #[test]
    fn no_key_means_no_vision() {
        set_key("   ");
        assert!(key().is_none());
    }
}
