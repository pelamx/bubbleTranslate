//! What the bubble draws inside its card: the translation, the footer and the
//! inline settings.

use super::*;

impl BubbleApp {
    /// Draws the bubble contents. Returns true when the user asked to close.
    pub(super) fn draw_body(&mut self, ui: &mut egui::Ui) -> bool {
        let mut dismiss = false;

        // -- header: name, byline and the close button --------------------
        ui.horizontal_top(|ui| {
            ui.label(
                egui::RichText::new("bubbleTranslate")
                    .size(11.5)
                    .color(pal().text_secondary),
            );
            // Right to left, so the close button takes the corner and the
            // byline sits just inside it.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new(CLOSE_GLYPH).size(17.0)).frame(false),
                    )
                    .on_hover_text(t("Close", "Kapat", "Cerrar"))
                    .clicked()
                {
                    dismiss = true;
                }
                ui.label(
                    egui::RichText::new("by pelamx")
                        .size(11.5)
                        .color(pal().text_muted),
                );
            });
        });

        ui.add_space(6.0);

        // -- body ---------------------------------------------------------
        match &self.state {
            State::Hidden => {}
            State::Working { via, .. } => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        egui::RichText::new(match via {
                            CaptureSource::Accessibility | CaptureSource::PrimarySelection => {
                                t("Translating…", "Çevriliyor…", "Traduciendo…")
                            }
                            CaptureSource::Clipboard => t(
                                "Translating (via copy)…",
                                "Çevriliyor (kopyalama ile)…",
                                "Traduciendo (vía copia)…",
                            ),
                            // Said plainly, because this is the one source
                            // that can misread: it guessed the letters off
                            // the pixels rather than being handed them, and
                            // somebody comparing the bubble with the screen
                            // should know which of the two happened.
                            CaptureSource::Ocr => t(
                                "Translating (read from screen)…",
                                "Çevriliyor (ekrandan okundu)…",
                                "Traduciendo (leído de la pantalla)…",
                            ),
                        })
                        .size(13.5)
                        .color(pal().text_secondary),
                    );
                });
            }
            State::Done { result, .. } => {
                let size = self.config.lock().unwrap().font_size;
                egui::ScrollArea::vertical()
                    .max_height(MAX_HEIGHT - 100.0)
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(&result.text)
                                .size(size)
                                .line_height(Some(size * LINE_HEIGHT_RATIO))
                                .color(pal().text_primary),
                        );
                    });
            }
            State::Capped { limit } => {
                let limit = *limit;
                ui.label(
                    egui::RichText::new(match crate::i18n::lang() {
                        crate::i18n::UiLang::En => {
                            format!("You have used today's {limit} free translations")
                        }
                        crate::i18n::UiLang::Tr => {
                            format!("Bugünkü {limit} ücretsiz çeviriyi kullandın")
                        }
                        crate::i18n::UiLang::Es => {
                            format!("Ya usaste las {limit} traducciones gratis de hoy")
                        }
                    })
                    .size(14.0)
                    .color(pal().text_primary),
                );
                ui.add_space(3.0);
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
                    .size(12.0)
                    .color(pal().text_muted),
                );
                // The button is on every refusal, not one an hour. "Not now"
                // is a real answer against a daily allowance, but a bubble
                // that states the problem while hiding the one control that
                // solves it is a dead end wearing an explanation — and the
                // "Not now" beside it is what keeps this from being a nag.
                ui.add_space(9.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(t("Upgrade to Pro", "Pro'ya geç", "Pasar a Pro"))
                        .clicked()
                    {
                        shell::open_url(&format!("{}?src=bubble", license::BUY_URL));
                        dismiss = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(t("Not now", "Şimdi değil", "Ahora no"))
                                    .size(12.0),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        dismiss = true;
                    }
                });
            }
            State::Failed { errors, .. } => {
                ui.label(
                    egui::RichText::new(t(
                        "No provider could translate this",
                        "Hiçbir servis bunu çeviremedi",
                        "Ningún proveedor pudo traducir esto",
                    ))
                    .size(14.0)
                    .color(pal().text_error),
                );
                ui.add_space(2.0);
                for (provider, err) in errors {
                    ui.label(
                        egui::RichText::new(format!("{}: {err}", provider.label()))
                            .size(12.0)
                            .color(pal().text_muted),
                    );
                }
            }
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        dismiss |= self.draw_footer(ui);

        if self.settings_open {
            ui.add_space(6.0);
            self.draw_settings(ui);
        } else {
            self.lang_popup_open = false;
        }

        dismiss
    }

    pub(super) fn draw_footer(&mut self, ui: &mut egui::Ui) -> bool {
        let dismiss = false;
        let target = self.target_lang();

        ui.horizontal(|ui| {
            // Provider and detected source language: says which of the three
            // backends actually answered, which matters when the chain fell
            // through to a fallback.
            let caption = match &self.state {
                State::Done { result, .. } => format!(
                    "{} · {} → {}",
                    result.provider.label(),
                    language_name(&result.source_lang),
                    language_name(&target),
                ),
                _ => format!("→ {}", language_name(&target)),
            };
            ui.label(
                egui::RichText::new(caption)
                    .size(11.5)
                    .color(pal().text_muted),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(egui::Button::new(egui::RichText::new("⚙").size(14.0)).frame(false))
                    .on_hover_text(t("Settings", "Ayarlar", "Ajustes"))
                    .clicked()
                {
                    self.settings_open = !self.settings_open;
                }

                if let State::Done { result, .. } = &self.state {
                    let just_copied = self
                        .copied_at
                        .is_some_and(|t| t.elapsed() < Duration::from_secs(2));
                    let label = if just_copied {
                        t("Copied", "Kopyalandı", "Copiado")
                    } else {
                        t("Copy", "Kopyala", "Copiar")
                    };
                    if ui
                        .add(egui::Button::new(egui::RichText::new(label).size(12.0)).frame(false))
                        .clicked()
                    {
                        capture::set_clipboard(ui.ctx(), &result.text);
                        self.copied_at = Some(Instant::now());
                    }
                }

                // Language switching lives in the settings panel rather than a
                // dropdown here: the bubble never becomes the active window, by
                // design, and a popup menu in a window that cannot take focus
                // does not open.
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new(language_name(&target)).size(12.0))
                            .frame(false),
                    )
                    .on_hover_text(t(
                        "Change target language",
                        "Hedef dili değiştir",
                        "Cambiar idioma de destino",
                    ))
                    .clicked()
                {
                    self.settings_open = !self.settings_open;
                }
            });
        });

        dismiss
    }

    pub(super) fn draw_settings(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        let mut retranslate = false;
        let mut retheme = false;
        let mut cfg = self.config.lock().unwrap();
        let mut dirty = false;

        if let Some(warning) = &self.readiness_warning {
            ui.label(
                egui::RichText::new(warning)
                    .size(11.5)
                    .color(egui::Color32::from_rgb(245, 195, 130)),
            );
            ui.add_space(4.0);
        }

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(t(
                    "Providers, in order",
                    "Servisler, sırayla",
                    "Proveedores, en orden",
                ))
                .size(12.0),
            );
            ui.label(
                egui::RichText::new(
                    cfg.providers
                        .iter()
                        .map(|p| p.label())
                        .collect::<Vec<_>>()
                        .join(" → "),
                )
                .size(12.0)
                .color(pal().text_secondary),
            );
        });

        if ui
            .checkbox(
                &mut cfg.auto_translate,
                egui::RichText::new(t(
                    "Translate on selection",
                    "Seçince çevir",
                    "Traducir al seleccionar",
                ))
                .size(12.0),
            )
            .changed()
        {
            dirty = true;
        }

        // The same choice as the main window's, kept here because this panel
        // is the one within reach the moment a bubble appears somewhere it was
        // not wanted — which is exactly when someone goes looking for it.
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(t(
                    "Hold while selecting",
                    "Seçerken basılı tut",
                    "Mantener al seleccionar",
                ))
                .size(12.0)
                .color(pal().text_secondary),
            );
            egui::ComboBox::from_id_salt("bubble-trigger-key")
                .selected_text(egui::RichText::new(cfg.trigger_key.label()).size(12.0))
                .width(150.0)
                .height(LANG_POPUP_HEIGHT)
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
        });

        // The bubble's colour scheme, applied the instant it is picked: the
        // palette is swapped here and the panel below repaints in the new
        // colours without the bubble being reopened. See [`set_palette`].
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(t("Theme", "Tema", "Tema"))
                    .size(12.0)
                    .color(pal().text_secondary),
            );
            egui::ComboBox::from_id_salt("bubble-theme")
                .selected_text(egui::RichText::new(cfg.theme.label()).size(12.0))
                .width(150.0)
                .height(LANG_POPUP_HEIGHT)
                .show_ui(ui, |ui| {
                    for theme in BubbleTheme::ALL {
                        if ui
                            .selectable_label(*theme == cfg.theme, theme.label())
                            .clicked()
                        {
                            cfg.theme = *theme;
                            set_palette(*theme);
                            dirty = true;
                            retheme = true;
                        }
                    }
                });
        });

        let mut chosen: Option<String> = None;
        let mut popup_open = false;

        // A dropdown rather than the whole list laid out flat. Seventeen
        // languages wrapped across a 324pt bubble is a paragraph of names to
        // read past every time the panel opens, and it pushes everything below
        // it out of reach. The popup is drawn by egui inside this same
        // viewport — it is not a platform menu — which is what lets a window
        // that never takes focus have one at all. Its height is capped so the
        // list scrolls instead of running off the bottom of a short bubble.
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(t("Translate into", "Hedef dil", "Traducir a"))
                    .size(12.0)
                    .color(pal().text_secondary),
            );
            let current = cfg.target_lang.clone();
            let mut picked: Option<String> = None;
            let combo = egui::ComboBox::from_id_salt("bubble-target")
                .selected_text(egui::RichText::new(language_name(&current)).size(12.0))
                .width(150.0)
                .height(LANG_POPUP_HEIGHT)
                .show_ui(ui, |ui| {
                    for (code, name) in LANGUAGES {
                        if ui.selectable_label(*code == current, *name).clicked() {
                            picked = Some((*code).to_string());
                        }
                    }
                });
            // `inner` is `Some` only on the frames the list is actually shown.
            popup_open = combo.inner.is_some();
            chosen = picked;
        });
        if let Some(code) = chosen
            && code != cfg.target_lang
        {
            cfg.target_lang = code;
            dirty = true;
            retranslate = true;
        }
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(t("Text size", "Yazı boyutu", "Tamaño del texto")).size(12.0),
            );
            if ui
                .add(
                    egui::Slider::new(&mut cfg.font_size, 12.0..=26.0)
                        .show_value(false)
                        .trailing_fill(true),
                )
                .drag_stopped()
            {
                dirty = true;
            }
            ui.label(
                egui::RichText::new(format!("{:.0}pt", cfg.font_size))
                    .size(11.0)
                    .color(pal().text_muted),
            );
        });

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(t("DeepL key", "DeepL anahtarı", "Clave de DeepL")).size(12.0),
            );
            if ui
                .add(
                    egui::TextEdit::singleline(&mut cfg.deepl_api_key)
                        .password(true)
                        .hint_text(t("optional", "isteğe bağlı", "opcional"))
                        .desired_width(180.0),
                )
                .lost_focus()
            {
                dirty = true;
            }
        });

        ui.label(
            egui::RichText::new(format!(
                "{} {}",
                t("Config:", "Ayarlar:", "Ajustes:"),
                Config::path().display()
            ))
            .size(10.5)
            .color(pal().text_muted),
        );

        self.lang_popup_open = popup_open;

        if dirty {
            let _ = cfg.save();
        }
        // The config lock must be released before asking the engine to redo the
        // translation, or the engine thread blocks on it.
        drop(cfg);
        // The palette was already swapped at the click; this re-derives egui's
        // own widget colours — button fills, the field well, the selection —
        // from it, so the whole bubble turns over at once rather than the text
        // changing now and the controls at the next reopen.
        if retheme {
            install_theme(ui.ctx());
            ui.ctx().request_repaint();
        }
        if retranslate {
            self.engine.request(Request::Retranslate);
        }
    }
}
