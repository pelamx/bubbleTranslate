//! How the bubble looks: its palettes, the card it is drawn on, and the fonts
//! that let it show scripts egui does not bundle.

use super::*;

/// The close button's glyph.
///
/// A multiplication sign rather than one of the several nicer-looking crosses
/// in the symbol blocks, because it lives in Latin-1 and is therefore in every
/// font that exists. The prettier ✕ (U+2715) is absent from the fonts most
/// Linux distributions ship, and an absent glyph does not degrade — it draws a
/// hollow box, which reads as a broken button rather than a close one.
pub(super) const CLOSE_GLYPH: &str = "×";

/// Bubble palette.
///
/// Every colour the bubble paints is a field here, so a theme is a value the
/// drawing code reads rather than a set of constants baked in at compile time.
/// The translation is the one thing worth reading, so it gets the lightest
/// text on a deliberately dark panel; everything else is chrome and steps down
/// from there. Each theme keeps that contrast — light text well clear of the
/// dim greys that make small text on a dark background hard to read.
#[derive(Clone, Copy)]
pub struct Palette {
    /// The translation itself, and anything that has to read as cleanly.
    pub text_primary: egui::Color32,
    pub text_secondary: egui::Color32,
    pub text_muted: egui::Color32,
    /// The card, and the outline around it.
    pub bubble_bg: egui::Color32,
    pub bubble_border: egui::Color32,
    pub text_error: egui::Color32,
    /// The controls: a button at rest, then under the pointer, then pressed.
    pub control: egui::Color32,
    pub control_hover: egui::Color32,
    pub control_active: egui::Color32,
    /// Behind text the user types into: a well, darker than anything around it.
    pub field_bg: egui::Color32,
    /// The one colour with any saturation. Selected text, the filled half of a
    /// slider, the focus ring.
    pub accent: egui::Color32,
    /// Outlines: a control at rest, and one being pointed at.
    pub control_edge: egui::Color32,
    pub control_edge_hover: egui::Color32,
    /// Alternating-row tint, and the fill of a disabled control.
    pub faint_bg: egui::Color32,
    pub disabled_bg: egui::Color32,
    /// Whether this is a light scheme — dark text on a pale card rather than
    /// the other way round. It picks which of egui's two bases the widgets it
    /// draws itself start from, so the handful of colours this palette does
    /// not name are still the right end of the range.
    pub light: bool,
}

pub(super) const fn rgb(r: u8, g: u8, b: u8) -> egui::Color32 {
    egui::Color32::from_rgb(r, g, b)
}

/// The colours for one theme. [`BubbleTheme::Slate`] is the original scheme;
/// the rest are the desktop palettes Omarchy ships, mapped onto the dozen-odd
/// roles the bubble actually uses so the card can be told to match the screen
/// around it.
pub(super) fn palette_for(theme: BubbleTheme) -> Palette {
    match theme {
        BubbleTheme::Slate => Palette {
            text_primary: rgb(240, 240, 240),
            text_secondary: rgb(186, 186, 186),
            text_muted: rgb(158, 158, 158),
            bubble_bg: rgb(30, 31, 34),
            bubble_border: rgb(88, 88, 88),
            text_error: rgb(255, 150, 150),
            control: rgb(52, 54, 60),
            control_hover: rgb(66, 69, 77),
            control_active: rgb(82, 86, 96),
            field_bg: rgb(22, 23, 26),
            accent: rgb(92, 156, 226),
            control_edge: rgb(70, 73, 81),
            control_edge_hover: rgb(104, 108, 118),
            faint_bg: rgb(38, 39, 43),
            disabled_bg: rgb(40, 42, 47),
            light: false,
        },
        BubbleTheme::TokyoNight => Palette {
            text_primary: rgb(192, 202, 245),
            text_secondary: rgb(169, 177, 214),
            text_muted: rgb(121, 130, 169),
            bubble_bg: rgb(26, 27, 38),
            bubble_border: rgb(65, 72, 104),
            text_error: rgb(247, 118, 142),
            control: rgb(36, 40, 59),
            control_hover: rgb(47, 53, 73),
            control_active: rgb(59, 66, 97),
            field_bg: rgb(22, 22, 30),
            accent: rgb(122, 162, 247),
            control_edge: rgb(47, 53, 73),
            control_edge_hover: rgb(84, 92, 126),
            faint_bg: rgb(30, 32, 48),
            disabled_bg: rgb(34, 36, 54),
            light: false,
        },
        BubbleTheme::Catppuccin => Palette {
            text_primary: rgb(205, 214, 244),
            text_secondary: rgb(186, 194, 222),
            text_muted: rgb(147, 153, 178),
            bubble_bg: rgb(30, 30, 46),
            bubble_border: rgb(69, 71, 90),
            text_error: rgb(243, 139, 168),
            control: rgb(49, 50, 68),
            control_hover: rgb(69, 71, 90),
            control_active: rgb(88, 91, 112),
            field_bg: rgb(24, 24, 37),
            accent: rgb(203, 166, 247),
            control_edge: rgb(69, 71, 90),
            control_edge_hover: rgb(108, 112, 134),
            faint_bg: rgb(36, 36, 56),
            disabled_bg: rgb(41, 42, 61),
            light: false,
        },
        BubbleTheme::Gruvbox => Palette {
            text_primary: rgb(235, 219, 178),
            text_secondary: rgb(213, 196, 161),
            text_muted: rgb(168, 153, 132),
            bubble_bg: rgb(40, 40, 40),
            bubble_border: rgb(80, 73, 69),
            text_error: rgb(251, 73, 52),
            control: rgb(60, 56, 54),
            control_hover: rgb(80, 73, 69),
            control_active: rgb(102, 92, 84),
            field_bg: rgb(29, 32, 33),
            accent: rgb(250, 189, 47),
            control_edge: rgb(80, 73, 69),
            control_edge_hover: rgb(124, 111, 100),
            faint_bg: rgb(50, 48, 47),
            disabled_bg: rgb(58, 55, 53),
            light: false,
        },
        BubbleTheme::Nord => Palette {
            text_primary: rgb(236, 239, 244),
            text_secondary: rgb(216, 222, 233),
            text_muted: rgb(123, 136, 161),
            bubble_bg: rgb(46, 52, 64),
            bubble_border: rgb(67, 76, 94),
            text_error: rgb(191, 97, 106),
            control: rgb(59, 66, 82),
            control_hover: rgb(67, 76, 94),
            control_active: rgb(76, 86, 106),
            field_bg: rgb(39, 44, 54),
            accent: rgb(136, 192, 208),
            control_edge: rgb(67, 76, 94),
            control_edge_hover: rgb(97, 110, 136),
            faint_bg: rgb(53, 59, 73),
            disabled_bg: rgb(59, 66, 82),
            light: false,
        },
        BubbleTheme::RosePine => Palette {
            text_primary: rgb(224, 222, 244),
            text_secondary: rgb(200, 197, 224),
            text_muted: rgb(144, 140, 170),
            bubble_bg: rgb(25, 23, 36),
            bubble_border: rgb(64, 61, 82),
            text_error: rgb(235, 111, 146),
            control: rgb(31, 29, 46),
            control_hover: rgb(38, 35, 58),
            control_active: rgb(57, 53, 82),
            field_bg: rgb(22, 20, 31),
            accent: rgb(235, 188, 186),
            control_edge: rgb(64, 61, 82),
            control_edge_hover: rgb(82, 79, 103),
            faint_bg: rgb(33, 32, 46),
            disabled_bg: rgb(42, 40, 57),
            light: false,
        },
        // The light pair. A pale card over a dark desktop is a brighter thing
        // than the text it is quoting, so these keep the same contrast the dark
        // schemes do — the translation is the darkest ink on the card, and the
        // chrome steps up towards the background rather than down towards it.
        BubbleTheme::CatppuccinLatte => Palette {
            text_primary: rgb(76, 79, 105),
            text_secondary: rgb(92, 95, 119),
            text_muted: rgb(124, 127, 147),
            bubble_bg: rgb(239, 241, 245),
            bubble_border: rgb(172, 176, 190),
            text_error: rgb(210, 15, 57),
            control: rgb(220, 224, 234),
            control_hover: rgb(204, 208, 218),
            control_active: rgb(188, 192, 204),
            field_bg: rgb(255, 255, 255),
            accent: rgb(136, 57, 239),
            control_edge: rgb(188, 192, 204),
            control_edge_hover: rgb(140, 143, 161),
            faint_bg: rgb(230, 233, 239),
            disabled_bg: rgb(220, 224, 232),
            light: true,
        },
        BubbleTheme::RosePineDawn => Palette {
            text_primary: rgb(87, 82, 121),
            text_secondary: rgb(121, 117, 147),
            text_muted: rgb(144, 140, 160),
            bubble_bg: rgb(250, 244, 237),
            bubble_border: rgb(206, 202, 205),
            text_error: rgb(180, 99, 122),
            control: rgb(242, 233, 225),
            control_hover: rgb(223, 218, 217),
            control_active: rgb(206, 202, 205),
            field_bg: rgb(255, 250, 243),
            accent: rgb(144, 122, 169),
            control_edge: rgb(223, 218, 217),
            control_edge_hover: rgb(180, 173, 174),
            faint_bg: rgb(255, 250, 243),
            disabled_bg: rgb(242, 233, 225),
            light: true,
        },
    }
}

thread_local! {
    /// The palette the bubble is painting with right now. The bubble is drawn
    /// on one thread and reads a handful of colours per frame, so a plain Cell
    /// carries it with no lock; [`set_palette`] swaps it when the choice
    /// changes.
    static ACTIVE_PALETTE: std::cell::Cell<Palette> =
        std::cell::Cell::new(palette_for(BubbleTheme::Slate));
}

/// The active palette, for the drawing code to read a colour from.
pub fn pal() -> Palette {
    ACTIVE_PALETTE.with(|p| p.get())
}

/// Point the bubble at a theme's colours. Call [`install_theme`] afterwards so
/// egui's own widgets — the ones it draws rather than this code — follow.
pub(super) fn set_palette(theme: BubbleTheme) {
    ACTIVE_PALETTE.with(|p| p.set(palette_for(theme)));
}

/// Whether the bubble's window can have a transparent background.
///
/// Everywhere but Windows it can, and that is what lets the bubble be a
/// rounded card with a drop shadow floating over the desktop.
///
/// Windows composites a window's alpha only when the graphics stack cooperates
/// — a DWM blur-behind region, a surface with an alpha mode, and a driver that
/// preserves it — and where any of that is missing the transparent pixels are
/// composited as black. The result is the worst of both: a black frame around
/// the bubble and a black smear where the shadow should be. Since a translator
/// has to work on every machine rather than on the ones with the right driver,
/// the Windows bubble is an opaque card — which is what a Windows tooltip is
/// anyway — with the window's own corners rounded by the desktop manager.
pub const TRANSPARENT_BUBBLE: bool = !cfg!(target_os = "windows");

/// The card's drop shadow.
///
/// Where the window behind the card is opaque the shadow would land on the
/// card's own colour and read as a smear along the bottom edge, so there the
/// desktop draws the window's shadow instead and this one is left off.
pub(super) const BUBBLE_SHADOW: egui::Shadow = if TRANSPARENT_BUBBLE {
    egui::Shadow {
        offset: [0, 4],
        blur: 18,
        spread: 0,
        color: egui::Color32::from_black_alpha(120),
    }
} else {
    egui::Shadow::NONE
};

/// The card's own outline, and the gap between it and its text.
pub(super) const CARD_STROKE: f32 = 1.0;
pub(super) const CARD_PAD_X: i8 = 16;
pub(super) const CARD_PAD_Y: i8 = 14;

/// How far outside the card its shadow reaches, on each side.
///
/// A shadow is painted outside the shape that casts it, and egui clips
/// painting at the window's edge — so a card laid out flush with the window
/// gets its shadow cut off on every side that has no room. Leaving room only
/// below is worse than leaving none at all: the surviving band spans the full
/// width with both ends cut square, which reads as a bar under the bubble
/// rather than as a shadow.
///
/// Derived from the shadow rather than written down beside it, so tuning the
/// blur or the offset moves the room it needs with it. Mirrors
/// `epaint::Shadow::margin`, in the integer units a [`egui::Margin`] takes.
pub(super) const fn shadow_margin() -> egui::Margin {
    let reach = (BUBBLE_SHADOW.spread + BUBBLE_SHADOW.blur.div_ceil(2)) as i8;
    let [dx, dy] = BUBBLE_SHADOW.offset;
    egui::Margin {
        left: reach - dx,
        right: reach + dx,
        top: reach - dy,
        bottom: reach + dy,
    }
}

/// The width left for the bubble's contents once the card, its padding and the
/// room for its shadow are taken out of the window.
pub(super) fn card_content_width() -> f32 {
    let m = shadow_margin();
    BUBBLE_WIDTH - (m.left + m.right) as f32 - (2 * CARD_PAD_X) as f32 - 2.0 * CARD_STROKE
}

/// The bubble's theme, pinned to the chosen palette.
///
/// Pinned rather than following the system, because every panel, every frame
/// and every label in the bubble names its own colour. Left to follow the
/// desktop, egui would hand the widgets it draws itself — buttons, text
/// fields, checkboxes, sliders — to the opposite palette on a machine set the
/// other way, and the result is a dark card with white boxes scattered through
/// it. Which base is used is the palette's own answer: [`Palette::light`] is
/// what says a theme is a light one, so egui's remaining internals — popup
/// shadows, scrollbars, hyperlinks — start from the right end rather than
/// being overridden one at a time.
pub(super) fn install_theme(ctx: &egui::Context) {
    let p = pal();
    let mut visuals = if p.light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    visuals.window_fill = p.bubble_bg;
    visuals.panel_fill = p.bubble_bg;
    // Applies to widget labels that do not set a colour themselves;
    // explicit RichText colours still win.
    visuals.override_text_color = Some(p.text_primary);

    // Text fields and other "sunken" surfaces.
    visuals.extreme_bg_color = p.field_bg;
    visuals.faint_bg_color = p.faint_bg;
    visuals.selection.bg_fill = p.accent.gamma_multiply(0.45);
    visuals.selection.stroke = egui::Stroke::new(1.0, p.text_primary);

    for (widget, fill, edge) in [
        (&mut visuals.widgets.inactive, p.control, p.control_edge),
        (
            &mut visuals.widgets.hovered,
            p.control_hover,
            p.control_edge_hover,
        ),
        (&mut visuals.widgets.active, p.control_active, p.accent),
    ] {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = egui::Stroke::new(1.0, edge);
        widget.fg_stroke = egui::Stroke::new(1.0, p.text_primary);
        widget.corner_radius = egui::CornerRadius::same(6);
    }
    // What a disabled control looks like: the same shape, sunk into the
    // background, with text that has visibly given up.
    visuals.widgets.noninteractive.bg_fill = p.disabled_bg;
    visuals.widgets.noninteractive.weak_bg_fill = p.disabled_bg;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, p.control_edge);
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, p.text_muted);
    visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.open.bg_fill = p.control_hover;
    visuals.widgets.open.weak_bg_fill = p.control_hover;
    visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0, p.control_edge_hover);
    visuals.widgets.open.fg_stroke = egui::Stroke::new(1.0, p.text_primary);
    visuals.widgets.open.corner_radius = egui::CornerRadius::same(6);

    ctx.set_visuals(visuals);
    // Both halves are needed: the visuals above are stored against one theme,
    // and this is what says which of the two egui should be using.
    ctx.set_theme(if p.light {
        egui::ThemePreference::Light
    } else {
        egui::ThemePreference::Dark
    });
}

/// Multiplier on the font size for line spacing. Translated paragraphs are
/// often long sentences with no visual breaks, and the extra leading is what
/// makes them scannable.
pub(super) const LINE_HEIGHT_RATIO: f32 = 1.45;

/// Fonts with broad script coverage, tried in order, so Chinese, Japanese,
/// Korean, Arabic and Cyrillic output renders instead of showing
/// missing-glyph boxes. egui's bundled fonts are Latin-only.
///
/// macOS ships one font that covers nearly everything. Windows splits the same
/// coverage across three that are always present. Linux distributions split it
/// across several Noto families and put them wherever they like, so the list is
/// longer and there is a search behind it — see [`find_fallback_fonts`].
#[cfg(target_os = "macos")]
pub(super) const FALLBACK_FONTS: &[&str] = &["/System/Library/Fonts/Supplemental/Arial Unicode.ttf"];

#[cfg(target_os = "linux")]
pub(super) const FALLBACK_FONTS: &[&str] = &[
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
];

/// In coverage order, not in preference order: Chinese and Japanese first
/// because they are the bulk of what a translator has to draw, then Korean,
/// then Segoe UI for Arabic, Hebrew, Greek and Cyrillic. All three ship with
/// every Windows 10 and 11 install, including the ones that never had an East
/// Asian language pack added — the font files are always there even when the
/// input methods are not.
#[cfg(target_os = "windows")]
pub(super) const FALLBACK_FONTS: &[&str] = &[
    r"C:\Windows\Fonts\msyh.ttc",
    r"C:\Windows\Fonts\malgun.ttf",
    r"C:\Windows\Fonts\segoeui.ttf",
];

pub(super) fn install_fonts(ctx: &egui::Context) {
    let found = find_fallback_fonts();
    if found.is_empty() {
        // Latin-only rendering is degraded but still usable, so this is not
        // worth failing startup over.
        eprintln!("bubbleTranslate: no Unicode fallback font found; non-Latin text may not render");
        return;
    }

    let mut fonts = egui::FontDefinitions::default();
    for (index, (path, bytes)) in found.into_iter().enumerate() {
        crate::trace!("fallback font: {path}");
        let name = format!("unicode-fallback-{index}");
        fonts
            .font_data
            .insert(name.clone(), Arc::new(egui::FontData::from_owned(bytes)));
        // Appended in the order they were found, which is the order they are
        // searched in: the first font that has the glyph draws it.
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push(name.clone());
        }
    }
    ctx.set_fonts(fonts);
}

/// Finds fonts with coverage past Latin, and reads them.
///
/// More than one, because no single file necessarily has the whole of it:
/// macOS ships one font that covers nearly everything, while Windows splits
/// Chinese, Korean and Arabic across three, and a Linux distribution splits
/// the same coverage across several Noto families and puts them wherever it
/// likes.
///
/// The list is tried first because on most systems it is both instant and
/// right. Asking fontconfig is the backstop: it knows where this particular
/// distribution put its fonts, which no hardcoded list can keep up with.
pub(super) fn find_fallback_fonts() -> Vec<(String, Vec<u8>)> {
    // A CJK font is tens of megabytes and stays resident, so this stops at
    // three: enough for the split Windows has, and past the point where a
    // Linux list of alternative paths for one font could load it twice over.
    const LIMIT: usize = 3;

    let mut found = Vec::new();
    for path in FALLBACK_FONTS {
        if found.len() == LIMIT {
            break;
        }
        if let Ok(bytes) = std::fs::read(path) {
            found.push(((*path).to_string(), bytes));
        }
    }
    if !found.is_empty() {
        return found;
    }

    #[cfg(target_os = "linux")]
    {
        // Asking for a Chinese sans-serif is a shortcut to "the font on this
        // machine with the widest coverage": whatever answers is almost
        // certainly a Noto CJK, which also carries Cyrillic, Greek and Arabic.
        if let Ok(out) = std::process::Command::new("fc-match")
            .args(["-f", "%{file}", "sans-serif:lang=zh"])
            .output()
        {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty()
                && let Ok(bytes) = std::fs::read(&path) {
                    found.push((path, bytes));
                }
        }
    }

    found
}
