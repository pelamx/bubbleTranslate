//! Omarchy: the desktop this app is most at home on, and the only one it
//! reaches into.
//!
//! - **The theme.** Omarchy keeps the palette of the theme in use as a
//!   `colors.toml` in its state directory. The bubble can be told to follow it,
//!   so switching the desktop's theme switches the bubble's along with it
//!   rather than leaving it on whichever of the built-in palettes was closest.
//! - **The bar.** Older versions could put a widget on Omarchy's bar. It has
//!   been withdrawn; what is left here takes it off again.

use std::path::PathBuf;

/// Where Omarchy keeps the theme in use: a directory holding the theme's
/// files, with `theme.name` beside it. The state directory is current Omarchy;
/// the config directory is where older releases kept the same thing.
fn current_dirs() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/state"));
    vec![
        state.join("omarchy/current"),
        home.join(".config/omarchy/current"),
    ]
}

/// The directory naming the current theme, on a machine running Omarchy.
fn current_dir() -> Option<PathBuf> {
    current_dirs()
        .into_iter()
        .find(|dir| dir.join("theme.name").is_file() || dir.join("theme").is_dir())
}

/// Whether this is an Omarchy desktop. Everything in this module is offered
/// only when it is.
pub fn present() -> bool {
    current_dir().is_some()
}

/// What changes when the desktop's theme does: its name, and when its colours
/// were last written — the second so editing a theme in place is noticed too.
/// Cheap enough to ask about once a second.
pub fn theme_stamp() -> Option<String> {
    let dir = current_dir()?;
    let name = std::fs::read_to_string(dir.join("theme.name")).unwrap_or_default();
    let written = std::fs::metadata(dir.join("theme/colors.toml"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0);
    Some(format!("{}:{written}", name.trim()))
}

/// The colours of the theme in use, as Omarchy names them.
pub struct ThemeColors {
    pub light: bool,
    colors: std::collections::HashMap<String, [u8; 3]>,
}

impl ThemeColors {
    /// One named colour, if the theme defines it.
    pub fn get(&self, key: &str) -> Option<[u8; 3]> {
        self.colors.get(key).copied()
    }
}

/// Reads the current theme's `colors.toml`. `None` off Omarchy, and on a theme
/// old enough not to have one — the bubble then keeps its original palette.
pub fn theme_colors() -> Option<ThemeColors> {
    let text = std::fs::read_to_string(current_dir()?.join("theme/colors.toml")).ok()?;
    parse_colors(&text)
}

fn parse_colors(text: &str) -> Option<ThemeColors> {
    let table: toml::Table = toml::from_str(text).ok()?;
    let colors: std::collections::HashMap<String, [u8; 3]> = table
        .iter()
        .filter_map(|(key, value)| Some((key.clone(), hex(value.as_str()?)?)))
        .collect();
    // A theme without these two is not one the bubble can be drawn from.
    if !colors.contains_key("background") || !colors.contains_key("foreground") {
        return None;
    }
    let light = table.get("mode").and_then(|m| m.as_str()) == Some("light");
    Some(ThemeColors { light, colors })
}

fn hex(value: &str) -> Option<[u8; 3]> {
    let digits = value.trim().strip_prefix('#')?;
    if digits.len() < 6 || !digits.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// The id of the bar widget older versions installed.
const PLUGIN_ID: &str = "app.bubbletranslate";

/// Takes the bar widget off again wherever an older version installed it. Run
/// at startup, silently, and by `--omarchy-remove`.
pub fn remove_stale_bar_widget() {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };
    if home
        .join(".config/omarchy/plugins")
        .join(PLUGIN_ID)
        .is_dir()
    {
        std::thread::spawn(|| {
            let _ = std::process::Command::new("omarchy")
                .args(["plugin", "disable", PLUGIN_ID])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            let _ = remove_bar_widget_files();
        });
    }
}

fn remove_bar_widget_files() -> std::io::Result<()> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    std::fs::remove_dir_all(home.join(".config/omarchy/plugins").join(PLUGIN_ID))
}

/// `--omarchy-remove`: takes the bar widget off by hand.
pub fn remove_bar_widget() -> i32 {
    let _ = std::process::Command::new("omarchy")
        .args(["plugin", "disable", PLUGIN_ID])
        .status();
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return 1;
    };
    let dir = home.join(".config/omarchy/plugins").join(PLUGIN_ID);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => {
            println!("bar widget removed");
            0
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => 0,
        Err(err) => {
            eprintln!("bubbleTranslate: could not remove {}: {err}", dir.display());
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_omarchy_colors_file() {
        let theme = parse_colors(
            "mode = \"light\"\naccent = \"#1e66f5\"\nbackground = \"#eff1f5\"\n\
             foreground = \"#4c4f69\"\nhyprland_active_border = \"rgba(26a269ee) 45deg\"\n",
        )
        .unwrap();
        assert!(theme.light);
        assert_eq!(theme.get("accent"), Some([0x1e, 0x66, 0xf5]));
        assert_eq!(theme.get("hyprland_active_border"), None);
    }

    #[test]
    fn a_theme_without_its_base_colours_is_refused() {
        assert!(parse_colors("accent = \"#ffffff\"\n").is_none());
        assert!(parse_colors("not toml at all [").is_none());
    }
}
