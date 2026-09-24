//! The section that explains reading text off the screen.
//!
//! Selecting is the gesture everyone already knows; drawing a box to read a
//! picture is not, and a keyboard shortcut nobody told you about does not
//! exist as far as you are concerned. So the window says what it is for, how
//! to do it, and — on Linux, where the reader is fetched on first use and
//! Tesseract is an option for more alphabets — whether it is ready, and what
//! to do when it is not.

use eframe::egui;

use super::{ERR_RED, OK_GREEN, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY, WARN_AMBER};
use crate::config::Config;
use crate::engine::Request;
use crate::i18n::t;
use crate::platform::BuiltinReader;

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
    // Only a download under way holds the button back: with the models not
    // fetched yet, trying it is what fetches them.
    let ready = status
        .as_ref()
        .is_none_or(|status| !matches!(status.builtin, BuiltinReader::Downloading(_)));

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

/// Which reader is ready and what it reads, how the built-in one is fetched,
/// and how to install Tesseract for what it does not read.
fn linux_status(ui: &mut egui::Ui, status: &crate::platform::ScreenReading) {
    let tesseract = !status.languages.is_empty();
    if tesseract {
        ui.label(
            egui::RichText::new(format!(
                "{} {}",
                t(
                    "● Ready — Tesseract reads",
                    "● Hazır — Tesseract'ın okuduğu diller:",
                    "● Listo — Tesseract lee"
                ),
                status.languages.join(", ")
            ))
            .size(12.5)
            .color(OK_GREEN),
        );
    } else {
        builtin_status(ui, &status.builtin);
    }

    // Tesseract is never required; it is offered for what the built-in
    // reader cannot do, and for the source language when its pack is not
    // there.
    if tesseract && status.missing_pack.is_none() {
        return key_note(ui, status);
    }
    ui.add_space(8.0);
    if tesseract {
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
    } else {
        ui.label(
            egui::RichText::new(t(
                "For it to read properly, install Tesseract",
                "Düzgün okuması için Tesseract'ı kur",
                "Para que lea bien, instala Tesseract",
            ))
            .size(12.5)
            .color(TEXT_PRIMARY)
            .strong(),
        );
        ui.label(
            egui::RichText::new(t(
                "The built-in reader only knows letters without accents: ç, ğ, ş, é, ü \
                 come out as plain c, g, s, e, u, and Cyrillic, Arabic, Chinese or \
                 Japanese text is not read at all. A word missing its letters can \
                 translate as a different word. Tesseract, with a pack for the \
                 language you read from, reads every letter as it is — and once it \
                 is installed, it does the reading instead.",
                "Dahili okuyucu yalnızca aksansız harfleri tanır: ç, ğ, ş, é, ü düz c, \
                 g, s, e, u olarak okunur; Kiril, Arapça, Çince ya da Japonca metin \
                 hiç okunmaz. Harfi eksik bir kelime başka bir kelime gibi çevrilebilir. \
                 Tesseract, okuduğun dilin paketiyle her harfi olduğu gibi okur ve \
                 kurulduğunda okumayı o yapar.",
                "El lector integrado solo conoce letras sin acento: ç, ğ, ş, é, ü salen \
                 como c, g, s, e, u, y el texto en cirílico, árabe, chino o japonés no \
                 se lee. Una palabra sin sus letras puede traducirse como otra. \
                 Tesseract, con el paquete del idioma que lees, lee cada letra tal como \
                 es, y una vez instalado es él quien lee.",
            ))
            .size(12.0)
            .color(TEXT_SECONDARY),
        );
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

    key_note(ui, status);
}

/// Where the built-in reader stands, and the button that fetches it.
fn builtin_status(ui: &mut egui::Ui, builtin: &BuiltinReader) {
    let note = |ui: &mut egui::Ui| {
        ui.label(
            egui::RichText::new(t(
                "It reads Latin letters without accents, digits and punctuation.",
                "Aksansız Latin harflerini, rakamları ve noktalama işaretlerini okur.",
                "Lee letras latinas sin acento, cifras y signos de puntuación.",
            ))
            .size(12.0)
            .color(TEXT_SECONDARY),
        );
    };
    match builtin {
        BuiltinReader::Ready => {
            ui.label(
                egui::RichText::new(t(
                    "● Ready — built-in reader",
                    "● Hazır — dahili okuyucu",
                    "● Listo — lector integrado",
                ))
                .size(12.5)
                .color(OK_GREEN),
            );
            note(ui);
        }
        BuiltinReader::Absent => {
            ui.label(
                egui::RichText::new(t(
                    "● The built-in reader is downloaded the first time you use it (12 MB)",
                    "● Dahili okuyucu ilk kullanımda indirilir (12 MB)",
                    "● El lector integrado se descarga la primera vez que lo usas (12 MB)",
                ))
                .size(12.5)
                .color(WARN_AMBER),
            );
            note(ui);
            ui.add_space(4.0);
            if ui
                .button(t("Download now", "Şimdi indir", "Descargar ahora"))
                .clicked()
            {
                crate::platform::download_screen_reader();
            }
        }
        BuiltinReader::Downloading(done) => {
            ui.label(
                egui::RichText::new(t(
                    "● Downloading the built-in reader…",
                    "● Dahili okuyucu indiriliyor…",
                    "● Descargando el lector integrado…",
                ))
                .size(12.5)
                .color(WARN_AMBER),
            );
            ui.add(egui::ProgressBar::new(*done).show_percentage());
            // Nothing else wakes the window while the bar moves.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }
        BuiltinReader::Failed(why) => {
            ui.label(
                egui::RichText::new(t(
                    "● The built-in reader could not be downloaded",
                    "● Dahili okuyucu indirilemedi",
                    "● No se pudo descargar el lector integrado",
                ))
                .size(12.5)
                .color(ERR_RED),
            );
            ui.label(egui::RichText::new(why).size(11.0).color(TEXT_MUTED));
            ui.add_space(4.0);
            if ui
                .button(t("Try again", "Tekrar dene", "Reintentar"))
                .clicked()
            {
                crate::platform::download_screen_reader();
            }
        }
    }
}

/// On a session that does not pass the key on, the command to bind instead.
fn key_note(ui: &mut egui::Ui, status: &crate::platform::ScreenReading) {
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
