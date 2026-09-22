//! Sending feedback by mail.

use super::*;

/// Where to write. The same address the website gives, so a reply from it is
/// recognisable rather than a stranger's.
pub(super) const SUPPORT_EMAIL: &str = "pelamx@bubbletranslate.app";

/// How much of a message a `mailto:` link can carry before the system that
/// opens it starts truncating or refusing. Windows is the strict one, at
/// roughly two thousand characters for the whole command; this leaves room for
/// the address, the subject and the encoding, which triples the length of
/// anything that is not plain ASCII.
pub(super) const MAILTO_BUDGET: usize = 1200;

/// Somewhere to say what is wrong, without leaving the app to find out where.
///
/// The message travels through the user's own mail program rather than a form
/// this app posts somewhere: bubbleTranslate keeps no account and no database,
/// and a complaint box that quietly shipped text to a server of ours would be
/// the one place that stopped being true. It also means the sender keeps a
/// copy in their sent mail and a reply lands where they expect it.
pub(super) fn feedback(ui: &mut egui::Ui, state: &mut MainState, cfg: &mut Config) -> bool {
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
pub(super) struct Mail {
    url: String,
    clipboard: Option<String>,
    note: &'static str,
}

/// Works out the above, and nothing else \u{2014} no windows opened, no clipboard
/// touched \u{2014} so the rules below can be tested rather than trusted.
pub(super) fn compose(message: &str, version: &str, os: &str, via: FeedbackVia) -> Mail {
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
pub(super) fn send_feedback(ctx: &egui::Context, message: &str, via: FeedbackVia) -> String {
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
