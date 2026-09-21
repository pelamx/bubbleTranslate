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
                                [egui::pos2(r.center().x - 7.0, y), egui::pos2(r.center().x + 7.0, y)],
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

fn languages(ui: &mut egui::Ui, state: &mut MainState, cfg: &mut Config) -> bool {
    let mut dirty = false;

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(t("Translate into", "Hedef dil", "Traducir a"))
                .size(12.5)
                .color(TEXT_SECONDARY),
        );
        let current = cfg.target_lang.clone();
        let mut picked: Option<String> = None;
        egui::ComboBox::from_id_salt("main-target")
            .selected_text(language_name(&current))
            .width(170.0)
            .show_ui(ui, |ui| {
                for (code, name) in LANGUAGES {
                    if ui.selectable_label(*code == current, *name).clicked() {
                        picked = Some((*code).to_string());
                    }
                }
            });
        if let Some(code) = picked {
            if code != cfg.target_lang {
                cfg.target_lang = code;
                dirty = true;
                // Apply the change to the text already captured, so the bubble
                // updates without needing a fresh selection.
                let _ = state.requests.send(Request::Retranslate);
            }
        }
    });

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(t("Source language", "Kaynak dil", "Idioma de origen"))
                .size(12.5)
                .color(TEXT_SECONDARY),
        );
        let current = cfg.source_lang.clone();
        let label = if current == "auto" {
            t(
                "Detect automatically",
                "Otomatik algıla",
                "Detectar automáticamente",
            )
        } else {
            language_name(&current)
        };
        let mut picked: Option<String> = None;
        egui::ComboBox::from_id_salt("main-source")
            .selected_text(label)
            .width(170.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(
                        current == "auto",
                        t(
                            "Detect automatically",
                            "Otomatik algıla",
                            "Detectar automáticamente",
                        ),
                    )
                    .clicked()
                {
                    picked = Some("auto".to_string());
                }
                for (code, name) in LANGUAGES {
                    if ui.selectable_label(*code == current, *name).clicked() {
                        picked = Some((*code).to_string());
                    }
                }
            });
        if let Some(code) = picked {
            if code != cfg.source_lang {
                cfg.source_lang = code;
                dirty = true;
            }
        }
    });

    dirty
}

fn providers(ui: &mut egui::Ui, state: &mut MainState, cfg: &mut Config) -> bool {
    let mut dirty = false;

    ui.label(
        egui::RichText::new(t(
            "Tried top to bottom; the first one to answer wins.",
            "Yukarıdan aşağı denenir; ilk yanıt veren kazanır.",
            "Se prueban de arriba abajo; gana el primero que responde.",
        ))
        .size(11.5)
        .color(TEXT_MUTED),
    );
    ui.add_space(6.0);

    // Reordering is applied after the loop so the list is not mutated while
    // it is being drawn.
    let mut swap: Option<(usize, usize)> = None;
    let count = cfg.providers.len();

    for (index, provider) in cfg.providers.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("{}.", index + 1))
                    .size(12.5)
                    .color(TEXT_MUTED),
            );
            ui.label(
                egui::RichText::new(provider.label())
                    .size(13.0)
                    .color(TEXT_PRIMARY),
            );

            // DeepL without a key never reaches the network, so say so here
            // rather than letting it look like a silent failure.
            if *provider == Provider::DeepL && cfg.deepl_api_key.trim().is_empty() {
                ui.label(
                    egui::RichText::new(t(
                        "(skipped — no API key)",
                        "(atlandı — API anahtarı yok)",
                        "(omitido — sin clave API)",
                    ))
                    .size(11.0)
                    .color(TEXT_MUTED),
                );
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(index + 1 < count, egui::Button::new("↓").small())
                    .clicked()
                {
                    swap = Some((index, index + 1));
                }
                if ui
                    .add_enabled(index > 0, egui::Button::new("↑").small())
                    .clicked()
                {
                    swap = Some((index, index - 1));
                }
            });
        });
    }
    if let Some((a, b)) = swap {
        cfg.providers.swap(a, b);
        dirty = true;
    }

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(t(
                "DeepL API key",
                "DeepL API anahtarı",
                "Clave API de DeepL",
            ))
            .size(12.5)
            .color(TEXT_SECONDARY),
        );
        if ui
            .add(
                egui::TextEdit::singleline(&mut cfg.deepl_api_key)
                    .password(true)
                    .hint_text(t(
                        "optional — free keys end in :fx",
                        "isteğe bağlı — ücretsiz anahtarlar :fx ile biter",
                        "opcional — las claves gratis terminan en :fx",
                    ))
                    .desired_width(220.0),
            )
            .lost_focus()
        {
            dirty = true;
        }
    });

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(t(
                "MyMemory email",
                "MyMemory e-postası",
                "Correo de MyMemory",
            ))
            .size(12.5)
            .color(TEXT_SECONDARY),
        );
        if ui
            .add(
                egui::TextEdit::singleline(&mut cfg.mymemory_email)
                    .hint_text(t(
                        "optional — raises the daily quota",
                        "isteğe bağlı — günlük kotayı artırır",
                        "opcional — aumenta la cuota diaria",
                    ))
                    .desired_width(220.0),
            )
            .lost_focus()
        {
            dirty = true;
        }
    });

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                !state.testing,
                egui::Button::new(t(
                    "Test providers",
                    "Servisleri test et",
                    "Probar proveedores",
                )),
            )
            .clicked()
        {
            state.testing = true;
            state.statuses.clear();
            let _ = state.requests.send(Request::TestProviders);
        }
        if state.testing {
            ui.spinner();
        }
    });

    if !state.statuses.is_empty() {
        ui.add_space(6.0);
        for (provider, outcome) in &state.statuses {
            ui.horizontal(|ui| {
                let (mark, colour, detail) = match outcome {
                    Ok(text) => ("ok", OK_GREEN, text.clone()),
                    Err(err) => ("fail", ERR_RED, err.clone()),
                };
                ui.label(
                    egui::RichText::new(format!("{:<9}", provider.label()))
                        .size(11.5)
                        .color(TEXT_SECONDARY),
                );
                ui.label(egui::RichText::new(mark).size(11.5).color(colour));
                ui.label(egui::RichText::new(detail).size(11.5).color(TEXT_MUTED));
            });
        }
    }

    dirty
}

/// The licence panel: what this install may do, and how to change it.
///
/// Deliberately the plainest section in the window. Everything here is either
/// a fact about the account or a button that opens a browser — the app itself
/// never asks for a card, an address, or an email.
fn account(
    ui: &mut egui::Ui,
    state: &mut MainState,
    cfg: &mut Config,
    licensing: &Licensing,
) -> bool {
    let mut dirty = false;

    // The two locks are taken one after the other, never together: the engine
    // reads them in this same order while a translation is in flight.
    let (is_pro, plan, status, busy, entitlement) = {
        let licence = licensing.license.lock().unwrap();
        (
            licence.entitlement.is_pro(),
            licence.entitlement.plan,
            licence.status.clone(),
            licence.busy,
            licence.entitlement.clone(),
        )
    };
    let (used, limit, left, grandfathered) = {
        let mut quota = licensing.quota.lock().unwrap();
        (
            quota.used_today(),
            quota.limit(&entitlement),
            quota.remaining(&entitlement),
            quota.is_grandfathered(),
        )
    };

    // -- where this install stands ----------------------------------------
    match limit {
        None => {
            let cycle = entitlement.cycle;
            ui.label(
                egui::RichText::new(if grandfathered {
                    t(
                        "● Unlimited translations",
                        "● Sınırsız çeviri",
                        "● Traducciones ilimitadas",
                    )
                    .to_string()
                } else {
                    match cycle {
                        Some(cycle) => format!(
                            "● {} ({}) — {}",
                            plan.label(),
                            cycle.label(),
                            t(
                                "unlimited translations",
                                "sınırsız çeviri",
                                "traducciones ilimitadas"
                            ),
                        ),
                        None => format!(
                            "● {} — {}",
                            plan.label(),
                            t(
                                "unlimited translations",
                                "sınırsız çeviri",
                                "traducciones ilimitadas"
                            ),
                        ),
                    }
                })
                .size(13.0)
                .color(OK_GREEN),
            );
            if grandfathered {
                ui.label(
                    egui::RichText::new(t(
                        "This install predates the free allowance, so the limit does \
                             not apply to it.",
                        "Bu kurulum ücretsiz kotadan önceye ait, sınır ona uygulanmaz.",
                        "Esta instalación es anterior al cupo gratuito, así que el \
                             límite no se le aplica.",
                    ))
                    .size(11.0)
                    .color(TEXT_MUTED),
                );
            }
            ui.label(
                egui::RichText::new(match i18n::lang() {
                    UiLang::En => format!("{used} translated today"),
                    UiLang::Tr => format!("Bugün {used} çeviri"),
                    UiLang::Es => format!("{used} traducidas hoy"),
                })
                .size(11.5)
                .color(TEXT_MUTED),
            );
        }
        Some(limit) => {
            let left = left.unwrap_or(0);
            let spent = left == 0;
            ui.label(
                egui::RichText::new(match (i18n::lang(), spent) {
                    (UiLang::En, true) => {
                        format!("● Free — all {limit} of today's translations used")
                    }
                    (UiLang::En, false) => {
                        format!("● Free — {left} of {limit} translations left today")
                    }
                    (UiLang::Tr, true) => {
                        format!("● Ücretsiz — bugünkü {limit} çevirinin hepsi kullanıldı")
                    }
                    (UiLang::Tr, false) => {
                        format!("● Ücretsiz — bugün {limit} çeviriden {left} kaldı")
                    }
                    (UiLang::Es, true) => {
                        format!("● Gratis — usadas las {limit} traducciones de hoy")
                    }
                    (UiLang::Es, false) => {
                        format!("● Gratis — quedan {left} de {limit} traducciones hoy")
                    }
                })
                .size(13.0)
                .color(if spent { WARN_AMBER } else { TEXT_SECONDARY }),
            );
            ui.label(
                egui::RichText::new(if spent {
                    t(
                        "The count comes back at midnight. Pro removes the limit.",
                        "Sayaç gece yarısı yenilenir. Pro sınırı kaldırır.",
                        "El contador vuelve a medianoche. Pro quita el límite.",
                    )
                } else {
                    t(
                        "Ten a day, counted at your local midnight. Re-reading something \
                         already translated does not count.",
                        "Günde on çeviri, yerel gece yarısında sıfırlanır. Zaten çevrilmiş \
                         bir şeyi tekrar okumak sayılmaz.",
                        "Diez al día, contadas desde tu medianoche local. Releer algo ya \
                         traducido no cuenta.",
                    )
                })
                .size(11.0)
                .color(TEXT_MUTED),
            );
        }
    }

    // -- anything the licence needs to say --------------------------------
    match &status {
        Status::None => {}
        Status::Active { renews } => {
            if let Some(renews) = renews {
                ui.label(
                    egui::RichText::new(format!(
                        "{} {renews}",
                        t("Renews", "Yenilenme:", "Se renueva el")
                    ))
                    .size(11.5)
                    .color(TEXT_MUTED),
                );
            }
            ui.label(
                egui::RichText::new(match i18n::lang() {
                    UiLang::En => format!(
                        "Checked online about every {} days; works offline in between.",
                        license::TOKEN_TTL_HINT / 86_400,
                    ),
                    UiLang::Tr => format!(
                        "Yaklaşık {} günde bir çevrimiçi kontrol edilir; arada çevrimdışı çalışır.",
                        license::TOKEN_TTL_HINT / 86_400,
                    ),
                    UiLang::Es => format!(
                        "Se comprueba en línea cada {} días más o menos; funciona sin conexión entre medias.",
                        license::TOKEN_TTL_HINT / 86_400,
                    ),
                })
                .size(11.0)
                .color(TEXT_MUTED),
            );
        }
        Status::Lapsed => {
            ui.label(
                egui::RichText::new(t(
                    "Your licence has expired.",
                    "Lisansının süresi doldu.",
                    "Tu licencia ha caducado.",
                ))
                .size(11.5)
                .color(WARN_AMBER),
            );
        }
        Status::Problem(why) => {
            ui.label(egui::RichText::new(why).size(11.5).color(ERR_RED));
        }
    }

    ui.add_space(8.0);

    // -- what can be done about it ----------------------------------------
    if is_pro {
        ui.horizontal(|ui| {
            if ui
                .button(t(
                    "Manage subscription",
                    "Aboneliği yönet",
                    "Gestionar suscripción",
                ))
                .clicked()
            {
                crate::shell::open_url(&format!("{}?src=window", license::MANAGE_URL));
            }
            if ui
                .button(t(
                    "Remove from this device",
                    "Bu cihazdan kaldır",
                    "Quitar de este dispositivo",
                ))
                .on_hover_text(t(
                    "Frees the device slot so the licence can be used on another machine.",
                    "Cihaz yerini boşaltır, böylece lisans başka bir makinede kullanılabilir.",
                    "Libera el hueco del dispositivo para usar la licencia en otra máquina.",
                ))
                .clicked()
            {
                state.key_input.clear();
                cfg.license_key.clear();
                dirty = true;
                let _ = state.requests.send(Request::DeactivateLicense);
            }
        });
    } else {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut state.key_input)
                    .hint_text("BT-XXXXX-XXXXX-XXXXX")
                    .desired_width(190.0),
            );
            let ready = !state.key_input.trim().is_empty() && !busy;
            if ui
                .add_enabled(
                    ready,
                    egui::Button::new(t("Activate", "Etkinleştir", "Activar")),
                )
                .clicked()
            {
                // Saved before the exchange, not after: a key that the service
                // could not be reached about is still the key the user owns,
                // and they should not have to find the email again.
                cfg.license_key = state.key_input.trim().to_string();
                dirty = true;
                let _ = state
                    .requests
                    .send(Request::ActivateLicense(cfg.license_key.clone()));
            }
            if busy {
                ui.spinner();
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button(t("Get Pro", "Pro al", "Obtener Pro")).clicked() {
                crate::shell::open_url(&format!("{}?src=window", license::BUY_URL));
            }
            ui.label(
                egui::RichText::new(format!(
                    "{} {} {} — {}",
                    license::PRICE_MONTHLY,
                    t("or", "ya da", "o"),
                    license::PRICE_YEARLY,
                    t(
                        "unlimited translations, three devices.",
                        "sınırsız çeviri, üç cihaz.",
                        "traducciones ilimitadas, tres dispositivos.",
                    ),
                ))
                .size(11.0)
                .color(TEXT_MUTED),
            );
        });
    }

    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(format!(
            "{} {} · {}",
            t("Device", "Cihaz", "Dispositivo"),
            &license::device_id()[..8],
            t(
                "translations are never sent through our servers",
                "çeviriler asla sunucularımızdan geçmez",
                "las traducciones nunca pasan por nuestros servidores",
            ),
        ))
        .size(10.5)
        .color(TEXT_MUTED),
    );

    dirty
}

fn behaviour(ui: &mut egui::Ui, cfg: &mut Config) -> bool {
    let mut dirty = false;

    dirty |= ui
        .checkbox(
            &mut cfg.auto_translate,
            t(
                "Translate on selection",
                "Seçince çevir",
                "Traducir al seleccionar",
            ),
        )
        .changed();

    ui.horizontal(|ui| {
        ui.label(t("Hold", "Seçerken", "Mantén"));
        egui::ComboBox::from_id_salt("trigger-key")
            .selected_text(cfg.trigger_key.label())
            .show_ui(ui, |ui| {
                for key in TriggerKey::ALL {
                    if ui
                        .selectable_label(*key == cfg.trigger_key, key.label())
                        .clicked()
                    {
                        cfg.trigger_key = *key;
                        dirty = true;
                    }
                }
            });
        ui.label(t("while selecting", "basılı tut", "al seleccionar"));
    });

    ui.horizontal(|ui| {
        ui.label(t("Bubble theme", "Baloncuk teması", "Tema de la burbuja"));
        egui::ComboBox::from_id_salt("bubble-theme-main")
            .selected_text(cfg.theme.label())
            .show_ui(ui, |ui| {
                for theme in BubbleTheme::ALL {
                    if ui
                        .selectable_label(*theme == cfg.theme, theme.label())
                        .clicked()
                    {
                        cfg.theme = *theme;
                        dirty = true;
                    }
                }
            });
    });
    ui.label(
        egui::RichText::new(t(
            "The colour scheme of the translation bubble. It changes the moment \
                 you pick one — the main window keeps its own look.",
            "Çeviri baloncuğunun renk şeması. Seçtiğin an değişir — ana pencere \
                 kendi görünümünü korur.",
            "El esquema de colores de la burbuja. Cambia en cuanto eliges uno — \
                 la ventana principal mantiene su aspecto.",
        ))
        .size(11.0)
        .color(TEXT_MUTED),
    );

    ui.label(
        egui::RichText::new(match crate::monitor::trigger_key_blocked() {
            None => t(
                "Only a selection made with that key held pops a bubble, so \
                 selecting text for any other reason stays quiet.",
                "Baloncuk yalnızca o tuş basılıyken yapılan seçimde açılır; başka \
                 amaçla metin seçmek sessiz kalır.",
                "Solo una selección hecha con esa tecla pulsada abre la burbuja, así \
                 que seleccionar texto por otro motivo no molesta.",
            )
            .to_string(),
            // The reason travels from the platform rather than being written
            // here, because what the user can do about it differs: a group to
            // join is worth saying, a protocol that does not exist is not.
            Some(reason) => match i18n::lang() {
                UiLang::En => format!(
                    "Not in force: {reason}. Every selection is translated until then — \
                     or bind a key to `bubbleTranslate --translate-selection`, which \
                     needs nothing from the session."
                ),
                UiLang::Tr => format!(
                    "Geçerli değil: {reason}. O zamana kadar her seçim çevrilir — ya da \
                     oturumdan hiçbir şey gerektirmeyen `bubbleTranslate --translate-selection` \
                     komutuna bir tuş ata."
                ),
                UiLang::Es => format!(
                    "No está activo: {reason}. Hasta entonces se traduce cada selección — \
                     o asigna una tecla a `bubbleTranslate --translate-selection`, que no \
                     necesita nada de la sesión."
                ),
            },
        })
        .size(11.0)
        .color(TEXT_MUTED),
    );
    dirty |= ui
        .checkbox(
            &mut cfg.watch_clipboard,
            t(
                "Also translate on copy (Ctrl+C)",
                "Kopyalayınca da çevir (Ctrl+C)",
                "Traducir también al copiar (Ctrl+C)",
            ),
        )
        .on_hover_text(t(
            "Selecting text publishes it to the desktop by itself, which is how the \
                 bubble works without the other application's help. A few — anything \
                 drawing its own text, this window included — publish nothing, and \
                 copying is the one gesture that always gets through.",
            "Metin seçmek onu masaüstüne kendiliğinden bildirir; baloncuk diğer \
                 uygulamanın yardımı olmadan böyle çalışır. Kendi metnini çizen birkaç \
                 uygulama — bu pencere dahil — hiçbir şey bildirmez; kopyalamak her \
                 zaman işe yarayan tek harekettir.",
            "Seleccionar texto lo publica en el escritorio por sí solo; así funciona \
                 la burbuja sin ayuda de la otra aplicación. Algunas — las que dibujan \
                 su propio texto, esta ventana incluida — no publican nada, y copiar es \
                 el único gesto que siempre funciona.",
        ))
        .changed();

    // Only macOS and Windows have a capture strategy to fall back to: both ask
    // the application for its selection and can synthesize a copy when it will
    // not say. On Linux the desktop hands the selection over directly, so
    // there is no second route to switch on and the setting would be a control
    // over nothing.
    if !cfg!(target_os = "linux") {
        dirty |= ui
            .checkbox(
                &mut cfg.clipboard_fallback,
                t(
                    "Use copy fallback when an app hides its selection",
                    "Uygulama seçimini gizlerse kopyalama yedeğini kullan",
                    "Usar copia de respaldo si una app oculta su selección",
                ),
            )
            .on_hover_text(if cfg!(target_os = "windows") {
                t(
                    "Needed for PDF viewers and anything drawing its own text, which \
                     expose nothing over UI Automation. Briefly borrows the clipboard \
                     and restores the text afterwards.",
                    "UI Automation üzerinden hiçbir şey sunmayan PDF görüntüleyiciler ve \
                     kendi metnini çizen uygulamalar için gerekir. Panoyu kısa süre ödünç \
                     alır ve sonra geri koyar.",
                    "Necesario para visores de PDF y apps que dibujan su propio texto, que \
                     no exponen nada por UI Automation. Toma prestado el portapapeles un \
                     momento y luego lo restaura.",
                )
            } else {
                t(
                    "Needed for terminals and PDF viewers, which expose nothing over the \
                     Accessibility API. Briefly borrows the clipboard and restores the \
                     text afterwards.",
                    "Erişilebilirlik API'si üzerinden hiçbir şey sunmayan terminaller ve \
                     PDF görüntüleyiciler için gerekir. Panoyu kısa süre ödünç alır ve \
                     sonra geri koyar.",
                    "Necesario para terminales y visores de PDF, que no exponen nada por la \
                     API de Accesibilidad. Toma prestado el portapapeles un momento y luego \
                     lo restaura.",
                )
            })
            .changed();
    }

    // Only offered where the window can be got back: without an indicator
    // this would be a switch that makes the app unreachable on its next start.
    if crate::shell::has_indicator() {
        dirty |= ui
            .checkbox(
                &mut cfg.start_in_background,
                t(
                    "Start without the window",
                    "Pencere olmadan başlat",
                    "Iniciar sin la ventana",
                ),
            )
            .on_hover_text(t(
                "Launches straight into the background: no settings window, just the \
                     tray icon and the bubble. The window is one click on that icon away.",
                "Doğrudan arka planda açılır: ayar penceresi yok, sadece tepsi simgesi \
                     ve baloncuk. Pencere o simgeye bir tık uzaklıkta.",
                "Arranca directamente en segundo plano: sin ventana de ajustes, solo el \
                     icono de la bandeja y la burbuja. La ventana está a un clic en ese icono.",
            ))
            .changed();
    }

    ui.add_space(8.0);
    let mut scale = cfg.ui_scale * 100.0;
    if slider(
        ui,
        t("Interface scale", "Arayüz ölçeği", "Escala de la interfaz"),
        &mut scale,
        60.0..=140.0,
        "%",
    ) {
        cfg.ui_scale = scale / 100.0;
        dirty = true;
    }
    ui.label(
        egui::RichText::new(t(
            "Sizes the whole app. The display's own scaling is already matched; \
                 this is for desktops that run denser or looser than that.",
            "Tüm uygulamayı boyutlandırır. Ekranın kendi ölçeği zaten uygulanır; \
                 bu, daha sık ya da daha seyrek çalışan masaüstleri içindir.",
            "Ajusta el tamaño de toda la app. La escala de la pantalla ya se aplica; \
                 esto es para escritorios más densos o más holgados.",
        ))
        .size(11.0)
        .color(TEXT_MUTED),
    );

    ui.add_space(8.0);
    dirty |= slider(
        ui,
        t(
            "Bubble text size",
            "Baloncuk yazı boyutu",
            "Tamaño del texto de la burbuja",
        ),
        &mut cfg.font_size,
        12.0..=26.0,
        "pt",
    );

    ui.add_space(4.0);
    let mut hide = cfg.auto_hide_secs as f32;
    if slider(
        ui,
        t("Auto-hide after", "Otomatik gizle", "Ocultar tras"),
        &mut hide,
        0.0..=60.0,
        "s",
    ) {
        cfg.auto_hide_secs = hide as u64;
        dirty = true;
    }
    ui.label(
        egui::RichText::new(t(
            "0 keeps the bubble up until closed. Pauses while hovered.",
            "0, baloncuğu kapatılana kadar açık tutar. Üzerindeyken durur.",
            "0 mantiene la burbuja hasta cerrarla. Se pausa al pasar el ratón.",
        ))
        .size(10.5)
        .color(TEXT_MUTED),
    );

    ui.add_space(6.0);
    let mut debounce = cfg.debounce_ms as f32;
    if slider(
        ui,
        t(
            "Settle delay",
            "Bekleme süresi",
            "Retardo de estabilización",
        ),
        &mut debounce,
        50.0..=600.0,
        "ms",
    ) {
        cfg.debounce_ms = debounce as u64;
        dirty = true;
    }
    ui.label(
        egui::RichText::new(t(
            "How still the selection must be to count as finished. Raise it if the \
                 bubble appears mid-sweep or an app returns a stale selection.",
            "Seçimin bitmiş sayılması için ne kadar sabit kalması gerektiği. Baloncuk \
                 sürüklerken çıkıyorsa ya da bir uygulama eski seçimi veriyorsa artır.",
            "Cuánto debe quedarse quieta la selección para darla por terminada. Súbelo \
                 si la burbuja sale a mitad del arrastre o una app da una selección vieja.",
        ))
        .size(10.5)
        .color(TEXT_MUTED),
    );

    ui.add_space(6.0);
    let mut max = cfg.max_chars as f32;
    if slider(
        ui,
        t("Longest selection", "En uzun seçim", "Selección más larga"),
        &mut max,
        100.0..=8000.0,
        t(" chars", " karakter", " caracteres"),
    ) {
        cfg.max_chars = max as usize;
        dirty = true;
    }

    dirty
}

fn slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).size(12.5).color(TEXT_SECONDARY));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(format!("{:.0}{suffix}", *value))
                    .size(11.5)
                    .color(TEXT_MUTED),
            );
            changed = ui
                .add(
                    egui::Slider::new(value, range)
                        .show_value(false)
                        .trailing_fill(true),
                )
                .drag_stopped();
        });
    });
    changed
}

/// Where to write. The same address the website gives, so a reply from it is
/// recognisable rather than a stranger's.
const SUPPORT_EMAIL: &str = "pelamx@bubbletranslate.app";

/// How much of a message a `mailto:` link can carry before the system that
/// opens it starts truncating or refusing. Windows is the strict one, at
/// roughly two thousand characters for the whole command; this leaves room for
/// the address, the subject and the encoding, which triples the length of
/// anything that is not plain ASCII.
const MAILTO_BUDGET: usize = 1200;

/// Somewhere to say what is wrong, without leaving the app to find out where.
///
/// The message travels through the user's own mail program rather than a form
/// this app posts somewhere: bubbleTranslate keeps no account and no database,
/// and a complaint box that quietly shipped text to a server of ours would be
/// the one place that stopped being true. It also means the sender keeps a
/// copy in their sent mail and a reply lands where they expect it.
fn feedback(ui: &mut egui::Ui, state: &mut MainState, cfg: &mut Config) -> bool {
    let mut dirty = false;
    ui.label(
        egui::RichText::new(t(
            "Something broken, something missing, or something that annoyed you \u{2014} \
                 it is read by the person who wrote the app.",
            "Bozuk, eksik ya da seni rahatsız eden bir şey mi var \u{2014} uygulamayı \
                 yazan kişi okur.",
            "Algo roto, algo que falta o algo que te molestó \u{2014} lo lee quien \
                 escribió la app.",
        ))
        .size(11.5)
        .color(TEXT_SECONDARY),
    );
    ui.add_space(6.0);

    ui.add(
        egui::TextEdit::multiline(&mut state.feedback)
            .desired_rows(4)
            .desired_width(f32::INFINITY)
            .hint_text(t(
                "What happened, or what you wish it did\u{2026}",
                "Ne oldu, ya da ne yapmasını isterdin\u{2026}",
                "Qué pasó, o qué te gustaría que hiciera\u{2026}",
            )),
    );
    ui.add_space(6.0);

    ui.horizontal(|ui| {
        let ready = !state.feedback.trim().is_empty();
        if ui
            .add_enabled(
                ready,
                egui::Button::new(t("Write the mail", "E-postayı yaz", "Escribir el correo")),
            )
            .on_hover_text(format!(
                "{} {SUPPORT_EMAIL}",
                t("Addressed to", "Alıcı:", "Dirigido a")
            ))
            .clicked()
        {
            state.feedback_note = Some(send_feedback(
                ui.ctx(),
                state.feedback.trim(),
                cfg.feedback_via,
            ));
        }
        ui.label(
            egui::RichText::new(t("in", "ile", "en"))
                .size(11.5)
                .color(TEXT_MUTED),
        );
        // Remembered, because someone who reads their mail on the web will
        // answer this question the same way every time.
        egui::ComboBox::from_id_salt("feedback-via")
            .selected_text(cfg.feedback_via.label())
            .show_ui(ui, |ui| {
                for via in FeedbackVia::ALL {
                    if ui
                        .selectable_label(cfg.feedback_via == *via, via.label())
                        .clicked()
                    {
                        cfg.feedback_via = *via;
                        dirty = true;
                    }
                }
            });
        if ui
            .button(t(
                "Copy the address",
                "Adresi kopyala",
                "Copiar la dirección",
            ))
            .on_hover_text(t(
                "If you would rather write from somewhere else",
                "Başka bir yerden yazmayı tercih edersen",
                "Si prefieres escribir desde otro sitio",
            ))
            .clicked()
        {
            ui.ctx().copy_text(SUPPORT_EMAIL.to_string());
            state.feedback_note = Some(format!(
                "{SUPPORT_EMAIL} {}",
                t(
                    "is on the clipboard.",
                    "panoya kopyalandı.",
                    "está en el portapapeles."
                )
            ));
        }
    });

    if let Some(note) = &state.feedback_note {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(note).size(11.0).color(OK_GREEN));
    }

    ui.add_space(6.0);
    // Said plainly, because a message that silently carried system details
    // would be exactly the kind of thing someone writes in to complain about.
    ui.label(
        egui::RichText::new({
            let (v, os) = (env!("CARGO_PKG_VERSION"), std::env::consts::OS);
            match i18n::lang() {
                UiLang::En => format!(
                    "Goes through your own mail app \u{2014} nothing is sent from here. The version \
                     ({v}) and system ({os}) are added to the end so a reply can make sense; \
                     you can delete them before sending."
                ),
                UiLang::Tr => format!(
                    "Kendi e-posta uygulamandan gider \u{2014} buradan hiçbir şey gönderilmez. \
                     Yanıt anlamlı olsun diye sürüm ({v}) ve sistem ({os}) sona eklenir; \
                     göndermeden önce silebilirsin."
                ),
                UiLang::Es => format!(
                    "Sale desde tu propia app de correo \u{2014} desde aquí no se envía nada. \
                     La versión ({v}) y el sistema ({os}) se añaden al final para que la \
                     respuesta tenga sentido; puedes borrarlos antes de enviar."
                ),
            }
        })
        .size(10.5)
        .color(TEXT_MUTED),
    );
    // The failure this is here for: a browser that is not the system's mail
    // handler answers a mailto: link with an empty tab and no explanation, and
    // the person is left thinking the button is broken.
    if cfg.feedback_via == FeedbackVia::MailApp {
        ui.label(
            egui::RichText::new(t(
                "Opened an empty tab instead? You read your mail on the web \u{2014} pick \
                     Gmail or Outlook.com above and it will open there.",
                "Boş bir sekme mi açıldı? E-postanı web'de okuyorsun \u{2014} yukarıdan \
                     Gmail ya da Outlook.com'u seç, orada açılsın.",
                "¿Se abrió una pestaña vacía? Lees tu correo en la web \u{2014} elige \
                     Gmail u Outlook.com arriba y se abrirá allí.",
            ))
            .size(10.5)
            .color(TEXT_MUTED),
        );
    }

    dirty
}

/// What a message turns into: the link to open, whatever has to go on the
/// clipboard first, and what to tell the user.
struct Mail {
    url: String,
    clipboard: Option<String>,
    note: &'static str,
}

/// Works out the above, and nothing else \u{2014} no windows opened, no clipboard
/// touched \u{2014} so the rules below can be tested rather than trusted.
fn compose(message: &str, version: &str, os: &str, via: FeedbackVia) -> Mail {
    let subject = format!("bubbleTranslate feedback \u{2014} {version} ({os})");
    let body = format!("{message}\n\n\u{2014}\nbubbleTranslate {version} on {os}");
    let to = SUPPORT_EMAIL;
    let (su, bo) = (urlencoding::encode(&subject), urlencoding::encode(&body));

    // The webmail compose pages are ordinary web pages: the browser is already
    // signed in, the address bar takes thousands of characters, and there is
    // nothing to install. Only `mailto:` has the length problem below.
    match via {
        FeedbackVia::Gmail => Mail {
            url: format!("https://mail.google.com/mail/?view=cm&fs=1&to={to}&su={su}&body={bo}"),
            clipboard: None,
            note: t(
                "Gmail should be open with it. Nothing has been sent until you send it.",
                "Gmail onunla açılmış olmalı. Sen gönderene kadar hiçbir şey gönderilmedi.",
                "Gmail debería estar abierto con él. No se envía nada hasta que lo envíes.",
            ),
        },
        FeedbackVia::Outlook => Mail {
            url: format!(
                "https://outlook.live.com/mail/0/deeplink/compose?to={to}&subject={su}&body={bo}"
            ),
            clipboard: None,
            note: t(
                "Outlook should be open with it. Nothing has been sent until you send it.",
                "Outlook onunla açılmış olmalı. Sen gönderene kadar hiçbir şey gönderilmedi.",
                "Outlook debería estar abierto con él. No se envía nada hasta que lo envíes.",
            ),
        },
        FeedbackVia::MailApp => {
            // A long message is not squeezed into the link: past the budget the
            // system either truncates it or refuses to open anything at all,
            // and losing what someone just wrote is worse than asking them to
            // paste it.
            if bo.len() > MAILTO_BUDGET {
                return Mail {
                    url: format!("mailto:{to}?subject={su}"),
                    clipboard: Some(body),
                    note: t(
                        "That is a long one, so it is on the clipboard \u{2014} paste it into \
                         the mail that just opened.",
                        "Bu uzun bir mesaj, panoya kopyalandı \u{2014} az önce açılan e-postaya \
                         yapıştır.",
                        "Es largo, así que está en el portapapeles \u{2014} pégalo en el correo \
                         que se acaba de abrir.",
                    ),
                };
            }
            Mail {
                url: format!("mailto:{to}?subject={su}&body={bo}"),
                clipboard: None,
                note: t(
                    "Your mail app should be open with it. Nothing has been sent until \
                     you send it.",
                    "E-posta uygulaman onunla açılmış olmalı. Sen gönderene kadar hiçbir \
                     şey gönderilmedi.",
                    "Tu app de correo debería estar abierta con él. No se envía nada hasta \
                     que lo envíes.",
                ),
            }
        }
    }
}

/// Hands the message to the mail program, and says what became of it.
fn send_feedback(ctx: &egui::Context, message: &str, via: FeedbackVia) -> String {
    let mail = compose(
        message,
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        via,
    );
    if let Some(text) = mail.clipboard {
        ctx.copy_text(text);
    }
    crate::shell::open_url(&mail.url);
    mail.note.to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_message_travels_in_the_link() {
        let mail = compose(
            "The bubble is too small",
            "0.2.2",
            "linux",
            FeedbackVia::MailApp,
        );
        assert!(mail.clipboard.is_none());
        assert!(
            mail.url
                .starts_with("mailto:pelamx@bubbletranslate.app?subject=")
        );
        assert!(mail.url.contains("&body="));
        // Spaces and the em dash have to survive as encoding, not as breaks.
        assert!(!mail.url.contains(' '));
        assert!(mail.url.contains("The%20bubble%20is%20too%20small"));
    }

    #[test]
    fn the_version_and_system_are_in_the_subject_and_the_body() {
        let mail = compose("hi", "9.9.9", "windows", FeedbackVia::MailApp);
        assert!(mail.url.contains("9.9.9"));
        assert!(mail.url.contains("windows"));
    }

    #[test]
    fn webmail_opens_a_compose_page_rather_than_a_mailto_link() {
        for (via, host) in [
            (FeedbackVia::Gmail, "mail.google.com"),
            (FeedbackVia::Outlook, "outlook.live.com"),
        ] {
            let mail = compose("the bubble is too small", "0.2.2", "linux", via);
            assert!(mail.url.starts_with("https://"), "{}", mail.url);
            assert!(mail.url.contains(host), "{}", mail.url);
            assert!(mail.url.contains("pelamx@bubbletranslate.app"));
            assert!(mail.url.contains("the%20bubble%20is%20too%20small"));
        }
    }

    #[test]
    fn webmail_carries_a_long_message_in_the_link() {
        // The budget is a mailto: limit, not a URL one: a browser takes this.
        let long = "a".repeat(MAILTO_BUDGET + 1);
        let mail = compose(&long, "0.2.2", "linux", FeedbackVia::Gmail);
        assert!(mail.clipboard.is_none());
        assert!(mail.url.contains(&long));
    }

    #[test]
    fn a_long_message_goes_to_the_clipboard_rather_than_being_cut() {
        let long = "a".repeat(MAILTO_BUDGET + 1);
        let mail = compose(&long, "0.2.2", "linux", FeedbackVia::MailApp);
        // The whole of it, kept.
        let kept = mail.clipboard.expect("a long message is not thrown away");
        assert!(kept.starts_with(&long));
        // And the link carries no body to be truncated.
        assert!(!mail.url.contains("&body="));
    }

    #[test]
    fn encoding_is_measured_rather_than_the_raw_length() {
        // Every character here is three bytes once encoded, so a message well
        // under the budget in characters is over it in a link.
        let cyrillic = "\u{434}".repeat(MAILTO_BUDGET / 4);
        assert!(cyrillic.chars().count() < MAILTO_BUDGET);
        assert!(
            compose(&cyrillic, "0.2.2", "linux", FeedbackVia::MailApp)
                .clipboard
                .is_some()
        );
    }
}
