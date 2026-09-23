//! The settings sections: languages, providers and behaviour.

use super::*;

pub(super) fn languages(ui: &mut egui::Ui, state: &mut MainState, cfg: &mut Config) -> bool {
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
        if let Some(code) = picked
            && code != cfg.target_lang
        {
            cfg.target_lang = code;
            dirty = true;
            // Apply the change to the text already captured, so the bubble
            // updates without needing a fresh selection.
            let _ = state.requests.send(Request::Retranslate);
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
        if let Some(code) = picked
            && code != cfg.source_lang
        {
            cfg.source_lang = code;
            dirty = true;
        }
    });

    dirty
}

pub(super) fn providers(ui: &mut egui::Ui, state: &mut MainState, cfg: &mut Config) -> bool {
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

pub(super) fn behaviour(ui: &mut egui::Ui, cfg: &mut Config) -> bool {
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

pub(super) fn slider(
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
