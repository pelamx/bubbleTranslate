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
mod license;
mod main_window;
mod platform;
mod quota;
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
use crate::license::{License, Licensing};
use crate::main_window::MainState;
use crate::quota::Quota;
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
    if args.iter().any(|a| a == "--license") {
        std::process::exit(license_status());
    }
    #[cfg(debug_assertions)]
    if let Some(pos) = args.iter().position(|a| a == "--dev-license") {
        std::process::exit(mint_dev_license(args.get(pos + 1).map(String::as_str)));
    }
    #[cfg(debug_assertions)]
    if args.iter().any(|a| a == "--dev-reset-trial") {
        std::process::exit(reset_trial());
    }

    // Sampled before the config is loaded, because loading it creates the
    // file when it is missing — and whether it existed a moment ago is the
    // only way to tell an install that predates metering from a new one. That
    // install keeps unlimited use; see `quota::Counter::legacy_unlimited`.
    let config_existed = Config::path().exists();

    let config = Arc::new(Mutex::new(Config::load()));
    let licensing = Licensing {
        license: Arc::new(Mutex::new(License::load())),
        quota: Arc::new(Mutex::new(Quota::load(config_existed))),
    };

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

            let engine = Engine::start(config.clone(), licensing.clone(), ui_tx, {
                let ctx = cc.egui_ctx.clone();
                move || ctx.request_repaint()
            });

            // Renew a token that is nearing its expiry. Off the UI thread, and
            // silent either way: a subscriber whose refresh fails today still
            // has weeks of validity in the token they are holding.
            if licensing.license.lock().unwrap().wants_refresh() {
                engine.request(Request::RefreshLicense);
            }

            // Feed the monitor's triggers into the engine, honouring the
            // auto-translate switch without tearing the tap down.
            let main = Arc::new(Mutex::new(MainState::new(
                engine.sender(),
                readiness,
                !background,
                config.lock().unwrap().license_key.clone(),
            )));

            // On macOS this is the status item, and it is what makes closing
            // the main window safe: with no Dock icon it is the only way back
            // to the window, and the only Quit affordance. On Linux there is
            // nothing yet, which is why closing the window quits instead.
            shell::install(cc.egui_ctx.clone());

            let requests = engine.sender();
            let monitor_config = config.clone();
            if let Err(err) = monitor::spawn(move |trigger| {
                if monitor_config.lock().unwrap().auto_translate {
                    let _ = requests.send(Request::Selection(trigger));
                }
            }) {
                eprintln!("bubbleTranslate: could not start the selection monitor: {err}");
            }

            Ok(Box::new(BubbleApp::new(
                cc, config, engine, ui_rx, warning, main, licensing, background,
            )))
        }),
    )
}

/// `bubbleTranslate --license`: says what this install is entitled to and how
/// much of the free trial is left, without opening a window.
///
/// The counterpart to `--check`. That one separates "the app is broken" from
/// "the network is"; this one separates either from "the trial is spent",
/// which otherwise looks identical from the outside — no bubble appears.
fn license_status() -> i32 {
    let config_existed = Config::path().exists();
    let licence = License::load();
    let quota = Quota::load(config_existed);

    println!("device    {}", license::device_id());
    println!(
        "plan      {}{}",
        licence.entitlement.plan.label(),
        match licence.entitlement.cycle {
            Some(cycle) => format!(" ({})", cycle.label()),
            None => String::new(),
        },
    );

    match quota.limit(&licence.entitlement) {
        None => {
            println!(
                "allowance unlimited{}",
                if quota.is_grandfathered() {
                    " (install predates the free trial)"
                } else {
                    ""
                },
            );
            println!("used      {} translated", quota.used());
        }
        Some(limit) => {
            println!("allowance {limit} free translations, once");
            println!(
                "used      {} of {limit}, {} left",
                quota.used(),
                quota.remaining(&licence.entitlement).unwrap_or(0),
            );
            println!("pro       {} or {}", license::PRICE_MONTHLY, license::PRICE_YEARLY);
        }
    }

    match &licence.status {
        license::Status::None => println!("licence   none entered"),
        license::Status::Active { renews } => {
            let days = licence.entitlement.exp.saturating_sub(license::now()) / 86_400;
            println!("licence   active, revalidates in {days} days");
            if let Some(renews) = renews {
                println!("renews    {renews}");
            }
        }
        license::Status::Lapsed => println!("licence   expired"),
        license::Status::Problem(why) => println!("licence   {why}"),
    }

    0
}

/// `bubbleTranslate --dev-reset-trial`: hands the ten free translations back.
///
/// A daily allowance tested itself: spend it, wait for midnight, and the free
/// path was available again. A trial that never resets has no such tomorrow —
/// without this, working on the free tier means editing `usage.json` by hand
/// after the tenth translation, and the free tier is the path most users see.
/// Debug builds only: in a release this would be the whole paywall.
#[cfg(debug_assertions)]
fn reset_trial() -> i32 {
    let config_existed = Config::path().exists();
    let mut quota = Quota::load(config_existed);
    if quota.is_grandfathered() {
        println!("This install predates the trial and is already unlimited.");
        return 0;
    }
    quota.reset_trial();
    println!(
        "Trial reset: {} free translations again.",
        license::FREE_TRIAL_TRANSLATIONS,
    );
    println!("  counter     {}", quota::path().display());
    0
}

/// `bubbleTranslate --dev-license [pro|free]`: signs a licence for this machine
/// with a locally generated key and installs it.
///
/// The whole point of P1 being buildable before the licence service is: this
/// exercises the real verification path, the real device binding and the real
/// cache file, against a key that only this machine has. Debug builds only.
#[cfg(debug_assertions)]
fn mint_dev_license(plan: Option<&str>) -> i32 {
    let plan = plan.unwrap_or("pro");
    if plan != "pro" && plan != "free" {
        eprintln!("usage: bubbleTranslate --dev-license [pro|free]");
        return 2;
    }
    let public_key = license::dev::mint(plan, 30);
    println!("Signed a 30-day {plan} licence for this device.");
    println!("  device      {}", license::device_id());
    println!("  licence     {}", license::path().display());
    println!("\nRun the app with the matching key, or it will not verify:\n");
    println!("  BUBBLETRANSLATE_LICENSE_PUBKEY={public_key} cargo run\n");
    0
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
