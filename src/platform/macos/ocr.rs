//! Reading text out of an image, with the recogniser macOS ships.
//!
//! Vision runs on the device. That matters more here than anywhere else in the
//! app: the picture handed to it is whatever was on the user's screen, which is
//! the most sensitive thing bubbleTranslate ever touches. It never leaves the
//! machine, there is no key to configure, and it works with the network off.

use std::path::Path;
use std::sync::OnceLock;

use objc2::AnyThread;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2_core_foundation::CGAffineTransform;
use objc2_core_image::CIImage;
use objc2_foundation::{NSArray, NSData, NSDictionary, NSNumber, NSString};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
};

/// Whether this machine can read anything at all.
///
/// Vision is part of the system, so the only way to have nothing is to have no
/// recognition languages — which does not happen in practice, but answering
/// from the list rather than from `true` keeps the question honest.
pub fn available() -> bool {
    !languages().is_empty()
}

/// The languages this machine's Vision can read, as it reports them.
///
/// Asked once and kept: the answer cannot change inside a process, and it
/// grows with every macOS release — which is exactly why it is asked instead
/// of being written down here.
pub fn languages() -> &'static [String] {
    static LANGUAGES: OnceLock<Vec<String>> = OnceLock::new();
    LANGUAGES
        .get_or_init(|| {
            autoreleasepool(|_| {
                let request = VNRecognizeTextRequest::new();
                request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
                unsafe { request.supportedRecognitionLanguagesAndReturnError() }
                    .map(|list| list.iter().map(|l| l.to_string()).collect())
                    .unwrap_or_default()
            })
        })
        .as_slice()
}

/// Reads the text out of an image file, or `None` when there is none to read.
///
/// The file rather than the pixels because that is what `screencapture`
/// produces, and reading it here means the caller can delete it the moment
/// this returns.
pub fn recognize(path: &Path, source_lang: &str) -> Option<String> {
    let png = std::fs::read(path).ok()?;
    recognize_bytes(&png, source_lang)
}

/// The same, for an image already in memory. Separate so the tests can reach
/// it without a screen or a permission — reading a picture is not capturing a
/// screen.
pub fn recognize_bytes(png: &[u8], source_lang: &str) -> Option<String> {
    autoreleasepool(|_| {
        let request = VNRecognizeTextRequest::new();
        // Accurate over fast: the user is already waiting on a network round
        // trip, so the saving would be invisible, while fast's misreadings
        // become mistranslations — a worse failure than a slower bubble.
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        request.setUsesLanguageCorrection(true);

        // Asked after the level is set: Vision answers for its current
        // configuration.
        match wanted_language(source_lang, languages()) {
            Some(id) => {
                let id = NSString::from_str(&id);
                request.setRecognitionLanguages(&NSArray::from_retained_slice(&[id]));
            }
            None => request.setAutomaticallyDetectsLanguage(true),
        }

        let data = NSData::with_bytes(png);
        let options: Retained<NSDictionary<_, _>> = NSDictionary::new();
        let handler = match prepared(&data) {
            Some(image) => unsafe {
                VNImageRequestHandler::initWithCIImage_options(
                    VNImageRequestHandler::alloc(),
                    &image,
                    &options,
                )
            },
            // Preparation is an improvement, not a requirement.
            None => VNImageRequestHandler::initWithData_options(
                VNImageRequestHandler::alloc(),
                &data,
                &options,
            ),
        };

        let base: Retained<VNRequest> = Retained::into_super(Retained::into_super(request.clone()));
        if let Err(e) = handler.performRequests_error(&NSArray::from_retained_slice(&[base])) {
            crate::trace!("ocr       the recogniser failed: {e}");
            return None;
        }

        let results = request.results()?;
        let mut lines = Vec::new();
        for observation in results.iter() {
            let Some(best) = observation.topCandidates(1).firstObject() else {
                continue;
            };
            let box_ = unsafe { observation.boundingBox() };
            lines.push(Line {
                y: box_.origin.y,
                x: box_.origin.x,
                height: box_.size.height,
                text: best.string().to_string(),
            });
        }
        Some(assemble(lines))
    })
}

/// Doubles the image and flattens it to high-contrast grey before the
/// recogniser sees it.
///
/// Both halves were measured, not guessed, on a poster title in hand-lettered
/// italic — the lettering Vision is worst at. Raw it read "Call You Her D
/// Name"; doubled and stretched, "Call You Her by Name", every word right. On
/// ordinary text — an interface, a paragraph, a Turkish sentence with its
/// diacritics, two columns — the output is identical either way, so the common
/// case pays nothing for it.
///
/// Doubling is what the recogniser wants: it works from a fixed grid, and a
/// small capture has too few samples per letter. Draining the colour helps
/// because a poster is designed to be looked at rather than read — yellow on
/// blue is a strong contrast to an eye and a weak one to a luminance-based
/// recogniser. Sharpening was tried as well and made it worse.
fn prepared(data: &NSData) -> Option<Retained<CIImage>> {
    /// Past this the letters blur faster than the detail grows: at three times,
    /// the same poster lost a word again.
    const SCALE: f64 = 2.0;
    /// Enough to separate ink from ground without crushing the strokes.
    const CONTRAST: f32 = 1.8;

    unsafe {
        let image = CIImage::imageWithData(data)?;
        let bigger = image.imageByApplyingTransform(CGAffineTransform {
            a: SCALE,
            b: 0.0,
            c: 0.0,
            d: SCALE,
            tx: 0.0,
            ty: 0.0,
        });
        let name = NSString::from_str("CIColorControls");
        let contrast = NSString::from_str("inputContrast");
        let saturation = NSString::from_str("inputSaturation");
        let params: Retained<NSDictionary<NSString>> = NSDictionary::from_slices::<NSString>(
            &[&contrast, &saturation],
            &[
                NSNumber::new_f32(CONTRAST).as_ref() as &AnyObject,
                NSNumber::new_f32(0.0).as_ref() as &AnyObject,
            ],
        );
        Some(bigger.imageByApplyingFilter_withInputParameters(&name, &params))
    }
}

/// A line as Vision found it: where its box sits, how tall it is, what it says.
struct Line {
    /// Bottom edge, in Vision's normalised bottom-left space.
    y: f64,
    x: f64,
    height: f64,
    text: String,
}

/// Puts recognised lines back in reading order and joins them.
///
/// Vision promises no ordering, and sorting on `y` alone is not enough: two
/// pieces of text side by side on one line get boxes whose `y` differs by a
/// hair, because a word with a descender starts lower than one without. Sorted
/// naively, a two-column page comes back with every row's right-hand side
/// first.
///
/// So lines are grouped into rows — anything within half a line-height of the
/// row belongs to it — and only then read left to right. Joined with newlines
/// rather than spaces: a hard wrap reads slightly worse unreflowed, but a wrap
/// cannot be told from a real line break, and flattening a list or an address
/// is the worse mistake.
fn assemble(mut lines: Vec<Line>) -> String {
    lines.sort_by(|a, b| b.y.partial_cmp(&a.y).unwrap_or(std::cmp::Ordering::Equal));

    let mut rows: Vec<Vec<Line>> = Vec::new();
    for line in lines {
        match rows.last_mut() {
            // The row's own height sets how far a neighbour may sit from it, so
            // a heading and the text under it are not pulled onto one line.
            Some(row) if (row[0].y - line.y).abs() < row[0].height * 0.5 => row.push(line),
            _ => rows.push(vec![line]),
        }
    }

    rows.iter_mut().for_each(|row| {
        row.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });

    rows.into_iter()
        .map(|row| {
            row.into_iter()
                .map(|l| l.text)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Which recognition language to ask for, given the configured source.
///
/// The config holds ISO-639-1 (`"en"`, `"de"`) or `"auto"`; Vision wants its
/// own identifiers (`"en-US"`, `"zh-Hans"`, `"tr-TR"`). `None` means "let
/// Vision detect it", which is also what a language it cannot read gets:
/// refusing would be worse than reading with the multilingual model and
/// letting the translator's own detection sort out the rest.
fn wanted_language(source_lang: &str, supported: &[String]) -> Option<String> {
    let wanted = source_lang.trim().to_ascii_lowercase();
    if wanted.is_empty() || wanted == "auto" {
        return None;
    }
    supported
        .iter()
        .find(|id| {
            id.split('-')
                .next()
                .is_some_and(|primary| primary.eq_ignore_ascii_case(&wanted))
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO: &[u8] = include_bytes!("../../../tests/fixtures/hello.png");
    /// Small, slanted, yellow on blue: the lettering Vision is worst at, and
    /// the reason [`prepared`] exists.
    const POSTER: &[u8] = include_bytes!("../../../tests/fixtures/poster.png");

    /// Proves the binding, the feature set, the up-cast and the result reading
    /// against the real framework — the parts a compile cannot prove. Reading
    /// an image needs no permission, so this runs on a CI runner.
    #[test]
    fn vision_reads_the_text_out_of_an_image() {
        let text = recognize_bytes(HELLO, "auto").expect("the fixture should read");
        assert!(text.contains("Hello"), "Vision read {text:?}");
        assert!(text.contains("picture"), "Vision read {text:?}");
    }

    /// Locks in the preparation. Raw, this image gives fragments; doubled and
    /// stretched to grey, every word comes back. If someone takes [`prepared`]
    /// out, this is what says so.
    #[test]
    fn a_stylised_title_is_read_whole() {
        let text = recognize_bytes(POSTER, "auto").expect("the poster should read");
        for word in ["Summer", "Nights", "Again"] {
            assert!(text.contains(word), "expected {word:?} in {text:?}");
        }
    }

    #[test]
    fn vision_says_which_languages_it_can_read() {
        let langs = languages();
        assert!(!langs.is_empty(), "Vision named no languages at all");
        assert!(
            langs.iter().any(|l| l.starts_with("en")),
            "expected English among {langs:?}"
        );
    }

    #[test]
    fn a_configured_language_becomes_visions_own_identifier() {
        let supported = [
            "en-US".to_string(),
            "de-DE".to_string(),
            "zh-Hans".to_string(),
        ];
        assert_eq!(wanted_language("de", &supported).as_deref(), Some("de-DE"));
        assert_eq!(wanted_language("zh", &supported).as_deref(), Some("zh-Hans"));
    }

    /// A language this machine cannot read falls back to detection rather than
    /// refusing. The list is passed in so this stays true whatever Apple ships.
    #[test]
    fn a_language_vision_cannot_read_falls_back_to_automatic() {
        let supported = ["en-US".to_string(), "de-DE".to_string()];
        assert_eq!(wanted_language("tr", &supported), None);
        assert_eq!(wanted_language("auto", &supported), None);
        assert_eq!(wanted_language("", &supported), None);
    }

    /// Two columns must not interleave. The `y` values differ by a hair on
    /// purpose: that is what real boxes do, and sorting on `y` alone then puts
    /// every row's right-hand side first.
    #[test]
    fn lines_come_back_in_reading_order() {
        let line = |y: f64, x: f64, text: &str| Line {
            y,
            x,
            height: 0.1,
            text: text.to_string(),
        };
        // y is a bottom-left origin, so 0.9 is the top row.
        let text = assemble(vec![
            line(0.502, 0.60, "DELTA"),
            line(0.898, 0.60, "BRAVO"),
            line(0.900, 0.10, "ALPHA"),
            line(0.500, 0.10, "CHARLIE"),
        ]);
        assert_eq!(text, "ALPHA BRAVO\nCHARLIE DELTA");
    }

    /// A row's own height decides what joins it, so a heading is not dragged
    /// onto the same line as the text beneath it.
    #[test]
    fn rows_far_apart_stay_apart() {
        let line = |y: f64, text: &str| Line {
            y,
            x: 0.1,
            height: 0.1,
            text: text.to_string(),
        };
        assert_eq!(
            assemble(vec![line(0.90, "heading"), line(0.40, "body")]),
            "heading\nbody"
        );
    }
}
