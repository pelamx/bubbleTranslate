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
use crate::config::{BubbleTheme, Config, LANGUAGES, Provider, TriggerKey, language_name};
use crate::engine::{Engine, Request, UiEvent};
use crate::i18n::t;
use crate::license::{self, Licensing};
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

/// How often to re-ask whether the permission the app is missing has been
/// granted. Only asked while it is missing, and only while something is
/// drawing — which is the case that matters, since that is when someone is
/// looking at the status this keeps honest.
const READINESS_POLL: Duration = Duration::from_millis(750);

mod draw;
pub(crate) mod theme;

use theme::*;
pub use theme::{TRANSPARENT_BUBBLE, pal};

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
    /// Today's free allowance is spent. Appears exactly where a translation would
    /// have, and always carries the way past it.
    Capped {
        limit: u32,
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
    /// How many frames have been painted since startup, counted only as far as
    /// [`BubbleApp::settle_hidden`] needs it.
    startup_frames: u8,
    /// Height requested for the viewport last frame, to avoid re-sending an
    /// identical resize every frame.
    last_height: f32,
    /// The zoom last handed to egui, so it is only reset when it changes.
    /// `None` until the display has been measured.
    applied_zoom: Option<f32>,
    /// The theme whose palette is currently installed, so a change made from
    /// any surface is noticed and applied exactly once.
    applied_theme: Option<BubbleTheme>,
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
    /// When the permission behind [`Self::readiness_warning`] was last
    /// re-asked.
    readiness_checked: Instant,
    /// Shared with the main window's deferred viewport callback, which must be
    /// `Send + Sync + 'static` and so cannot borrow from here.
    main: Arc<Mutex<MainState>>,
    config_for_main: Arc<Mutex<Config>>,
    /// Read by the settings window, written by the engine.
    licensing: Licensing,
    reopen_hooked: bool,
    /// Mirrors the current activation policy so it is only set when it changes.
    dock_visible: bool,
    /// Set when the app was told to start with no window, and cleared once the
    /// indicator has settled. While it is set the app is deliberately
    /// invisible, and it is the indicator that has to justify that.
    started_hidden: bool,
}

impl BubbleApp {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        config: Arc<Mutex<Config>>,
        engine: Engine,
        events: Receiver<UiEvent>,
        readiness_warning: Option<String>,
        main: Arc<Mutex<MainState>>,
        licensing: Licensing,
        started_hidden: bool,
    ) -> Self {
        install_fonts(&cc.egui_ctx);
        // Point the palette at the saved theme before egui's own widget
        // colours are baked in, so the first bubble is already themed.
        let theme = config.lock().unwrap().theme;
        set_palette(theme);
        install_theme(&cc.egui_ctx);

        Self {
            config_for_main: config.clone(),
            main,
            licensing,
            config,
            engine,
            events,
            state: State::Hidden,
            anchor: None,
            visible: false,
            startup_frames: 0,
            applied_zoom: None,
            applied_theme: Some(theme),
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
            readiness_checked: Instant::now(),
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
                UiEvent::Capped { at, limit } => {
                    self.anchor = at;
                    self.state = State::Capped { limit };
                    self.settings_open = false;
                    self.copied_at = None;
                    self.show(ctx);
                    // Nothing follows this the way `Done` follows `Working`,
                    // so the auto-hide countdown has to start here.
                    self.shown_at = Instant::now();
                }
                UiEvent::ManualCapped { used, limit } => {
                    let mut main = self.main.lock().unwrap();
                    main.translating = false;
                    main.result = None;
                    main.capped = Some((used, limit));
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
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
            BUBBLE_WIDTH,
            self.last_height.max(MIN_HEIGHT),
        )));
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(Self::window_origin(
            pos,
        )));
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

    /// Where the *window* goes so that the bubble lands at `pos`.
    ///
    /// The two are the same everywhere but Windows, where the bubble's window
    /// keeps a frame it never shows and the contents therefore start a title
    /// bar below the window's own corner. See [`crate::platform::frame_offset`].
    fn window_origin(pos: egui::Pos2) -> egui::Pos2 {
        let (dx, dy) = crate::platform::frame_offset();
        egui::pos2(pos.x - dx, pos.y - dy)
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

    /// Puts the bubble's window away at startup, once.
    ///
    /// The viewport is built hidden, and then shown anyway: the toolkit makes
    /// the window visible itself after it has painted its first frame, so that
    /// no application it hosts can ever flash an unpainted window. For an app
    /// whose main window *is* the thing that should not be seen yet, that is a
    /// 400-pixel rectangle left on the desktop with nothing in it — invisible
    /// for as long as the window is transparent, which is why it went unnoticed
    /// until Windows, and a click-blocking hole in the desktop even there.
    ///
    /// It has to be undone on the frame *after* the first, because that is when
    /// it happens; hence the extra repaint, which is the only one the hidden
    /// bubble ever asks for.
    fn settle_hidden(&mut self, ctx: &egui::Context) {
        if self.startup_frames >= 2 {
            return;
        }
        self.startup_frames += 1;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        ctx.request_repaint();
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
            true
        }
        #[cfg(target_os = "windows")]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};

            let Ok(handle) = frame.window_handle() else {
                return false;
            };
            let RawWindowHandle::Win32(win32) = handle.as_raw() else {
                return false;
            };
            crate::platform::mark_as_notification(win32.hwnd.get());
            true
        }
        #[cfg(target_os = "macos")]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};

            let Ok(handle) = frame.window_handle() else {
                return false;
            };
            let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
                return false;
            };
            shell::drop_window_shadow(appkit.ns_view.as_ptr());
            true
        }
    }

    /// Picks up a permission granted while the app was already running.
    ///
    /// The selection monitor waits for the same grant on its own thread, so
    /// the app starts working without being told; this is what stops the
    /// window insisting it is blind while the bubble is plainly translating.
    /// Nothing polls while the app is idle and unwatched — `logic` sleeps
    /// then, and there is no one for a stale status to mislead.
    fn refresh_readiness(&mut self) {
        if self.readiness_checked.elapsed() < READINESS_POLL {
            return;
        }
        self.readiness_checked = Instant::now();
        let was_ok = self.main.lock().unwrap().readiness.ok;
        if let Some(readiness) = capture::recheck(was_ok) {
            self.readiness_warning = (!readiness.ok).then(|| readiness.summary.clone());
            self.main.lock().unwrap().readiness = readiness;
        }
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

        if self
            .applied_zoom
            .is_none_or(|applied| (applied - wanted).abs() > 0.001)
        {
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
            .with_title("bubbleTranslate")
            .with_inner_size(main_window::WINDOW_SIZE)
            .with_min_inner_size([400.0, 420.0]);

        let state = self.main.clone();
        let config = self.config_for_main.clone();
        let licensing = self.licensing.clone();
        ctx.show_viewport_deferred(id, builder, move |ui, _class| {
            // Closing the window must not take the translator down with it;
            // the app keeps running and the menu bar item brings it back.
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                state.lock().unwrap().open = false;
            }
            main_window::draw(ui, &state, &config, &licensing);
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
    /// rectangle — or, where transparency would be composited as black, the
    /// bubble's own colour, so that the parts of the window the card does not
    /// cover are indistinguishable from the card. See [`TRANSPARENT_BUBBLE`].
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if TRANSPARENT_BUBBLE {
            [0.0, 0.0, 0.0, 0.0]
        } else {
            pal().bubble_bg.to_normalized_gamma_f32()
        }
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
        self.refresh_readiness();

        self.drain_events(ctx);
        self.drive_main_window(ctx);

        // While the pointer is inside the bubble, gestures belong to us, not
        // to a new selection in the app underneath.
        let hovered = self.visible && self.pointer_over_bubble(ctx);
        monitor::set_paused(hovered);
        let theme = {
            let cfg = self.config.lock().unwrap();
            monitor::set_watch_clipboard(cfg.watch_clipboard);
            monitor::set_trigger_key(cfg.trigger_key);
            cfg.theme
        };
        // Pick up a theme changed anywhere — this bubble's own panel, the main
        // window, or the config file edited by hand — and turn the palette over
        // there and then. The bubble's panel already swapped it for immediate
        // feedback; this is what makes the choice dynamic from every other
        // surface too.
        if self.applied_theme != Some(theme) {
            self.applied_theme = Some(theme);
            set_palette(theme);
            install_theme(ctx);
            ctx.request_repaint();
        }

        if matches!(self.state, State::Hidden) {
            if self.visible {
                self.hide(ctx);
            }
            self.settle_hidden(ctx);
            // Nothing to draw; sleep until the engine wakes us.
            ctx.request_repaint_after(Duration::from_secs(3600));
            return;
        }

        // The bubble resizes to whatever it has to say, and where the window
        // has an outline of its own that outline has to follow the card's.
        crate::platform::shape_bubble();

        // Revealed here rather than from the draw. eframe calls `ui` only for a
        // viewport that is *already* visible, so a bubble that waits for its own
        // draw to show itself never shows at all — on any platform. `logic`
        // runs every frame either way. The size and position queued by `show`
        // are ahead of this command, so the bubble still arrives placed rather
        // than jumping into position afterwards.
        if self.pending_show {
            self.pending_show = false;
            self.visible = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
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
            .fill(pal().bubble_bg)
            .stroke(egui::Stroke::new(CARD_STROKE, pal().bubble_border))
            .corner_radius(10.0)
            .inner_margin(egui::Margin::symmetric(CARD_PAD_X, CARD_PAD_Y))
            // A shadow needs somewhere to fall; see [`shadow_margin`]. The
            // card is inset by exactly that much, which is also what keeps its
            // outline inside the window rather than half a pixel past it.
            .outer_margin(shadow_margin())
            .shadow(BUBBLE_SHADOW);

        let mut dismiss = false;
        let response = bubble.show(ui, |ui| {
            ui.set_width(card_content_width());
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
        // The frame's rect already covers the card plus the room its shadow
        // falls in, so nothing is added here for it. Windows, whose card casts
        // no shadow and so has no margin, still wants a hair of slack for the
        // rounding the desktop manager puts on the window's own corners.
        let below = if TRANSPARENT_BUBBLE { 0.0 } else { 2.0 };
        let wanted = (response.response.rect.height() + below + room).clamp(MIN_HEIGHT, MAX_HEIGHT);
        if (wanted - self.last_height).abs() > 1.0 {
            self.last_height = wanted;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                BUBBLE_WIDTH,
                wanted,
            )));
            // Re-anchor: a taller bubble may no longer fit below the cursor.
            let pos = self.clamped_position(&ctx);
            self.last_pos = pos;
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(Self::window_origin(
                pos,
            )));
        }

        if dismiss {
            self.hide(&ctx);
        }
    }
}
