//! Reading text out of pixels with nothing installed: ocrs, a small neural
//! reader written in Rust, running two models on this machine.
//!
//! Tesseract reads better — every accented letter, every alphabet it has a
//! pack for — but it is a package the user has to install, and until they do
//! the key that reads the screen would do nothing. This is what answers in
//! the meantime, and for anyone who never installs it. Its models know the
//! unaccented Latin alphabet, digits and punctuation, which is most of what
//! is on a screen in English and enough of it in most languages written in
//! Latin letters for a translator to understand.
//!
//! The models are twelve megabytes, which would nearly double a download
//! most people use only to translate selections; they are fetched the first
//! time the screen is read instead, checked against the hashes below, and
//! kept. The picture itself still never leaves the machine — only the models
//! travel, and only towards it.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use sha2::{Digest, Sha256};

use super::ocr::Frame;

/// One model file, and what it must hash to.
struct Model {
    name: &'static str,
    size: u64,
    sha256: &'static str,
}

/// Word detection and line recognition, as published by the ocrs project and
/// mirrored beside the downloads so they stay where installed copies look.
const MODELS: [Model; 2] = [
    Model {
        name: "text-detection.rten",
        size: 2_510_284,
        sha256: "f15cfb56bd02c4bf478a20343986504a1f01e1665c2b3a0ad66340f054b1b5ca",
    },
    Model {
        name: "text-recognition.rten",
        size: 9_716_568,
        sha256: "e484866d4cce403175bd8d00b128feb08ab42e208de30e42cd9889d8f1735a6e",
    },
];

/// Where the models are published: a release of their own in the downloads
/// repository, marked as a pre-release so it never becomes the one
/// `/releases/latest/` points at.
const BASE_URL: &str =
    "https://github.com/bubbleTranslate/downloads/releases/download/ocr-models-1";

/// Where this reader stands on this machine.
#[derive(Debug, Clone, PartialEq)]
pub enum State {
    /// The models have never been fetched.
    Absent,
    /// Being fetched; how far, from 0 to 1.
    Downloading(f32),
    /// On disk and ready.
    Ready,
    /// The last attempt failed, and why.
    Failed(String),
}

/// `None` until first asked, when the disk is looked at once.
static STATE: Mutex<Option<State>> = Mutex::new(None);

fn set(state: State) {
    if let Ok(mut slot) = STATE.lock() {
        *slot = Some(state);
    }
}

/// Where this reader stands.
pub fn state() -> State {
    let Ok(mut slot) = STATE.lock() else {
        return State::Absent;
    };
    slot.get_or_insert_with(|| {
        if MODELS
            .iter()
            .all(|m| std::fs::metadata(dir().join(m.name)).is_ok_and(|f| f.len() == m.size))
        {
            State::Ready
        } else {
            State::Absent
        }
    })
    .clone()
}

/// Where the models are kept: beside the licence and the allowance.
fn dir() -> PathBuf {
    if let Some(home) = crate::config::state_home() {
        return home.join("ocr");
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bubbleTranslate")
        .join("ocr")
}

/// Fetches the models in the background, unless they are here or on their
/// way already. The window follows along through [`state`].
pub fn start_download() {
    if matches!(state(), State::Ready | State::Downloading(_)) {
        return;
    }
    set(State::Downloading(0.0));
    let _ = std::thread::Builder::new()
        .name("ocr-models".into())
        .spawn(|| {
            let _ = download();
        });
}

/// Makes sure the models are here, fetching them now if they are not.
///
/// Blocks, so it is only called off the interface thread — by the key, which
/// has nothing to show until the reader is ready anyway.
pub fn ensure() -> Result<(), String> {
    loop {
        match state() {
            State::Ready => return Ok(()),
            // The window started it; wait for that one rather than racing it.
            State::Downloading(_) => std::thread::sleep(Duration::from_millis(100)),
            State::Absent | State::Failed(_) => {
                set(State::Downloading(0.0));
                return download();
            }
        }
    }
}

fn download() -> Result<(), String> {
    let result = fetch_all();
    match &result {
        Ok(()) => set(State::Ready),
        Err(err) => {
            crate::trace!("ocr       model download failed: {err}");
            set(State::Failed(err.clone()));
        }
    }
    result
}

fn fetch_all() -> Result<(), String> {
    let dir = dir();
    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(15)))
            // Twelve megabytes on a slow connection; past this it is not slow
            // but stuck.
            .timeout_global(Some(Duration::from_secs(600)))
            .user_agent(concat!("bubbleTranslate/", env!("CARGO_PKG_VERSION")))
            .build(),
    );
    let total: u64 = MODELS.iter().map(|m| m.size).sum();
    let mut done = 0u64;
    for model in &MODELS {
        let path = dir.join(model.name);
        if std::fs::metadata(&path).is_ok_and(|f| f.len() == model.size) {
            done += model.size;
            continue;
        }
        let url = format!("{BASE_URL}/{}", model.name);
        let response = agent.get(&url).call().map_err(|err| err.to_string())?;
        let mut body = response.into_body().into_reader();

        // Written beside the real name and renamed once it hashes right, so a
        // download cut short never looks like a model.
        let part = path.with_extension("part");
        let mut file = std::fs::File::create(&part).map_err(|err| err.to_string())?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut got = 0u64;
        loop {
            let n = body.read(&mut buf).map_err(|err| err.to_string())?;
            if n == 0 {
                break;
            }
            got += n as u64;
            if got > model.size {
                let _ = std::fs::remove_file(&part);
                return Err(format!("{} is larger than it should be", model.name));
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n]).map_err(|err| err.to_string())?;
            set(State::Downloading((done + got) as f32 / total as f32));
        }
        drop(file);
        if hex(&hasher.finalize()) != model.sha256 {
            let _ = std::fs::remove_file(&part);
            return Err(format!("{} did not match its checksum", model.name));
        }
        std::fs::rename(&part, &path).map_err(|err| err.to_string())?;
        done += model.size;
    }
    crate::trace!("ocr       models are in {}", dir.display());
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The engine, loaded once and kept: loading reads twelve megabytes, reading
/// a region takes a fraction of that time.
static ENGINE: Mutex<Option<&'static ocrs::OcrEngine>> = Mutex::new(None);

fn engine() -> Result<&'static ocrs::OcrEngine, String> {
    let mut slot = ENGINE.lock().map_err(|_| "the engine lock was poisoned")?;
    if let Some(engine) = *slot {
        return Ok(engine);
    }
    let load = |model: &Model| -> Result<rten::Model, String> {
        let bytes = std::fs::read(dir().join(model.name)).map_err(|err| err.to_string())?;
        // Checked again here, not only after the download: a file that went
        // bad on disk is fetched again instead of read as nonsense.
        if hex(&Sha256::digest(&bytes)) != model.sha256 {
            let _ = std::fs::remove_file(dir().join(model.name));
            set(State::Absent);
            return Err(format!("{} was damaged and has been removed", model.name));
        }
        rten::Model::load(bytes).map_err(|err| err.to_string())
    };
    let engine = ocrs::OcrEngine::new(ocrs::OcrEngineParams {
        detection_model: Some(load(&MODELS[0])?),
        recognition_model: Some(load(&MODELS[1])?),
        ..Default::default()
    })
    .map_err(|err| err.to_string())?;
    let engine: &'static ocrs::OcrEngine = Box::leak(Box::new(engine));
    *slot = Some(engine);
    Ok(engine)
}

/// Reads whatever text is in `frame`, one line of the screen to a line.
///
/// `None` when the models are not here or could not be run.
pub fn recognize(frame: &Frame, scale: f64) -> Option<String> {
    let engine = match engine() {
        Ok(engine) => engine,
        Err(err) => {
            crate::trace!("ocr       the built-in reader did not load: {err}");
            return None;
        }
    };
    // The same enlargement and margin Tesseract gets, for the same reasons:
    // letters under ten pixels tall and letters touching the edge both read
    // worse here too.
    let (grey, width, height) = frame.prepared(scale);
    let started = std::time::Instant::now();
    let source = ocrs::ImageSource::from_bytes(&grey, (width, height)).ok()?;
    let input = engine.prepare_input(source).ok()?;
    let words = engine.detect_words(&input).ok()?;
    let lines = engine.find_text_lines(&input, &words);
    let text = engine
        .recognize_text(&input, &lines)
        .ok()?
        .into_iter()
        .flatten()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    crate::trace!(
        "ocr       built-in reader read {} chars from {width}x{height} in {:?}",
        text.chars().count(),
        started.elapsed()
    );
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fetches the models into `BUBBLETRANSLATE_HOME` and reads the PNG named
    /// by `BUBBLETRANSLATE_OCR_SAMPLE`: the whole path, network included,
    /// which is why it is not part of the ordinary run.
    #[test]
    #[ignore]
    fn reads_a_sample_after_fetching_the_models() {
        let path = std::env::var("BUBBLETRANSLATE_OCR_SAMPLE").expect("a PNG to read");
        let decoder =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        let channels = info.line_size / info.width as usize;
        let mut bgrx = Vec::new();
        for px in buf[..info.buffer_size()].chunks(channels) {
            bgrx.extend_from_slice(&[px[2], px[1], px[0], 0]);
        }
        let frame = Frame {
            width: info.width,
            height: info.height,
            bgrx,
        };
        ensure().unwrap();
        let text = recognize(&frame, 2.0).unwrap();
        println!("{text}");
        assert!(!text.trim().is_empty());
    }
}
