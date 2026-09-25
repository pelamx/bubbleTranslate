//! The licence panel.

use super::*;

/// The licence panel: what this install may do, and how to change it.
///
/// Deliberately the plainest section in the window. Everything here is either
/// a fact about the account or a button that opens a browser — the app itself
/// never asks for a card, an address, or an email.
pub(super) fn account(
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
    let (used, limit, left) = {
        let mut quota = licensing.quota.lock().unwrap();
        (
            quota.used_today(),
            quota.limit(&entitlement),
            quota.remaining(&entitlement),
        )
    };

    // -- where this install stands ----------------------------------------
    match limit {
        None => {
            let cycle = entitlement.cycle;
            ui.label(
                egui::RichText::new(match cycle {
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
                })
                .size(13.0)
                .color(OK_GREEN),
            );
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
                license::open_buy("window");
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
