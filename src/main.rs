//! bubbleTranslate — a bubble translator for macOS and Linux.
//!
//! Select text in any app that lets you select text — a PDF, a terminal, a
//! browser, a code editor — and a small bubble appears at the cursor with the
//! translation. Three backends are tried in order (Google, then MyMemory, then
//! DeepL) so a throttled or unconfigured one falls through to the next.
//!
//! Threads:
//!   main      — the egui bubble viewport
//!   monitor   — watches for finished selections, however this system reports
//!               them; see [`platform`]
//!   engine    — capture + HTTP, kept off the UI thread

mod config;
mod engine;
mod main_window;
mod platform;
mod trace;
mod translate;
mod ui;

/// The platform boundary, re-exported so the rest of the crate can say
/// `capture::` and `shell::` without caring which implementation it got.
pub(crate) use platform::{capture, monitor, shell};

use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

use eframe::egui;

use crate::config::Config;
use crate::engine::{Engine, Request};
use crate::main_window::MainState;
use crate::ui::{BUBBLE_WIDTH, BubbleApp};

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(pos) = args.iter().position(|a| a == "--translate") {
        let text = args[pos + 1..].join(" ");
        std::process::exit(translate_once(&text));
    }
    if args.iter().any(|a| a == "--check") {
        std::process::exit(check_providers());
    }

    let config = Arc::new(Mutex::new(Config::load()));

    // Whether to come up with no interface at all. The flag is for autostart
    // entries and for trying it once without committing; the setting is for
    // making it the habit. Either is enough — this is the kind of switch a
    // desktop file should be able to set without the config agreeing.
    let background =
        args.iter().any(|a| a == "--background") || config.lock().unwrap().start_in_background;

    // Asked up front because the answer shapes the whole session: on macOS
    // this is what pops the permission dialog, at most once, and on Linux it
    // is where the compositor's protocols are probed.
    let readiness = capture::readiness();
    if !readiness.ok {
        eprintln!("bubbleTranslate: {}", readiness.detail);
    }
    let warning = (!readiness.ok).then(|| readiness.summary.clone());

    let (ui_tx, ui_rx) = channel();

    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([BUBBLE_WIDTH, 120.0])
        .with_min_inner_size([BUBBLE_WIDTH, 60.0])
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_always_on_top()
        // Start hidden: the bubble only exists once there is something to say.
        .with_visible(false)
        // Never take focus. The app being read must stay frontmost, or its
        // selection would be dropped the moment we appeared.
        .with_active(false)
        .with_taskbar(false)
        // The X11 `WM_CLASS` and the Wayland app id, so a window manager can
        // be told about the bubble by name.
        .with_app_id("bubbleTranslate");

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "bubbleTranslate",
        options,
        Box::new(move |cc| {
            shell::run_in_background();

            let engine = Engine::start(config.clone(), ui_tx, {
                let ctx = cc.egui_ctx.clone();
                move || ctx.request_repaint()
            });

            let main = Arc::new(Mutex::new(MainState::new(
                engine.sender(),
                readiness,
                !background,
            )));

            // On macOS this is the status item, and it is what makes closing
            // the main window safe: with no Dock icon it is the only way back
            // to the window, and the only Quit affordance. On Linux there is
            // nothing yet, which is why closing the window quits instead.
            shell::install(cc.egui_ctx.clone());

            let requests = engine.sender();
            if let Err(err) = monitor::spawn(move |trigger| {
                // Nothing here may block. This runs inside the event tap
                // callback, and macOS disables a tap that dwells too long — so
                // reading the auto-translate switch here, behind a mutex the UI
                // holds while it draws, is enough to kill the tap. The engine
                // applies the switch instead; it can afford to wait for it.
                let _ = requests.send(Request::Selection(trigger));
            }) {
                eprintln!("bubbleTranslate: could not start the selection monitor: {err}");
            }

            Ok(Box::new(BubbleApp::new(
                cc, config, engine, ui_rx, warning, main, background,
            )))
        }),
    )
}

/// `bubbleTranslate --translate <text>`: runs the provider chain once and prints the
/// result, without opening a window or needing any permissions. This is how to
/// tell a backend problem apart from a capture problem.
fn translate_once(text: &str) -> i32 {
    if text.trim().is_empty() {
        eprintln!("usage: bubbleTranslate --translate <text>");
        return 2;
    }
    let cfg = Config::load();
    println!(
        "chain: {}  →  {}",
        cfg.active_providers()
            .iter()
            .map(|p| p.label())
            .collect::<Vec<_>>()
            .join(", "),
        cfg.target_lang,
    );
    match translate::Translator::new().translate(text, &cfg) {
        Ok(result) => {
            println!(
                "[{}] {} → {}",
                result.provider.label(),
                result.source_lang,
                cfg.target_lang
            );
            println!("{}", result.text);
            0
        }
        Err(errors) => {
            eprintln!("every provider failed:");
            for (provider, err) in errors {
                eprintln!("  {}: {err}", provider.label());
            }
            1
        }
    }
}

/// `bubbleTranslate --check`: probes each backend independently with a fixed phrase
/// and reports which ones answer. Exits non-zero if none do.
fn check_providers() -> i32 {
    const PROBE: &str = "Merhaba dünya";
    let cfg = Config::load();
    let translator = translate::Translator::new();
    let mut healthy = 0;

    println!("probe: \"{PROBE}\" → {}\n", cfg.target_lang);
    for provider in [
        config::Provider::Google,
        config::Provider::MyMemory,
        config::Provider::DeepL,
    ] {
        // "FAILED" is reserved for a provider the chain would actually have
        // used; one that is off or unconfigured is merely skipped.
        let in_chain = cfg.active_providers().contains(&provider);
        let position = cfg
            .providers
            .iter()
            .position(|p| *p == provider)
            .map(|i| format!("#{}", i + 1))
            .unwrap_or_else(|| "off".to_string());

        match translator.translate_with(provider, PROBE, &cfg) {
            Ok(result) => {
                healthy += 1;
                println!(
                    "  {:<9} {:<4} ok      {} → {}",
                    provider.label(),
                    position,
                    result.source_lang,
                    result.text
                );
            }
            Err(err) => println!(
                "  {:<9} {:<4} {}  {err}",
                provider.label(),
                position,
                if in_chain { "FAILED " } else { "skipped" },
            ),
        }
    }

    println!("\n{healthy}/3 providers responding");
    if healthy == 0 { 1 } else { 0 }
}
