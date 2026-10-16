//! "How to use it", at the top of the window: the selection gesture and Esc,
//! said once where anyone opening the window will see it. The screen-reading
//! shortcuts have their own section further down, and are not repeated here.
//!
//! The app shows nothing until it is asked, so the window is the only place
//! the gesture can be learnt. The selection key is the one chosen in the
//! settings, named the way this keyboard names it.

use eframe::egui;

use crate::config::{Config, TriggerKey};
use crate::i18n::t;

use super::screen::key_chip;
use super::{TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY};

/// One gesture: the keys, and what they do.
fn row(ui: &mut egui::Ui, keys: &[&str], what: &str) {
    ui.horizontal_wrapped(|ui| {
        for (n, key) in keys.iter().enumerate() {
            if n > 0 {
                ui.label(egui::RichText::new("+").size(12.0).color(TEXT_MUTED));
            }
            key_chip(ui, key);
        }
        ui.label(egui::RichText::new(what).size(12.5).color(TEXT_PRIMARY));
    });
    ui.add_space(4.0);
}

fn tip(ui: &mut egui::Ui, text: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("•").size(12.0).color(TEXT_MUTED));
        ui.label(egui::RichText::new(text).size(12.0).color(TEXT_SECONDARY));
    });
}

pub fn how_to(ui: &mut egui::Ui, cfg: &Config) {
    let key = cfg.trigger_key.label();
    let select = t(
        "select text, or double-click a word — the translation appears next to the pointer",
        "metin seç ya da bir kelimeye çift tıkla — çeviri imlecin yanında çıkar",
        "selecciona texto o haz doble clic en una palabra — la traducción aparece junto al puntero",
    );
    if cfg.trigger_key == TriggerKey::Always {
        row(ui, &[t("Select", "Seç", "Selecciona")], select);
    } else {
        row(ui, &[key], select);
    }
    row(
        ui,
        &["Esc"],
        t(
            "put the bubble or the translated screen away",
            "baloncuğu ya da çevrilmiş ekranı kapat",
            "cierra la burbuja o la pantalla traducida",
        ),
    );

    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(t("Tips", "İpuçları", "Consejos"))
            .size(12.5)
            .color(TEXT_PRIMARY)
            .strong(),
    );
    ui.add_space(2.0);
    tip(
        ui,
        t(
            "In the bubble: Copy takes the translation, Listen reads it aloud, and ⚙ changes \
             the language it translates into — the same words are translated again at no \
             cost.",
            "Baloncukta: Kopyala çeviriyi alır, Dinle sesli okur, ⚙ çeviri dilini değiştirir — \
             aynı metin yeniden ücretsiz çevrilir.",
            "En la burbuja: Copiar copia la traducción, Escuchar la lee en voz alta y ⚙ cambia \
             el idioma de destino — el mismo texto se vuelve a traducir sin coste.",
        ),
    );
    tip(
        ui,
        t(
            "Holding the key keeps everyday selecting and copying untouched; choose another \
             key, or any selection, under Behaviour.",
            "Tuşa basılı tutma şartı, günlük seçme ve kopyalamaya dokunmaz; başka bir tuşu ya da \
             her seçimi Davranış bölümünden seçebilirsin.",
            "Mantener la tecla deja intactos el seleccionar y copiar de siempre; elige otra \
             tecla, o cualquier selección, en Comportamiento.",
        ),
    );
    tip(
        ui,
        t(
            "The language you read from is detected by itself. If text is already in your \
             language, it is translated into your second language instead.",
            "Okuduğun dil kendiliğinden algılanır. Metin zaten senin dilindeyse, ikinci diline \
             çevrilir.",
            "El idioma de origen se detecta solo. Si el texto ya está en tu idioma, se traduce a \
             tu segundo idioma.",
        ),
    );
    tip(
        ui,
        t(
            "Free: 10 text translations and 3 screen translations a day, back at midnight. Pro \
             has no limit.",
            "Ücretsiz: günde 10 metin ve 3 ekran çevirisi, gece yarısı yenilenir. Pro'da sınır \
             yok.",
            "Gratis: 10 traducciones de texto y 3 de pantalla al día, se renuevan a medianoche. \
             Pro no tiene límite.",
        ),
    );
    tip(
        ui,
        t(
            "Closing this window leaves bubbleTranslate running; it lives in the tray, and \
             opening it again brings the window back.",
            "Bu pencereyi kapatmak bubbleTranslate'i kapatmaz; sistem tepsisinde çalışır, \
             yeniden açınca pencere geri gelir.",
            "Cerrar esta ventana deja bubbleTranslate en marcha; vive en la bandeja, y abrirla \
             de nuevo trae la ventana.",
        ),
    );
}
