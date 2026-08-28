//! The floating bubble.
//!
//! One borderless, always-on-top viewport that spends most of its life hidden.
//! When a translation starts it moves to the cursor and shows itself; it never
//! takes keyboard focus, so the app you were reading stays frontmost and its
//! selection stays intact.

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::capture;
use crate::config::{Config, LANGUAGES, Provider, language_name};
use crate::engine::{Engine, Request, UiEvent};
use crate::main_window::{self, MainState};
use crate::monitor;
use crate::platform::CaptureSource;
use crate::shell;
use crate::translate::{TranslateError, Translation};

pub const BUBBLE_WIDTH: f32 = 400.0;
const MIN_HEIGHT: f32 = 90.0;
const MAX_HEIGHT: f32 = 460.0;
/// How tall the language list is allowed to get. Also the room the bubble
/// makes for it while it is open — the two have to agree, or the list is
/// either clipped or floating over empty space.
const LANG_POPUP_HEIGHT: f32 = 200.0;
/// Offset from the pointer so the bubble never lands under the cursor itself.
const CURSOR_OFFSET: (f32, f32) = (14.0, 20.0);

/// How often a visible bubble re-checks whether the pointer is over it.
const HOVER_POLL: Duration = Duration::from_millis(150);

/// The close button's glyph.
///
/// A multiplication sign rather than one of the several nicer-looking crosses
/// in the symbol blocks, because it lives in Latin-1 and is therefore in every
/// font that exists. The prettier ✕ (U+2715) is absent from the fonts most
/// Linux distributions ship, and an absent glyph does not degrade — it draws a
/// hollow box, which reads as a broken button rather than a close one.
const CLOSE_GLYPH: &str = "×";

/// Bubble palette.
///
/// The translation is the one thing worth reading, so it gets near-white on a
/// deliberately dark panel; everything else is chrome and steps down from
/// there. These are all well above the dim greys that make small text on a
/// dark background hard to read.
const TEXT_PRIMARY: egui::Color32 = egui::Color32::from_gray(240);
const TEXT_SECONDARY: egui::Color32 = egui::Color32::from_gray(186);
const TEXT_MUTED: egui::Color32 = egui::Color32::from_gray(158);
const BUBBLE_BG: egui::Color32 = egui::Color32::from_rgb(30, 31, 34);
const BUBBLE_BORDER: egui::Color32 = egui::Color32::from_gray(88);
const TEXT_ERROR: egui::Color32 = egui::Color32::from_rgb(255, 150, 150);

/// Multiplier on the font size for line spacing. Translated paragraphs are
/// often long sentences with no visual breaks, and the extra leading is what
/// makes them scannable.
const LINE_HEIGHT_RATIO: f32 = 1.45;

/// Fonts with broad script coverage, tried in order, so Chinese, Japanese,
/// Korean, Arabic and Cyrillic output renders instead of showing
/// missing-glyph boxes. egui's bundled fonts are Latin-only.
///
/// macOS ships one font that covers nearly everything. Linux distributions
/// split the same coverage across several Noto families and put them wherever
/// they like, so the list is longer and the search is by name as well as by
/// path — see [`find_fallback_font`].
#[cfg(target_os = "macos")]
const FALLBACK_FONTS: &[&str] = &["/System/Library/Fonts/Supplemental/Arial Unicode.ttf"];

#[cfg(target_os = "linux")]
const FALLBACK_FONTS: &[&str] = &[
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
];

enum State {
    Hidden,
    Working {
        via: CaptureSource,
    },
    Done {
        result: Translation,
    },
    Failed {
        errors: Vec<(Provider, TranslateError)>,
    },
}

pub struct BubbleApp {
    config: Arc<Mutex<Config>>,
    engine: Engine,
    events: Receiver<UiEvent>,
    state: State,
    /// Where the bubble should sit, in global points (top-left origin).
    /// `None` when the pointer's position is not knowable here.
    anchor: Option<(f64, f64)>,
    visible: bool,
    /// Height requested for the viewport last frame, to avoid re-sending an
    /// identical resize every frame.
    last_height: f32,
    /// The zoom last handed to egui, so it is only reset when it changes.
    /// `None` until the display has been measured.
    applied_zoom: Option<f32>,
    /// Cleared once the window manager has been told what the bubble is.
    marking_pending: bool,
    /// Cleared once the bubble is set to appear on every workspace.
    ///
    /// Separate from the marking above because it cannot be done at the same
    /// time: a compositor that has to be asked directly only knows about the
    /// window once it has been mapped, which does not happen until the first
    /// translation.
    workspace_pending: bool,
    /// Set when the bubble has been positioned and is waiting to be revealed.
    ///
    /// Showing and sizing cannot happen in the same breath: the size is only
    /// known once the content has been laid out, which is after the window
    /// would already be on screen. Waiting one pass means the first thing the
    /// user sees is a finished bubble rather than one growing into place.
    pending_show: bool,
    /// Where the bubble was last placed, in points.
    ///
    /// Kept rather than read back from the window because the windowing
    /// system's own report of the position is not dependable for a window
    /// that never takes focus, and this side knows the answer exactly: it is
    /// what was just sent.
    last_pos: egui::Pos2,
    shown_at: Instant,
    settings_open: bool,
    /// Whether the language dropdown's list is showing.
    ///
    /// The bubble sizes itself to its content, and a popup is not content — it
    /// is an overlay drawn on top, clipped to the window like anything else.
    /// Without knowing it is open the window stays short and the list is cut
    /// off at the bubble's edge, so this is what buys it room.
    lang_popup_open: bool,
    copied_at: Option<Instant>,
    /// Why selections cannot be watched, when they cannot. Shown in the
    /// bubble's settings panel; the same problem is spelled out at length in
    /// the main window.
    readiness_warning: Option<String>,
    /// Shared with the main window's deferred viewport callback, which must be
    /// `Send + Sync + 'static` and so cannot borrow from here.
    main: Arc<Mutex<MainState>>,
    config_for_main: Arc<Mutex<Config>>,
    reopen_hooked: bool,
    /// Mirrors the current activation policy so it is only set when it changes.
    dock_visible: bool,
    /// Set when the app was told to start with no window, and cleared once the
    /// indicator has settled. While it is set the app is deliberately
    /// invisible, and it is the indicator that has to justify that.
    started_hidden: bool,
}

impl BubbleApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        config: Arc<Mutex<Config>>,
        engine: Engine,
        events: Receiver<UiEvent>,
        readiness_warning: Option<String>,
        main: Arc<Mutex<MainState>>,
        started_hidden: bool,
    ) -> Self {
        install_fonts(&cc.egui_ctx);

        let mut visuals = egui::Visuals::dark();
        visuals.window_fill = BUBBLE_BG;
        visuals.panel_fill = BUBBLE_BG;
        // Applies to widget labels that do not set a colour themselves;
        // explicit RichText colours still win.
        visuals.override_text_color = Some(TEXT_PRIMARY);
        cc.egui_ctx.set_visuals(visuals);

        Self {
            config_for_main: config.clone(),
            main,
            config,
            engine,
            events,
            state: State::Hidden,
            anchor: None,
            visible: false,
            applied_zoom: None,
            marking_pending: true,
            workspace_pending: true,
            pending_show: false,
            last_height: 0.0,
            last_pos: egui::Pos2::ZERO,
            shown_at: Instant::now(),
            settings_open: false,
            lang_popup_open: false,
            copied_at: None,
            readiness_warning,
            reopen_hooked: false,
            dock_visible: false,
            started_hidden,
        }
    }

    fn drain_events(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                UiEvent::Working { at, via } => {
                    self.anchor = at;
                    self.state = State::Working { via };
                    self.settings_open = false;
                    self.copied_at = None;
                    self.show(ctx);
                }
                UiEvent::Done {
                    source_text,
                    result,
                } => {
                    self.main.lock().unwrap().push_recent(
                        source_text,
                        result.text.clone(),
                        result.provider,
                    );
                    self.state = State::Done { result };
                    self.shown_at = Instant::now();
                }
                UiEvent::Failed { errors } => {
                    self.state = State::Failed { errors };
                    self.shown_at = Instant::now();
                }
                UiEvent::ManualDone(result) => {
                    let mut main = self.main.lock().unwrap();
                    main.translating = false;
                    main.result = Some(Ok(result));
                }
                UiEvent::ManualFailed(errors) => {
                    let mut main = self.main.lock().unwrap();
                    main.translating = false;
                    main.result = Some(Err(errors));
                }
                UiEvent::ProviderStatus(statuses) => {
                    let mut main = self.main.lock().unwrap();
                    main.testing = false;
                    main.statuses = statuses;
                }
            }
        }
    }

    fn show(&mut self, ctx: &egui::Context) {
        let pos = self.clamped_position(ctx);
        crate::trace!(
            "bubble    anchor={:?} -> ({:.0}, {:.0})",
            self.anchor,
            pos.x,
            pos.y,
        );
        self.last_pos = pos;
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
        if !self.visible {
            self.pending_show = true;
            // Ask again for the bubble to be on every workspace. Not a
            // one-time setup: a compositor that holds this as its own state
            // rather than as a window property loses it when the window
            // unmaps, and the bubble unmaps every time it hides.
            self.workspace_pending = true;
        }
        self.shown_at = Instant::now();
    }

    fn hide(&mut self, ctx: &egui::Context) {
        self.pending_show = false;
        if self.visible {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.visible = false;
        }
        self.state = State::Hidden;
        self.settings_open = false;
        monitor::set_paused(false);
    }

    /// Keeps the bubble fully on screen, flipping it above/left of the cursor
    /// when there isn't room below/right.
    fn clamped_position(&self, ctx: &egui::Context) -> egui::Pos2 {
        const MARGIN: f32 = 8.0;
        let height = self.last_height.max(MIN_HEIGHT);
        let monitor = ctx.input(|i| i.viewport().monitor_size);

        // The anchor arrives in whatever space the pointer was read in, which
        // is the toolkit's points on some systems and not on others.
        let anchor = self
            .anchor
            .map(|at| crate::platform::to_points(at, monitor));

        let Some(anchor) = anchor else {
            // Nothing said where the pointer is, so there is no cursor to sit
            // beside. The bottom-right corner is where a desktop puts
            // transient things anyway, and it is at least predictable.
            return match monitor {
                Some(monitor) => egui::pos2(
                    (monitor.x - BUBBLE_WIDTH - MARGIN).max(MARGIN),
                    (monitor.y - height - MARGIN).max(MARGIN),
                ),
                None => egui::pos2(MARGIN, MARGIN),
            };
        };

        let mut x = anchor.0 as f32 + CURSOR_OFFSET.0;
        let mut y = anchor.1 as f32 + CURSOR_OFFSET.1;

        if let Some(monitor) = monitor {
            if x + BUBBLE_WIDTH + MARGIN > monitor.x {
                x = (anchor.0 as f32 - BUBBLE_WIDTH - CURSOR_OFFSET.0)
                    .max(MARGIN)
                    .min(monitor.x - BUBBLE_WIDTH - MARGIN);
            }
            if y + height + MARGIN > monitor.y {
                y = (anchor.1 as f32 - height - CURSOR_OFFSET.1).max(MARGIN);
            }
            x = x.clamp(MARGIN, (monitor.x - BUBBLE_WIDTH - MARGIN).max(MARGIN));
            y = y.clamp(MARGIN, (monitor.y - height - MARGIN).max(MARGIN));
        }
        egui::pos2(x, y)
    }

    /// Tells the window manager what the bubble is: a notification that wants
    /// no decoration and no focus.
    ///
    /// Returns whether it took, so the caller can retry — the window may not
    /// exist yet on the first frame. Deliberately done before the bubble is
    /// ever mapped, which is what makes a window manager read it: the viewport
    /// starts hidden and is only shown once there is something to say.
    #[allow(unused_variables)]
    fn describe_bubble_to_wm(&self, frame: &eframe::Frame) -> bool {
        #[cfg(target_os = "linux")]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};

            let Ok(handle) = frame.window_handle() else {
                return false;
            };
            let RawWindowHandle::Xlib(x11) = handle.as_raw() else {
                return false;
            };
            crate::platform::mark_as_notification(x11.window as u32);
            return true;
        }
        #[cfg(not(target_os = "linux"))]
        true
    }

    /// Keeps egui's zoom in step with the display and the user's preference.
    ///
    /// Two independent factors. The display's own scaling is measured and
    /// matched, so that a point is the same size here as in every other window
    /// on screen. On top of that sits the user's `ui_scale`, because how large
    /// a desktop's applications choose to draw is a matter of taste that
    /// nothing can be read off the system.
    fn apply_zoom(&mut self, ctx: &egui::Context) {
        // The display cannot be measured until there is a window on it.
        let Some(native) = ctx.native_pixels_per_point() else {
            return;
        };
        let display = crate::platform::preferred_zoom(native).unwrap_or(1.0);
        let wanted = display * self.config.lock().unwrap().ui_scale.clamp(0.5, 2.0);

        if self.applied_zoom.is_none_or(|applied| (applied - wanted).abs() > 0.001) {
            crate::trace!("zoom      native={native} display={display} -> {wanted}");
            self.applied_zoom = Some(wanted);
            ctx.set_zoom_factor(wanted);
        }
    }

    /// Whether the pointer is inside the bubble right now.
    ///
    /// The system is asked first, because egui only knows what the window
    /// manager told it and on some systems the bubble — which never takes
    /// focus — is told the pointer arrived and never told it left. Where the
    /// system declines to answer, or the window's own rectangle is not known
    /// yet, egui's pointer state is the best available and is right on the
    /// platforms that do deliver both events.
    fn pointer_over_bubble(&self, ctx: &egui::Context) -> bool {
        let rect = egui::Rect::from_min_size(
            self.last_pos,
            egui::vec2(BUBBLE_WIDTH, self.last_height.max(MIN_HEIGHT)),
        );
        let monitor = ctx.input(|i| i.viewport().monitor_size);
        if let Some(over) = crate::platform::pointer_over(rect, monitor) {
            return over;
        }
        ctx.input(|i| i.pointer.has_pointer())
    }

    /// Registers (or drops) the main window's viewport for this frame.
    ///
    /// A deferred viewport only survives while the parent keeps asking for it,
    /// so while the window is open the root must keep painting even though the
    /// bubble itself may be hidden.
    fn drive_main_window(&mut self, ctx: &egui::Context) {
        // The windowing backend's app delegate does not exist yet when the app
        // is constructed, so keep trying until the reopen hook lands.
        if !self.reopen_hooked {
            self.reopen_hooked = shell::hook_reopen();
        }

        // Asked for by the indicator's menu, which is the only Quit the app
        // has. Closing the root viewport is what ends `run_native`, and only
        // this loop can do it — the indicator's callback runs on a thread that
        // owns no viewport at all.
        if shell::take_quit_request() {
            crate::trace!("quit requested");
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // An app asked to start in the background is invisible except for its
        // indicator, so it is only safe once that indicator is real. Registering
        // a tray is a negotiation with the desktop that can fail, and it is
        // asynchronous, so the verdict arrives after the first frames — hence
        // waiting for it rather than reading it once at startup. If it never
        // arrives, show the window: an app the user has no way to reach is a
        // worse outcome than an unwanted window.
        if self.started_hidden && shell::indicator_settled() {
            self.started_hidden = false;
            if !shell::has_indicator() {
                crate::trace!("no indicator; showing the window rather than hiding headless");
                let mut main = self.main.lock().unwrap();
                main.open = true;
                main.sized = false;
            }
        }

        if shell::take_open_request() {
            let mut main = self.main.lock().unwrap();
            // Only a window that was actually closed needs its size reasserted;
            // resetting it on an already-open window would undo a resize the
            // user made by hand.
            if !main.open {
                main.open = true;
                main.sized = false;
            }
            main.focus_requested = true;
        }

        let (open, wants_focus) = {
            let main = self.main.lock().unwrap();
            (main.open, main.focus_requested)
        };

        // Dock presence follows the window: regular while it is open so the app
        // can actually be brought to the front, accessory once it closes so the
        // bubble goes back to never stealing focus.
        if open != self.dock_visible {
            shell::set_foreground(open);
            self.dock_visible = open;
        }

        if !open {
            // With nothing outside the app's own windows to bring it back,
            // closing the main window is the only Quit there is. Staying alive
            // would leave a translator running that the user cannot reach.
            if !shell::has_indicator() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }

        let id = egui::ViewportId::from_hash_of("bubbleTranslate-main");

        // Raising happens in two halves, in this order: the process is pulled
        // forward (only possible now that the policy is regular), then this
        // particular window is made key. The flag is cleared here rather than
        // in the window's own draw, so the request fires exactly once — macOS
        // ignores activation that is asked for every frame.
        if wants_focus {
            shell::activate();
            ctx.send_viewport_cmd_to(id, egui::ViewportCommand::Focus);
            self.main.lock().unwrap().focus_requested = false;
        }
        let builder = egui::ViewportBuilder::default()
            .with_title("Bubble Translate")
            .with_inner_size(main_window::WINDOW_SIZE)
            .with_min_inner_size([400.0, 420.0]);

        let state = self.main.clone();
        let config = self.config_for_main.clone();
        ctx.show_viewport_deferred(id, builder, move |ui, _class| {
            // Closing the window must not take the translator down with it;
            // the app keeps running and the menu bar item brings it back.
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                state.lock().unwrap().open = false;
            }
            main_window::draw(ui, &state, &config);
        });

        // Keep the parent painting so the viewport above is re-registered.
        ctx.request_repaint();
    }

    fn target_lang(&self) -> String {
        self.config.lock().unwrap().target_lang.clone()
    }
}

impl eframe::App for BubbleApp {
    /// Transparent so the rounded corners of the bubble don't sit on a grey
    /// rectangle.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    /// Runs even while the bubble is hidden, so this is where the engine's
    /// results are picked up and the window is shown or dismissed.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Done here rather than at construction because the window has to
        // exist before the system will say what it is being scaled by.
        // Done before the bubble is ever mapped, which is what makes the
        // window manager read it: the viewport starts hidden and is only shown
        // once there is a translation to put in it.
        if self.marking_pending {
            self.marking_pending = !self.describe_bubble_to_wm(_frame);
        }
        // Only once the bubble has actually been on screen: until then a
        // compositor has no window to be told about.
        if self.workspace_pending && self.visible {
            self.workspace_pending = !crate::platform::keep_on_all_workspaces();
        }

        self.apply_zoom(ctx);

        self.drain_events(ctx);
        self.drive_main_window(ctx);

        // While the pointer is inside the bubble, gestures belong to us, not
        // to a new selection in the app underneath.
        let hovered = self.visible && self.pointer_over_bubble(ctx);
        monitor::set_paused(hovered);
        monitor::set_watch_clipboard(self.config.lock().unwrap().watch_clipboard);

        if matches!(self.state, State::Hidden) {
            if self.visible {
                self.hide(ctx);
            }
            // Nothing to draw; sleep until the engine wakes us.
            ctx.request_repaint_after(Duration::from_secs(3600));
            return;
        }

        // A visible bubble keeps painting whether or not anything changed:
        // the pointer moving onto it is not an event this window can rely on
        // being told about, so noticing it means looking.
        ctx.request_repaint_after(HOVER_POLL);

        // Auto-dismiss, paused while the pointer is inside so a bubble being
        // read never vanishes mid-sentence.
        let auto_hide = self.config.lock().unwrap().auto_hide_secs;
        if hovered {
            self.shown_at = Instant::now();
        } else if auto_hide > 0 && !self.settings_open {
            let elapsed = self.shown_at.elapsed();
            let budget = Duration::from_secs(auto_hide);
            if elapsed >= budget {
                self.hide(ctx);
                return;
            }
            ctx.request_repaint_after(budget - elapsed);
        }

        if matches!(self.state, State::Working { .. }) {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if matches!(self.state, State::Hidden) {
            return;
        }

        let bubble = egui::Frame::new()
            .fill(BUBBLE_BG)
            .stroke(egui::Stroke::new(1.0, BUBBLE_BORDER))
            .corner_radius(10.0)
            .inner_margin(egui::Margin::symmetric(16, 14))
            .shadow(egui::Shadow {
                offset: [0, 4],
                blur: 18,
                spread: 0,
                color: egui::Color32::from_black_alpha(120),
            });

        let mut dismiss = false;
        let response = bubble.show(ui, |ui| {
            ui.set_width(BUBBLE_WIDTH - 32.0);
            dismiss = self.draw_body(ui);
        });

        let ctx = ui.ctx().clone();

        // Size the window to whatever the content needed. One frame behind,
        // which is invisible in practice because the bubble appears in the
        // "Working" state first and grows into the result.
        let room = if self.lang_popup_open {
            LANG_POPUP_HEIGHT
        } else {
            0.0
        };
        let wanted = (response.response.rect.height() + 20.0 + room).clamp(MIN_HEIGHT, MAX_HEIGHT);
        if (wanted - self.last_height).abs() > 1.0 {
            self.last_height = wanted;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                BUBBLE_WIDTH,
                wanted,
            )));
            // Re-anchor: a taller bubble may no longer fit below the cursor.
            let pos = self.clamped_position(&ctx);
            self.last_pos = pos;
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
        }

        // Reveal only now, with the window already at its final size and
        // position: both commands above were queued ahead of this one, so the
        // bubble arrives finished instead of resizing in front of the user.
        if self.pending_show {
            self.pending_show = false;
            self.visible = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }

        if dismiss {
            self.hide(&ctx);
        }
    }
}

impl BubbleApp {
    /// Draws the bubble contents. Returns true when the user asked to close.
    fn draw_body(&mut self, ui: &mut egui::Ui) -> bool {
        let mut dismiss = false;

        // -- header: name, byline and the close button --------------------
        ui.horizontal_top(|ui| {
            ui.label(
                egui::RichText::new("bubbleTranslate")
                    .size(11.5)
                    .color(TEXT_SECONDARY),
            );
            // Right to left, so the close button takes the corner and the
            // byline sits just inside it.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                if ui
                    .add(egui::Button::new(egui::RichText::new(CLOSE_GLYPH).size(17.0)).frame(false))
                    .on_hover_text("Close")
                    .clicked()
                {
                    dismiss = true;
                }
                ui.label(
                    egui::RichText::new("by pelamx")
                        .size(11.5)
                        .color(TEXT_MUTED),
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
                            CaptureSource::Accessibility
                            | CaptureSource::PrimarySelection => "Translating…",
                            CaptureSource::Clipboard => "Translating (via copy)…",
                        })
                        .size(13.5)
                        .color(TEXT_SECONDARY),
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
                                .color(TEXT_PRIMARY),
                        );
                    });
            }
            State::Failed { errors, .. } => {
                ui.label(
                    egui::RichText::new("No provider could translate this")
                        .size(14.0)
                        .color(TEXT_ERROR),
                );
                ui.add_space(2.0);
                for (provider, err) in errors {
                    ui.label(
                        egui::RichText::new(format!("{}: {err}", provider.label()))
                            .size(12.0)
                            .color(TEXT_MUTED),
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

    fn draw_footer(&mut self, ui: &mut egui::Ui) -> bool {
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
            ui.label(egui::RichText::new(caption).size(11.5).color(TEXT_MUTED));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(egui::Button::new(egui::RichText::new("⚙").size(14.0)).frame(false))
                    .on_hover_text("Settings")
                    .clicked()
                {
                    self.settings_open = !self.settings_open;
                }

                if let State::Done { result, .. } = &self.state {
                    let just_copied = self
                        .copied_at
                        .is_some_and(|t| t.elapsed() < Duration::from_secs(2));
                    let label = if just_copied { "Copied" } else { "Copy" };
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
                    .on_hover_text("Change target language")
                    .clicked()
                {
                    self.settings_open = !self.settings_open;
                }
            });
        });

        dismiss
    }

    fn draw_settings(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        let mut retranslate = false;
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
            ui.label(egui::RichText::new("Providers, in order").size(12.0));
            ui.label(
                egui::RichText::new(
                    cfg.providers
                        .iter()
                        .map(|p| p.label())
                        .collect::<Vec<_>>()
                        .join(" → "),
                )
                .size(12.0)
                .color(TEXT_SECONDARY),
            );
        });

        if ui
            .checkbox(
                &mut cfg.auto_translate,
                egui::RichText::new("Translate on selection").size(12.0),
            )
            .changed()
        {
            dirty = true;
        }

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
                egui::RichText::new("Translate into")
                    .size(12.0)
                    .color(TEXT_SECONDARY),
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
        if let Some(code) = chosen {
            if code != cfg.target_lang {
                cfg.target_lang = code;
                dirty = true;
                retranslate = true;
            }
        }
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Text size").size(12.0));
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
                    .color(TEXT_MUTED),
            );
        });

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("DeepL key").size(12.0));
            if ui
                .add(
                    egui::TextEdit::singleline(&mut cfg.deepl_api_key)
                        .password(true)
                        .hint_text("optional")
                        .desired_width(180.0),
                )
                .lost_focus()
            {
                dirty = true;
            }
        });

        ui.label(
            egui::RichText::new(format!("Config: {}", Config::path().display()))
                .size(10.5)
                .color(TEXT_MUTED),
        );

        self.lang_popup_open = popup_open;

        if dirty {
            let _ = cfg.save();
        }
        // The config lock must be released before asking the engine to redo the
        // translation, or the engine thread blocks on it.
        drop(cfg);
        if retranslate {
            self.engine.request(Request::Retranslate);
        }
    }
}

fn install_fonts(ctx: &egui::Context) {
    let Some((path, bytes)) = find_fallback_font() else {
        // Latin-only rendering is degraded but still usable, so this is not
        // worth failing startup over.
        eprintln!("bubbleTranslate: no Unicode fallback font found; non-Latin text may not render");
        return;
    };
    crate::trace!("fallback font: {path}");
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "unicode-fallback".to_owned(),
        Arc::new(egui::FontData::from_owned(bytes)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("unicode-fallback".to_owned());
    }
    ctx.set_fonts(fonts);
}

/// Finds a font with coverage past Latin, and reads it.
///
/// The list is tried first because on most systems it is both instant and
/// right. Asking fontconfig is the backstop: it knows where this particular
/// distribution put its fonts, which no hardcoded list can keep up with.
fn find_fallback_font() -> Option<(String, Vec<u8>)> {
    for path in FALLBACK_FONTS {
        if let Ok(bytes) = std::fs::read(path) {
            return Some(((*path).to_string(), bytes));
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Asking for a Chinese sans-serif is a shortcut to "the font on this
        // machine with the widest coverage": whatever answers is almost
        // certainly a Noto CJK, which also carries Cyrillic, Greek and Arabic.
        let out = std::process::Command::new("fc-match")
            .args(["-f", "%{file}", "sans-serif:lang=zh"])
            .output()
            .ok()?;
        let path = String::from_utf8(out.stdout).ok()?;
        let path = path.trim();
        if !path.is_empty() {
            if let Ok(bytes) = std::fs::read(path) {
                return Some((path.to_string(), bytes));
            }
        }
    }

    None
}
