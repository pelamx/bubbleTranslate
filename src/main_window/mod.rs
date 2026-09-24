//! The main window: settings, a scratch translate box, and backend health.
//!
//! Runs as a deferred viewport off the bubble's root, so closing it leaves the
//! translator running in the background — the bubble does not depend on this
//! window existing.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use eframe::egui;

use crate::config::{
    BubbleTheme, Config, FeedbackVia, LANGUAGES, Provider, TriggerKey, language_name,
};
use crate::engine::Request;
use crate::i18n::{self, UiLang, t};
use crate::license::{self, Licensing, Status};
use crate::platform::Readiness;
use crate::translate::{TranslateError, Translation};

mod account;
mod feedback;
mod screen;
mod settings;

use account::*;
use feedback::*;
use settings::*;

/// Narrow and tall: the panels are a single column of short rows, so height
/// is what keeps the whole thing visible without scrolling, and extra width
/// would only stretch the labels. Clamped to the display at runtime — see
/// `place_window`.
pub const WINDOW_SIZE: [f32; 2] = [470.0, 880.0];

/// Space left around the window when it has to be shrunk to fit the display.
/// The top allowance covers the menu bar, the bottom the Dock.
const SCREEN_MARGIN_X: f32 = 40.0;
const SCREEN_MARGIN_Y: f32 = 120.0;

/// Cap on the activity list. It is a convenience for re-reading a translation
/// that has already scrolled away, not a persistent history.
const MAX_RECENT: usize = 25;

/// The window's own background, matching the bubble's panel so the two read as
/// one application. Two shades of it: the window is lit from the top, which
/// gives a tall column of sections somewhere to start and somewhere to end.
const WINDOW_BG: egui::Color32 = egui::Color32::from_rgb(30, 31, 34);
const WINDOW_BG_TOP: egui::Color32 = egui::Color32::from_rgb(40, 42, 48);
const TEXT_PRIMARY: egui::Color32 = egui::Color32::from_gray(240);
const TEXT_SECONDARY: egui::Color32 = egui::Color32::from_gray(186);
const TEXT_MUTED: egui::Color32 = egui::Color32::from_gray(155);
const OK_GREEN: egui::Color32 = egui::Color32::from_rgb(120, 210, 140);
const WARN_AMBER: egui::Color32 = egui::Color32::from_rgb(245, 195, 130);
const ERR_RED: egui::Color32 = egui::Color32::from_rgb(255, 150, 150);

pub struct MainState {
    /// Whether the window should currently exist.
    pub open: bool,
    /// Cleared whenever the window is (re)opened so its size is reasserted;
    /// see the note in `draw`.
    pub sized: bool,
    /// Set when the user asks for the window while it is already open, so it
    /// gets raised instead of quietly staying behind other apps.
    pub focus_requested: bool,
    pub requests: Sender<Request>,
    /// Whether selections can be watched at all here, and why not when they
    /// cannot. The reason is platform-specific, so it travels with the verdict
    /// rather than being written into this window.
    pub readiness: Readiness,

    // Scratch translate box.
    pub input: String,
    pub translating: bool,
    pub result: Option<Result<Translation, Vec<(Provider, TranslateError)>>>,

    // Backend health panel.
    pub testing: bool,
    pub statuses: Vec<(Provider, Result<String, String>)>,

    /// What the bubble has translated this session, newest first.
    pub recent: Vec<RecentEntry>,

    // Account.
    /// Set when the translate box was refused for want of today's allowance,
    /// as (used, limit). Cleared on the next attempt.
    pub capped: Option<(u32, u32)>,
    /// The licence key being typed. Separate from the saved one in the config
    /// so a half-typed key is never written to disk, but seeded from it at
    /// startup: a key whose activation could not reach the service is still
    /// the user's key, and it should be waiting for them to try again rather
    /// than sending them back to the email it came in.
    pub key_input: String,

    // Feedback.
    /// What the user is writing to send in. Kept here rather than in the
    /// config: a half-written complaint is not a setting, and it should not
    /// survive on disk.
    pub feedback: String,
    /// What happened to the last attempt to send, shown under the box.
    pub feedback_note: Option<String>,
}

pub struct RecentEntry {
    pub source: String,
    pub translated: String,
    pub provider: Provider,
}

impl MainState {
    pub fn new(
        requests: Sender<Request>,
        readiness: Readiness,
        open: bool,
        license_key: String,
    ) -> Self {
        Self {
            open,
            sized: false,
            focus_requested: false,
            requests,
            readiness,
            input: String::new(),
            translating: false,
            result: None,
            testing: false,
            statuses: Vec::new(),
            recent: Vec::new(),
            capped: None,
            key_input: license_key,
            feedback: String::new(),
            feedback_note: None,
        }
    }

    pub fn push_recent(&mut self, source: String, translated: String, provider: Provider) {
        self.recent.insert(
            0,
            RecentEntry {
                source,
                translated,
                provider,
            },
        );
        self.recent.truncate(MAX_RECENT);
    }
}

pub fn draw(
    ui: &mut egui::Ui,
    state: &Arc<Mutex<MainState>>,
    config: &Arc<Mutex<Config>>,
    licensing: &Licensing,
) {
    let mut state = state.lock().unwrap();
    let mut cfg = config.lock().unwrap();
    let mut dirty = false;

    if !state.sized {
        place_window(ui.ctx());
        state.sized = true;
    }

    // Paint the background before anything else.
    //
    // The bubble's viewport is transparent so its rounded corners sit on the
    // desktop rather than on a grey rectangle, and that clear colour belongs
    // to the renderer, not to one window — every viewport the app owns starts
    // fully transparent. On macOS the window server puts an opaque window
    // behind this one anyway; elsewhere nothing does, and the desktop shows
    // through everywhere a widget has not painted.
    let window_rect = ui
        .ctx()
        .input(|i| i.raw.screen_rect)
        .unwrap_or_else(|| ui.max_rect());
    paint_background(ui, window_rect);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(4.0);
            header(ui, &state, &mut cfg, &mut dirty);
            ui.add_space(10.0);
            update_banner(ui);

            section(ui, t("Translate", "Çevir", "Traducir"), |ui| {
                translate_box(ui, &mut state, &cfg)
            });
            section(
                ui,
                t(
                    "Read the screen",
                    "Ekrandaki yazıyı oku",
                    "Leer la pantalla",
                ),
                |ui| screen::screen_reading(ui, &mut state, &cfg),
            );
            section(ui, t("Languages", "Diller", "Idiomas"), |ui| {
                dirty |= languages(ui, &mut state, &mut cfg);
            });
            section(ui, t("Providers", "Servisler", "Proveedores"), |ui| {
                dirty |= providers(ui, &mut state, &mut cfg);
            });
            section(ui, t("Account", "Hesap", "Cuenta"), |ui| {
                dirty |= account(ui, &mut state, &mut cfg, licensing);
            });
            section(ui, t("Behaviour", "Davranış", "Comportamiento"), |ui| {
                dirty |= behaviour(ui, &mut cfg);
            });
            section(
                ui,
                t(
                    "Send feedback",
                    "Geri bildirim gönder",
                    "Enviar comentarios",
                ),
                |ui| {
                    dirty |= feedback(ui, &mut state, &mut cfg);
                },
            );
            if !state.recent.is_empty() {
                section(ui, t("Recent", "Son çeviriler", "Recientes"), |ui| {
                    recent(ui, &state)
                });
            }

            ui.add_space(8.0);
            footer(ui);
            ui.add_space(8.0);
        });

    if dirty {
        let _ = cfg.save();
    }
}

/// Sizes and positions the window on the first frame after it opens.
///
/// Two reasons this cannot be left to the viewport builder. A deferred
/// viewport ignores the builder's `inner_size` and comes up at its minimum
/// size instead. And the default placement takes no account of the display, so
/// the preferred height can put the lower part of the window past the bottom
/// edge — which silently breaks every dropdown, because a popup opens downward
/// into the off-screen remainder of the window and is simply never seen.
///
/// So: clamp to the display, then centre.
fn place_window(ctx: &egui::Context) {
    let monitor = ctx.input(|i| i.viewport().monitor_size);

    let Some(monitor) = monitor else {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(WINDOW_SIZE.into()));
        return;
    };

    let width = WINDOW_SIZE[0].min(monitor.x - SCREEN_MARGIN_X);
    let height = WINDOW_SIZE[1].min(monitor.y - SCREEN_MARGIN_Y);
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(width, height)));

    let x = ((monitor.x - width) / 2.0).max(SCREEN_MARGIN_X / 2.0);
    let y = ((monitor.y - height) / 2.0).max(SCREEN_MARGIN_Y / 4.0);
    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(x, y)));
}

/// Fills the window with a vertical gradient, lightest at the top.
///
/// A flat fill is what this was, and against it the section cards — barely
/// eight shades lighter — read as a single slab of grey. The gradient gives
/// the window a direction: the cards near the top sit *in* their background,
/// the ones at the bottom sit on it, and the eye gets a horizon to judge them
/// against.
///
/// Painted as two triangles rather than with a shader, which is all egui needs
/// to interpolate a colour across a rectangle.
fn paint_background(ui: &egui::Ui, rect: egui::Rect) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), WINDOW_BG_TOP);
    mesh.colored_vertex(rect.right_top(), WINDOW_BG_TOP);
    mesh.colored_vertex(rect.left_bottom(), WINDOW_BG);
    mesh.colored_vertex(rect.right_bottom(), WINDOW_BG);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(2, 1, 3);
    ui.ctx()
        .layer_painter(egui::LayerId::background())
        .add(egui::Shape::mesh(mesh));
}

fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.label(
        egui::RichText::new(title)
            .size(12.0)
            .color(TEXT_MUTED)
            .strong(),
    );
    ui.add_space(4.0);
    egui::Frame::new()
        .fill(egui::Color32::from_rgb(38, 39, 43))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui);
        });
    ui.add_space(14.0);
}

/// Where the settings file lives, and what closing this window means.
///
/// Worth spelling out, because closing it does not do what closing a window
/// usually does. Wherever the app has an indicator — the menu bar on macOS, a
/// tray icon on Linux — closing puts the interface away and the translator
/// carries on watching selections, and quitting is deliberately somewhere
/// else: the indicator's own menu. A window that can be dismissed by reflex
/// should not also be the thing that stops the app.
fn footer(ui: &mut egui::Ui) {
    ui.label(
        egui::RichText::new(format!(
            "{} {}",
            t("Settings file:", "Ayar dosyası:", "Archivo de ajustes:"),
            Config::path().display()
        ))
        .size(10.5)
        .color(TEXT_MUTED),
    );

    if crate::shell::has_indicator() {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(t(
                "Closing this window leaves the translator running in the background. \
                     To stop it, use Quit in the tray icon's menu.",
                "Bu pencereyi kapatmak çevirmeni arka planda çalışır bırakır. \
                     Durdurmak için tepsi simgesinin menüsündeki Quit'i kullan.",
                "Cerrar esta ventana deja el traductor funcionando en segundo plano. \
                     Para detenerlo, usa Quit en el menú del icono de la bandeja.",
            ))
            .size(10.5)
            .color(TEXT_MUTED),
        );
    }
}

/// A newer build is published: say which, and open its download. Nothing is
/// installed from here; the user replaces the app the way they installed it.
fn update_banner(ui: &mut egui::Ui) {
    let Some(newer) = crate::update::available() else {
        return;
    };
    egui::Frame::new()
        .fill(egui::Color32::from_rgb(44, 58, 48))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(format!(
                    "{} {}",
                    t(
                        "A new version is available:",
                        "Yeni sürüm var:",
                        "Hay una versión nueva:"
                    ),
                    newer.version
                ))
                .size(13.0)
                .color(OK_GREEN)
                .strong(),
            );
            ui.label(
                egui::RichText::new(format!(
                    "{} {}. {}",
                    t("You have", "Sende olan:", "Tienes la"),
                    env!("CARGO_PKG_VERSION"),
                    t(
                        "Your licence and settings stay as they are.",
                        "Lisansın ve ayarların olduğu gibi kalır.",
                        "Tu licencia y tus ajustes se mantienen.",
                    ),
                ))
                .size(12.0)
                .color(TEXT_SECONDARY),
            );
            ui.add_space(4.0);
            if ui.button(t("Download", "İndir", "Descargar")).clicked() {
                crate::shell::open_url(&newer.url);
            }
        });
    ui.add_space(14.0);
}

fn header(ui: &mut egui::Ui, state: &MainState, cfg: &mut Config, dirty: &mut bool) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("bubbleTranslate")
                .size(21.0)
                .color(TEXT_PRIMARY)
                .strong(),
        );
        // The running version, always on show. It is the first thing a bug
        // report needs and the thing the update banner below is talking
        // about, so it is worth a permanent corner of the window rather than
        // only appearing once a newer build exists.
        ui.label(
            egui::RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                .size(11.5)
                .color(TEXT_MUTED),
        );
        ui.label(
            egui::RichText::new("by pelamx")
                .size(11.5)
                .color(TEXT_MUTED),
        );
        // The interface language, one click each, drawn the way the website
        // draws it: a rounded strip, the active choice lifted onto a card with
        // a short blue underline.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(4.0);
            egui::Frame::new()
                .fill(egui::Color32::from_white_alpha(10))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(70)))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::same(3))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    // Right to left, inherited, so the list is walked backwards.
                    for lang in UiLang::ALL.iter().rev() {
                        let on = cfg.ui_lang == *lang;
                        let text = egui::RichText::new(lang.code())
                            .size(11.5)
                            .strong()
                            .color(if on { TEXT_PRIMARY } else { TEXT_MUTED });
                        let button = egui::Button::new(text)
                            .corner_radius(7.0)
                            .stroke(egui::Stroke::NONE)
                            .min_size(egui::vec2(32.0, 22.0))
                            .fill(if on {
                                egui::Color32::from_rgb(56, 58, 66)
                            } else {
                                egui::Color32::TRANSPARENT
                            });
                        let response = ui.add(button);
                        if on {
                            let r = response.rect;
                            let y = r.bottom() - 3.0;
                            ui.painter().line_segment(
                                [
                                    egui::pos2(r.center().x - 7.0, y),
                                    egui::pos2(r.center().x + 7.0, y),
                                ],
                                egui::Stroke::new(2.0, egui::Color32::from_rgb(79, 140, 255)),
                            );
                        }
                        if response.clicked() && !on {
                            cfg.ui_lang = *lang;
                            i18n::set(*lang);
                            *dirty = true;
                        }
                    }
                });
        });
    });
    ui.add_space(2.0);

    if state.readiness.ok {
        ui.label(
            egui::RichText::new(t(
                "● Watching for selections",
                "● Seçimler izleniyor",
                "● Vigilando selecciones",
            ))
            .size(12.5)
            .color(OK_GREEN),
        );
        // Name the key the selection has to be made with. Without it this
        // line describes a gesture that does nothing: a selection made with
        // no key held is ignored, and the user is left watching the green dot
        // wondering why. There is nothing to name when they chose no key —
        // and nothing to promise when the session will not report one, which
        // the Behaviour section explains rather than this line.
        let hold = (cfg.trigger_key != TriggerKey::Always
            && crate::monitor::trigger_key_blocked().is_none())
        .then(|| cfg.trigger_key.label());
        ui.label(
            egui::RichText::new(match hold {
                Some(key) => match i18n::lang() {
                    UiLang::En => format!(
                        "Hold {key} and select text in any app — double-click a word, \
                         drag a phrase, or triple-click a line."
                    ),
                    UiLang::Tr => format!(
                        "{key} tuşunu basılı tutup herhangi bir uygulamada metin seç — \
                         kelimeye çift tıkla, ifadeyi sürükle ya da satıra üç kez tıkla."
                    ),
                    UiLang::Es => format!(
                        "Mantén {key} y selecciona texto en cualquier app — doble clic en \
                         una palabra, arrastra una frase o triple clic en una línea."
                    ),
                },
                None => t(
                    "Select text in any app — double-click a word, drag a phrase, \
                     or triple-click a line.",
                    "Herhangi bir uygulamada metin seç — kelimeye çift tıkla, ifadeyi \
                     sürükle ya da satıra üç kez tıkla.",
                    "Selecciona texto en cualquier app — doble clic en una palabra, \
                     arrastra una frase o triple clic en una línea.",
                )
                .to_string(),
            })
            .size(12.0)
            .color(TEXT_MUTED),
        );
    } else {
        ui.label(
            egui::RichText::new(t(
                "● Not watching for selections",
                "● Seçimler izlenmiyor",
                "● No se vigilan selecciones",
            ))
            .size(12.5)
            .color(ERR_RED),
        );
        ui.label(
            egui::RichText::new(&state.readiness.detail)
                .size(12.0)
                .color(WARN_AMBER),
        );
        // Where the system has a page for this, offer the page. The status
        // above says what is wrong; a sentence cannot tick the switch, and
        // the path to it is long enough that describing it is how people get
        // lost.
        if let Some(fix) = &state.readiness.fix {
            ui.add_space(4.0);
            if ui.button(fix.label).clicked() {
                crate::shell::open_url(fix.url);
            }
        }
    }
}

fn translate_box(ui: &mut egui::Ui, state: &mut MainState, cfg: &Config) {
    ui.add(
        egui::TextEdit::multiline(&mut state.input)
            .desired_rows(3)
            .desired_width(f32::INFINITY)
            .hint_text(t(
                "Type or paste text to translate…",
                "Çevrilecek metni yaz ya da yapıştır…",
                "Escribe o pega el texto a traducir…",
            )),
    );
    ui.add_space(6.0);

    ui.horizontal(|ui| {
        let can_send = !state.input.trim().is_empty() && !state.translating;
        if ui
            .add_enabled(
                can_send,
                egui::Button::new(t("Translate", "Çevir", "Traducir")),
            )
            .clicked()
        {
            state.translating = true;
            state.result = None;
            state.capped = None;
            let _ = state.requests.send(Request::Manual(state.input.clone()));
        }
        if ui.button(t("Clear", "Temizle", "Borrar")).clicked() {
            state.input.clear();
            state.result = None;
            state.capped = None;
        }
        if state.translating {
            ui.spinner();
        }
        ui.label(
            egui::RichText::new(format!("→ {}", language_name(&cfg.target_lang)))
                .size(11.5)
                .color(TEXT_MUTED),
        );
    });

    if let Some((_, limit)) = state.capped {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(match i18n::lang() {
                UiLang::En => format!("Today's {limit} free translations are used"),
                UiLang::Tr => format!("Bugünkü {limit} ücretsiz çeviri kullanıldı"),
                UiLang::Es => format!("Se usaron las {limit} traducciones gratis de hoy"),
            })
            .size(13.0)
            .color(WARN_AMBER),
        );
        ui.label(
            egui::RichText::new(format!(
                "{} {} {} {}.",
                t(
                    "They come back at midnight. Pro removes the limit —",
                    "Gece yarısı yenilenir. Pro sınırı kaldırır —",
                    "Vuelven a medianoche. Pro quita el límite —",
                ),
                license::PRICE_MONTHLY,
                t("or", "ya da", "o"),
                license::PRICE_YEARLY,
            ))
            .size(11.5)
            .color(TEXT_MUTED),
        );
        ui.add_space(4.0);
        if ui
            .button(t("Upgrade to Pro", "Pro'ya geç", "Pasar a Pro"))
            .clicked()
        {
            crate::shell::open_url(&format!("{}?src=window", license::BUY_URL));
        }
    }

    match &state.result {
        None => {}
        Some(Ok(result)) => {
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(&result.text)
                    .size(cfg.font_size)
                    .line_height(Some(cfg.font_size * 1.45))
                    .color(TEXT_PRIMARY),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{} · {} → {}",
                        result.provider.label(),
                        language_name(&result.source_lang),
                        language_name(&cfg.target_lang),
                    ))
                    .size(11.0)
                    .color(TEXT_MUTED),
                );
                if ui.small_button(t("Copy", "Kopyala", "Copiar")).clicked() {
                    crate::capture::set_clipboard(ui.ctx(), &result.text);
                }
            });
        }
        Some(Err(errors)) => {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(t(
                    "No provider could translate this",
                    "Hiçbir servis bunu çeviremedi",
                    "Ningún proveedor pudo traducir esto",
                ))
                .size(13.0)
                .color(ERR_RED),
            );
            for (provider, err) in errors {
                ui.label(
                    egui::RichText::new(format!("{}: {err}", provider.label()))
                        .size(11.5)
                        .color(TEXT_MUTED),
                );
            }
        }
    }
}

fn recent(ui: &mut egui::Ui, state: &MainState) {
    for entry in state.recent.iter().take(10) {
        ui.label(
            egui::RichText::new(truncate(&entry.source, 70))
                .size(11.0)
                .color(TEXT_MUTED),
        );
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(truncate(&entry.translated, 90))
                    .size(12.5)
                    .color(TEXT_PRIMARY),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(t("Copy", "Kopyala", "Copiar")).clicked() {
                    crate::capture::set_clipboard(ui.ctx(), &entry.translated);
                }
                ui.label(
                    egui::RichText::new(entry.provider.label())
                        .size(10.0)
                        .color(TEXT_MUTED),
                );
            });
        });
        ui.add_space(8.0);
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    let mut out: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        out.push('…');
    }
    out
}
