//! The worker that turns "something got selected" into "here is a bubble".
//!
//! Runs on its own thread so neither the network round trip nor the pasteboard
//! polling in [`crate::capture`] can stall the UI or the event tap.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::capture;
use crate::config::{Config, Provider};
use crate::license::{self, Licensing};
use crate::platform::{CaptureSource, Trigger};
use crate::quota::Verdict;
use crate::translate::{TranslateError, Translation, Translator};

/// How long an identical selection is treated as a repeat of the one just
/// handled rather than a fresh request.
const REPEAT_WINDOW: Duration = Duration::from_secs(2);

/// How long [`settle`] will keep deferring to a selection that will not stop
/// changing. Longer than any hand-made gesture, short enough that a client
/// stuck re-asserting its selection still produces a bubble.
const MAX_SETTLE: Duration = Duration::from_secs(5);

pub enum Request {
    /// The monitor saw a selection gesture finish.
    Selection(Trigger),
    /// A key bound in the desktop asked for the current selection to be
    /// translated.
    ///
    /// Separate from [`Request::Selection`] because it means the opposite
    /// thing about intent: a selection is a guess that the user wants a
    /// translation, and this is the user saying so. It therefore ignores the
    /// auto-translate switch — turning that off and binding a key is the way
    /// to get a translator that never interrupts, and a hotkey that went quiet
    /// with it would leave no way to ask at all.
    Hotkey(Trigger),
    /// The user changed the target language; redo the last selection.
    Retranslate,
    /// Text typed into the main window's translate box. Deliberately separate
    /// from the selection path: it must not disturb the bubble or the
    /// last-selection state the language picker retranslates.
    Manual(String),
    /// Probe every backend independently for the main window's status panel.
    TestProviders,
    /// Exchange a licence key for a signed entitlement token.
    ActivateLicense(String),
    /// Renew the token before it expires. Fired at startup when the cached one
    /// is inside its refresh window, and never surfaced to the user.
    RefreshLicense,
    /// Release this device and drop the local licence.
    DeactivateLicense,
}

pub enum UiEvent {
    /// A translation is in flight, so the bubble can anchor itself and spin.
    Working {
        /// `None` where the session will not say where the pointer is; the
        /// bubble then picks a corner instead of the cursor.
        at: Option<(f64, f64)>,
        via: CaptureSource,
    },
    /// The captured text rides along for the main window's Recent list only.
    /// The bubble shows the translation by itself.
    Done {
        source_text: String,
        result: Translation,
    },
    /// Every provider in the chain refused. Carries each one's reason.
    Failed {
        errors: Vec<(Provider, TranslateError)>,
    },
    /// Result of a main-window translate-box request.
    ManualDone(Translation),
    ManualFailed(Vec<(Provider, TranslateError)>),
    /// Per-provider health, in the order they were probed.
    ProviderStatus(Vec<(Provider, Result<String, String>)>),
    /// Today's free allowance is spent. Carries the anchor so the bubble
    /// appears exactly where the translation would have.
    Capped {
        at: Option<(f64, f64)>,
        limit: u32,
    },
    /// The same, for the main window's translate box. The user pressed a
    /// button and is owed an answer every time they press it.
    ManualCapped {
        used: u32,
        limit: u32,
    },
}

/// Phrase used to probe the backends. Short, unambiguously non-English, and
/// cheap against every provider's quota.
const PROBE_TEXT: &str = "Merhaba dünya";

pub struct Engine {
    tx: Sender<Request>,
}

impl Engine {
    pub fn start(
        config: Arc<Mutex<Config>>,
        licensing: Licensing,
        ui: Sender<UiEvent>,
        wake_ui: impl Fn() + Send + 'static,
    ) -> Self {
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("translate-engine".into())
            .spawn(move || run(rx, config, licensing, ui, wake_ui))
            .expect("failed to spawn translation engine");
        Self { tx }
    }

    pub fn sender(&self) -> Sender<Request> {
        self.tx.clone()
    }

    pub fn request(&self, request: Request) {
        let _ = self.tx.send(request);
    }
}

fn run(
    rx: Receiver<Request>,
    config: Arc<Mutex<Config>>,
    licensing: Licensing,
    ui: Sender<UiEvent>,
    wake_ui: impl Fn(),
) {
    let translator = Translator::new();
    // Separate from the translator's agent: licence calls are rare, want a
    // longer timeout, and have no business sharing a connection pool or a
    // browser user-agent with the scraped Google endpoint.
    let licence_agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .http_status_as_error(false)
            .user_agent(concat!("bubbleTranslate/", env!("CARGO_PKG_VERSION")))
            .build(),
    );
    let mut last_text = String::new();
    let mut last_at = None;
    let mut last_via = CaptureSource::PrimarySelection;
    let mut last_started = Instant::now() - REPEAT_WINDOW;

    // Holds a request that arrived mid-debounce and superseded the gesture
    // being settled; it is processed on the next turn of the loop.
    let mut pending: Option<Request> = None;

    loop {
        let mut request = match pending.take() {
            Some(request) => request,
            None => match rx.recv() {
                Ok(request) => request,
                Err(_) => return,
            },
        };

        let (cfg, debounce) = {
            let guard = config.lock().unwrap();
            (guard.clone(), Duration::from_millis(guard.debounce_ms))
        };

        // Requests that need no capture are handled here; the rest fall
        // through to the selection pipeline below.
        request = match request {
            Request::Manual(text) => {
                let entitlement = licensing.license.lock().unwrap().entitlement.clone();
                let verdict = licensing.quota.lock().unwrap().verdict(&text, &entitlement);
                let event = if let Verdict::Capped { used, limit } = verdict {
                    UiEvent::ManualCapped { used, limit }
                } else {
                    match translator.translate(&text, &cfg) {
                        Ok(result) => {
                            // Only a first reading is charged; a repeat of
                            // something already translated today is not.
                            if verdict == Verdict::Allow {
                                licensing.quota.lock().unwrap().record(&text);
                            }
                            UiEvent::ManualDone(result)
                        }
                        Err(errors) => UiEvent::ManualFailed(errors),
                    }
                };
                let _ = ui.send(event);
                wake_ui();
                continue;
            }
            Request::ActivateLicense(key) => {
                licensing.license.lock().unwrap().busy = true;
                wake_ui();

                // The lock is deliberately not held across the round trip —
                // the settings window redraws while this is in flight.
                let outcome = license::activate(&licence_agent, &key)
                    .and_then(|grant| licensing.license.lock().unwrap().accept(grant));

                let mut licence = licensing.license.lock().unwrap();
                licence.busy = false;
                match outcome {
                    Ok(()) => crate::trace!("licence   activated"),
                    Err(err) => {
                        crate::trace!("licence   refused: {err}");
                        licence.status = license::Status::Problem(err);
                    }
                }
                drop(licence);
                wake_ui();
                continue;
            }
            Request::RefreshLicense => {
                let token = licensing
                    .license
                    .lock()
                    .unwrap()
                    .token()
                    .map(str::to_string);
                if let Some(token) = token {
                    match license::refresh(&licence_agent, &token) {
                        Ok(grant) => {
                            let _ = licensing.license.lock().unwrap().accept(grant);
                            crate::trace!("licence   refreshed");
                        }
                        // Silent on purpose. A refresh that could not reach the
                        // service is our problem, not the subscriber's, and the
                        // token they already hold is good for weeks yet.
                        Err(err) => crate::trace!("licence   refresh failed: {err}"),
                    }
                    wake_ui();
                }
                continue;
            }
            Request::DeactivateLicense => {
                let token = licensing
                    .license
                    .lock()
                    .unwrap()
                    .token()
                    .map(str::to_string);
                if let Some(token) = token {
                    license::deactivate(&licence_agent, &token);
                }
                licensing.license.lock().unwrap().forget();
                wake_ui();
                continue;
            }
            Request::TestProviders => {
                let statuses = [Provider::Google, Provider::MyMemory, Provider::DeepL]
                    .into_iter()
                    .map(|provider| {
                        let outcome = translator
                            .translate_with(provider, PROBE_TEXT, &cfg)
                            .map(|t| t.text)
                            .map_err(|e| e.to_string());
                        (provider, outcome)
                    })
                    .collect();
                let _ = ui.send(UiEvent::ProviderStatus(statuses));
                wake_ui();
                continue;
            }
            other => other,
        };

        // Anything that arrives mid-debounce supersedes the gesture and is
        // requeued rather than dropped.
        if let Request::Selection(trigger) = request {
            match settle(&rx, trigger, debounce) {
                Settled::Trigger(settled) => request = Request::Selection(settled),
                Settled::Superseded(other) => {
                    pending = Some(other);
                    continue;
                }
            }
        }

        // Whether the user asked for this translation outright, which changes
        // two decisions below: the auto-translate switch does not apply to a
        // request, and neither does the repeat window — pressing the key twice
        // on the same words means "again", not "the same gesture twice".
        let asked_for = matches!(request, Request::Hotkey(_));

        // The last element is whether the allowance applies. Re-reading the
        // same selection in another language is the same translation, so
        // switching target language must never cost anything.
        let (text, at, via, metered) = match request {
            // Already handled above; the compiler cannot see that.
            Request::Manual(_)
            | Request::TestProviders
            | Request::ActivateLicense(_)
            | Request::RefreshLicense
            | Request::DeactivateLicense => continue,
            Request::Retranslate => {
                if last_text.is_empty() {
                    continue;
                }
                // Re-shown where and how the original capture was, so
                // switching languages does not move the bubble or change what
                // it says about where the text came from.
                (last_text.clone(), last_at, last_via, false)
            }
            Request::Selection(trigger) | Request::Hotkey(trigger) => {
                // Applied here rather than in the tap callback, which must not
                // touch the config mutex. A hotkey is exempt: it is a request,
                // not a guess.
                if !cfg.auto_translate && !asked_for {
                    crate::trace!("skip      auto-translate is off");
                    continue;
                }
                let Some(capture) =
                    capture::selected_text(cfg.clipboard_fallback, trigger.clipboard_before)
                else {
                    crate::trace!("capture   nothing (AX empty, clipboard produced nothing)");
                    continue;
                };
                let text = capture.text;
                let chars = text.chars().count();
                crate::trace!(
                    "capture   via={:?} chars={chars} text={:?}",
                    capture.via,
                    truncate_for_log(&text),
                );
                if chars < cfg.min_chars || chars > cfg.max_chars {
                    crate::trace!(
                        "skip      {chars} chars outside [{}, {}]",
                        cfg.min_chars,
                        cfg.max_chars,
                    );
                    continue;
                }
                // Guards against the same selection being read twice in quick
                // succession (a click that lands inside an existing selection
                // re-reads it). Selecting the same words again later is a
                // deliberate act and does go through.
                if !asked_for && text == last_text && last_started.elapsed() < REPEAT_WINDOW {
                    crate::trace!("skip      same text within repeat window");
                    continue;
                }
                (text, trigger.at, capture.via, true)
            }
        };

        // The meter, and the only place a translation is refused for a reason
        // that is not technical. It sits after the capture so a selection too
        // short to translate never costs anything, and before the request so
        // nothing is spent on a provider that may fail anyway.
        let mut chargeable = false;
        if metered {
            let entitlement = licensing.license.lock().unwrap().entitlement.clone();
            let verdict = licensing.quota.lock().unwrap().verdict(&text, &entitlement);
            crate::trace!("quota     {verdict}");
            match gate(verdict, at) {
                Gate::Translate { charge } => chargeable = charge,
                Gate::Refuse(event) => {
                    let _ = ui.send(event);
                    wake_ui();
                    continue;
                }
            }
        }

        last_started = Instant::now();

        last_text = text.clone();
        last_at = at;
        last_via = via;

        let _ = ui.send(UiEvent::Working { at, via });
        wake_ui();

        let event = match translator.translate(&text, &cfg) {
            Ok(result) => {
                // Counted here rather than above: a translation nobody
                // received is not one the user should have paid for.
                if chargeable {
                    licensing.quota.lock().unwrap().record(&text);
                }
                crate::trace!(
                    "translate [{}] {} -> {:?}",
                    result.provider.label(),
                    result.source_lang,
                    truncate_for_log(&result.text),
                );
                UiEvent::Done {
                    source_text: text,
                    result,
                }
            }
            Err(errors) => {
                for (provider, err) in &errors {
                    crate::trace!("translate {} FAILED: {err}", provider.label());
                }
                UiEvent::Failed { errors }
            }
        };
        let _ = ui.send(event);
        wake_ui();
    }
}

fn truncate_for_log(text: &str) -> String {
    text.chars().take(48).collect()
}

enum Settled {
    /// The debounce window closed; this was the last gesture in it.
    Trigger(Trigger),
    /// A non-gesture request arrived mid-window and takes priority.
    Superseded(Request),
}

/// Waits for the selection to stop changing, returning the last trigger seen.
///
/// The window slides: every trigger that arrives restarts it, so what is
/// waited for is a moment of quiet *after* the gesture rather than a fixed
/// delay after its first sign. That is the difference between a bubble that
/// waits for the sentence and one that pops up over the second word of it.
///
/// It matters because a trigger does not mean the same thing on every system.
/// On macOS the tap already only fires on a completed gesture, so the window
/// closes on the first timeout and nothing here changes. On Linux a trigger is
/// "the selection changed", and dragging a sentence out emits one per word as
/// it grows — there, the quiet is the only sign the user has let go.
///
/// Capped, because sliding forever would let a client that re-asserts its
/// selection on a timer hold the bubble off for good. Past the cap the newest
/// trigger is taken as the answer.
fn settle(rx: &Receiver<Request>, mut latest: Trigger, window: Duration) -> Settled {
    let started = Instant::now();
    let mut deadline = started + window;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Settled::Trigger(latest);
        }
        match rx.recv_timeout(remaining) {
            Ok(Request::Selection(next)) => {
                latest = next;
                if started.elapsed() >= MAX_SETTLE {
                    crate::trace!("settle    still changing after {MAX_SETTLE:?}; taking it");
                    return Settled::Trigger(latest);
                }
                deadline = Instant::now() + window;
            }
            Ok(other) => return Settled::Superseded(other),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                return Settled::Trigger(latest);
            }
        }
    }
}

/// What a meter verdict means for the selection that produced it.
///
/// Pulled out of the loop for one reason: it is the only decision in there
/// that can be tested. Everything around it needs a screen to capture from.
enum Gate {
    /// Translate it. `charge` is false for a repeat of text already read,
    /// which is free.
    Translate { charge: bool },
    /// Do not translate it, and show this instead. Deliberately not an
    /// `Option`: a gesture that produces nothing at all is what a broken app
    /// looks like from the outside, so a refusal always says so.
    Refuse(UiEvent),
}

fn gate(verdict: Verdict, at: Option<(f64, f64)>) -> Gate {
    match verdict {
        Verdict::Allow => Gate::Translate { charge: true },
        Verdict::Repeat => Gate::Translate { charge: false },
        Verdict::Capped { limit, .. } => Gate::Refuse(UiEvent::Capped { at, limit }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(x: f64) -> Request {
        Request::Selection(Trigger {
            at: Some((x, 0.0)),
            clipboard_before: None,
        })
    }

    /// The regression this seam exists for: a spent allowance still answers.
    ///
    /// This decision once read a throttled upgrade prompt as permission to
    /// send nothing at all, so for 59 minutes out of every 60, selecting text
    /// did visibly nothing. That is indistinguishable from a broken app and is
    /// reported as one. The throttle is gone, but the seam stays: a refusal
    /// has to reach the screen, at the place the translation would have been.
    #[test]
    fn a_spent_allowance_still_answers_the_gesture() {
        let spent = Verdict::Capped {
            used: 10,
            limit: 10,
        };
        match gate(spent, Some((120.0, 340.0))) {
            Gate::Refuse(UiEvent::Capped { at, limit }) => {
                // Where the translation would have appeared, not wherever the
                // bubble last happened to sit.
                assert_eq!(at, Some((120.0, 340.0)));
                assert_eq!(limit, 10);
            }
            _ => panic!("a spent allowance produced no bubble at all"),
        }
    }

    /// A first reading is charged, a re-reading is not, and both translate.
    #[test]
    fn only_a_first_reading_is_charged() {
        assert!(matches!(
            gate(Verdict::Allow, None),
            Gate::Translate { charge: true },
        ));
        assert!(matches!(
            gate(Verdict::Repeat, None),
            Gate::Translate { charge: false },
        ));
    }

    /// A selection that keeps growing must not settle while it is growing.
    ///
    /// This is the bubble appearing mid-sweep, expressed as a test: five
    /// changes 40ms apart, against a 100ms window that a fixed deadline would
    /// have let expire after the first two.
    #[test]
    fn a_growing_selection_settles_only_once_it_stops() {
        let (tx, rx) = channel();
        let started = Instant::now();
        std::thread::spawn(move || {
            for step in 1..=5 {
                std::thread::sleep(Duration::from_millis(40));
                let _ = tx.send(selection(f64::from(step)));
            }
            // Held so the channel does not disconnect and end the wait early.
            std::thread::sleep(Duration::from_millis(500));
        });

        let settled = settle(
            &rx,
            Trigger {
                at: Some((0.0, 0.0)),
                clipboard_before: None,
            },
            Duration::from_millis(100),
        );
        let elapsed = started.elapsed();

        match settled {
            // The last change, not the first: what the user ended up selecting.
            Settled::Trigger(trigger) => assert_eq!(trigger.at, Some((5.0, 0.0))),
            Settled::Superseded(_) => panic!("nothing should have superseded the gesture"),
        }
        assert!(
            elapsed >= Duration::from_millis(300),
            "settled after {elapsed:?}, i.e. while the selection was still changing",
        );
    }

    /// The window still closes when nothing more arrives.
    #[test]
    fn a_finished_selection_settles_after_the_window() {
        let (tx, rx) = channel();
        let started = Instant::now();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(tx);
        });

        let settled = settle(
            &rx,
            Trigger {
                at: Some((7.0, 0.0)),
                clipboard_before: None,
            },
            Duration::from_millis(50),
        );
        assert!(started.elapsed() < Duration::from_millis(250));
        assert!(matches!(settled, Settled::Trigger(t) if t.at == Some((7.0, 0.0))));
    }

    /// Another kind of request mid-window takes over rather than being lost.
    #[test]
    fn another_request_supersedes_the_gesture() {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            let _ = tx.send(Request::Retranslate);
            std::thread::sleep(Duration::from_millis(200));
        });

        let settled = settle(
            &rx,
            Trigger {
                at: None,
                clipboard_before: None,
            },
            Duration::from_millis(100),
        );
        assert!(matches!(settled, Settled::Superseded(Request::Retranslate)));
    }
}
