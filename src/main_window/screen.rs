//! The section that explains reading text off the screen.
//!
//! Selecting is the gesture everyone already knows; drawing a box to read a
//! picture is not, and a keyboard shortcut nobody told you about does not
//! exist as far as you are concerned. So the window says what it is for, how
//! to do it, and — on Linux, where the reading engine is something the user
//! installs — whether it is ready, and the one command that makes it so.

use eframe::egui;

use super::{ERR_RED, OK_GREEN, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY, WARN_AMBER};
use crate::config::Config;
use crate::engine::Request;
use crate::i18n::t;

use super::MainState;

/// The key, as it is written on this system's keyboard.
fn shortcut() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘⇧E"
    } else {
        "Ctrl+Shift+E"
    }
}

pub fn screen_reading(ui: &mut egui::Ui, state: &mut MainState, cfg: &Config) {
    ui.horizontal_wrapped(|ui| {
        key_chip(ui, shortcut());
        ui.label(
            egui::RichText::new(t(
                "Translate text you cannot select",
                "Seçilemeyen metni çevir",
                "Traduce texto que no se puede seleccionar",
            ))
            .size(13.0)
            .color(TEXT_PRIMARY)
            .strong(),
        );
    });
    ui.add_space(6.0);

    let steps: [&str; 3] = [
        match shortcut() {
            "⌘⇧E" => t(
                "Press ⌘⇧E — the screen dims.",
                "⌘⇧E'ye bas — ekran kararır.",
                "Pulsa ⌘⇧E — la pantalla se oscurece.",
            ),
            _ => t(
                "Press Ctrl+Shift+E — the screen dims.",
                "Ctrl+Shift+E'ye bas — ekran kararır.",
                "Pulsa Ctrl+Shift+E — la pantalla se oscurece.",
            ),
        },
        t(
            "Drag a box over the text: a picture, a video, a scanned page, a game.",
            "Yazının üzerine bir kutu çiz: bir resim, video, taranmış sayfa ya da oyun.",
            "Arrastra un recuadro sobre el texto: una imagen, un vídeo, una página \
             escaneada, un juego.",
        ),
        t(
            "The translation appears under the box. Esc or the right button cancels.",
            "Çeviri kutunun altında çıkar. Esc ya da sağ tık iptal eder.",
            "La traducción aparece bajo el recuadro. Esc o el botón derecho cancela.",
        ),
    ];
    for (n, step) in steps.iter().enumerate() {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(format!("{}.", n + 1))
                    .size(12.0)
                    .color(TEXT_MUTED),
            );
            ui.label(egui::RichText::new(*step).size(12.0).color(TEXT_SECONDARY));
        });
    }
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(t(
            "The picture is read on this computer and never sent anywhere.",
            "Görüntü bu bilgisayarda okunur ve hiçbir yere gönderilmez.",
            "La imagen se lee en este ordenador y nunca se envía a ningún sitio.",
        ))
        .size(11.0)
        .color(TEXT_MUTED),
    );

    let status = crate::platform::screen_reading(&cfg.source_lang);
    let ready = status
        .as_ref()
        .is_none_or(|status| !status.languages.is_empty());

    ui.add_space(8.0);
    if ui
        .add_enabled(
            ready,
            egui::Button::new(t("Try it now", "Şimdi dene", "Probar ahora")),
        )
        .clicked()
    {
        let _ = state.requests.send(Request::ScreenRegion);
    }

    if let Some(status) = status {
        ui.add_space(10.0);
        linux_status(ui, &status);
    }
}

/// Whether Tesseract is ready, what it reads, and how to install what is not
/// there.
fn linux_status(ui: &mut egui::Ui, status: &crate::platform::ScreenReading) {
    if !status.engine {
        ui.label(
            egui::RichText::new(t(
                "● Needs Tesseract, which does the reading",
                "● Okumayı yapan Tesseract gerekiyor",
                "● Necesita Tesseract, que hace la lectura",
            ))
            .size(12.5)
            .color(ERR_RED),
        );
        ui.label(
            egui::RichText::new(t(
                "Install it once — it stays through every update. Selecting text to \
                 translate does not need it.",
                "Bir kez kurman yeterli, her güncellemede yerinde kalır. Metin seçip \
                 çevirmek için gerekmez.",
                "Instálalo una vez; se mantiene en cada actualización. Traducir texto \
                 seleccionado no lo necesita.",
            ))
            .size(12.0)
            .color(TEXT_SECONDARY),
        );
    } else if status.languages.is_empty() {
        ui.label(
            egui::RichText::new(t(
                "● Tesseract has no languages yet",
                "● Tesseract'ta henüz dil yok",
                "● Tesseract aún no tiene idiomas",
            ))
            .size(12.5)
            .color(ERR_RED),
        );
    } else {
        ui.label(
            egui::RichText::new(format!(
                "{} {}",
                t(
                    "● Ready — reads",
                    "● Hazır — okuduğu diller:",
                    "● Listo — lee"
                ),
                status.languages.join(", ")
            ))
            .size(12.5)
            .color(OK_GREEN),
        );
        if let Some(pack) = status.missing_pack {
            ui.label(
                egui::RichText::new(format!(
                    "{} ({pack})",
                    t(
                        "Your source language is not among them yet",
                        "Kaynak dilin henüz aralarında yok",
                        "Tu idioma de origen aún no está entre ellos",
                    )
                ))
                .size(12.0)
                .color(WARN_AMBER),
            );
        }
    }

    match (&status.install, status.missing_pack) {
        (Some(command), _) => {
            ui.add_space(4.0);
            command_box(ui, command);
            ui.label(
                egui::RichText::new(t(
                    "Run it in a terminal; this window notices by itself.",
                    "Bir terminalde çalıştır; bu pencere kendiliğinden fark eder.",
                    "Ejecútalo en una terminal; esta ventana lo detecta sola.",
                ))
                .size(11.0)
                .color(TEXT_MUTED),
            );
        }
        (None, Some(pack)) => {
            ui.label(
                egui::RichText::new(format!(
                    "{} tesseract + {pack}",
                    t(
                        "Install from your distribution's packages:",
                        "Dağıtımının paketlerinden kur:",
                        "Instala desde los paquetes de tu distribución:",
                    )
                ))
                .size(12.0)
                .color(TEXT_SECONDARY),
            );
        }
        (None, None) => {}
    }

    if !status.key_heard {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(t(
                "This session does not pass Ctrl+Shift+E on to the app. Bind this command \
                 to a shortcut in your desktop's keyboard settings instead:",
                "Bu oturum Ctrl+Shift+E'yi uygulamaya iletmiyor. Bunun yerine bu komutu \
                 masaüstünün klavye ayarlarında bir kısayola bağla:",
                "Esta sesión no pasa Ctrl+Shift+E a la app. Asigna este comando a un \
                 atajo en los ajustes de teclado de tu escritorio:",
            ))
            .size(12.0)
            .color(WARN_AMBER),
        );
        command_box(ui, "bubbleTranslate --read-screen");
    }
}

/// A key combination, drawn as a key cap.
fn key_chip(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(egui::Color32::from_rgb(56, 58, 66))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(80)))
        .corner_radius(5.0)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .monospace()
                    .size(12.0)
                    .color(TEXT_PRIMARY),
            );
        });
}

/// A command to run, with a button that copies it.
fn command_box(ui: &mut egui::Ui, command: &str) {
    egui::Frame::new()
        .fill(egui::Color32::from_rgb(24, 25, 28))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(command)
                        .monospace()
                        .size(11.5)
                        .color(egui::Color32::from_rgb(130, 220, 170)),
                );
                if ui.small_button(t("Copy", "Kopyala", "Copiar")).clicked() {
                    ui.ctx().copy_text(command.to_string());
                }
            });
        });
}
