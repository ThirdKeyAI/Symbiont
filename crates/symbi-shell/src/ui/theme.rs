//! Theme system for symbi-shell.
//!
//! One central [`Theme`] struct owns every color used in the UI. All
//! widget code reads from [`current`] rather than hardcoding
//! `Color::Green` / `Color::DarkGray` / etc., so switching themes is a
//! single assignment at startup.
//!
//! Selection precedence (first wins):
//! 1. `$HOME/.symbi[-<profile>]/theme.toml` — user file, may extend a
//!    built-in and override individual fields.
//! 2. `--theme <name>` CLI flag or `$SYMBI_THEME` env var.
//! 3. [`Theme::default_dark`].
//!
//! To add a built-in: write a new `fn my_theme() -> Theme` returning a
//! filled struct, add it to [`from_name`] and [`BUILTIN_NAMES`].

use anyhow::{anyhow, Result};
use ratatui::style::Color;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// All the colors used across the UI, grouped by semantic role rather
/// than ratatui name. Where the same color appears in many places
/// (e.g. our ubiquitous `DarkGray` "dim" text) one field serves them
/// all — the goal is for a user's theme override to change every
/// related element in lockstep.
#[derive(Clone, Debug)]
pub struct Theme {
    // --- Transcript roles ---
    pub user: Color,
    pub agent: Color,
    pub sys: Color,
    pub err: Color,
    pub meta: Color,
    /// Generic "dimmed" color — used for separators, hint text,
    /// prefixes, and most DarkGray fills today.
    pub dim: Color,

    // --- Notices (ℹ / ✓ / ⚠ / ✗) ---
    pub info: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,

    // --- Tool cards ---
    pub tool_name: Color,
    pub tool_args: Color,
    pub tool_done: Color,
    pub tool_running: Color,
    pub tool_error: Color,

    // --- Diff hunks ---
    pub diff_add: Color,
    pub diff_del: Color,
    pub diff_hunk: Color,
    pub diff_context: Color,

    // --- Markdown ---
    pub md_heading: Color,
    pub md_code: Color,
    pub md_link: Color,
    pub md_blockquote: Color,
    pub md_list_ordered: Color,
    pub md_list_unordered: Color,

    // --- Syntax highlighting (DSL / Cedar / TOML / Clad) ---
    pub syn_keyword: Color,
    pub syn_string: Color,
    pub syn_number: Color,
    pub syn_comment: Color,
    pub syn_type: Color,
    pub syn_operator: Color,

    // --- Footer + input ---
    pub footer_accent: Color,
    pub input_text: Color,
    pub input_border: Color,
}

/// Names of every built-in theme, in display order. Used for
/// `--theme <name>` validation and any future "list themes" command.
pub const BUILTIN_NAMES: &[&str] = &["default-dark", "solarized-dark", "high-contrast"];

impl Theme {
    /// The default theme — the cyan/green/yellow/gray palette that
    /// shipped before theming. Everything a pre-theming session user
    /// saw is reproduced here exactly.
    pub fn default_dark() -> Self {
        Self {
            user: Color::Cyan,
            agent: Color::Green,
            sys: Color::DarkGray,
            err: Color::Red,
            meta: Color::DarkGray,
            dim: Color::DarkGray,

            info: Color::Cyan,
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,

            tool_name: Color::White,
            tool_args: Color::Gray,
            tool_done: Color::Green,
            tool_running: Color::Yellow,
            tool_error: Color::Red,

            diff_add: Color::Green,
            diff_del: Color::Red,
            diff_hunk: Color::Magenta,
            diff_context: Color::Gray,

            md_heading: Color::Cyan,
            md_code: Color::Yellow,
            md_link: Color::Cyan,
            md_blockquote: Color::Green,
            md_list_ordered: Color::LightBlue,
            md_list_unordered: Color::DarkGray,

            syn_keyword: Color::Magenta,
            syn_string: Color::Green,
            syn_number: Color::Yellow,
            syn_comment: Color::DarkGray,
            syn_type: Color::Cyan,
            syn_operator: Color::Gray,

            footer_accent: Color::Cyan,
            input_text: Color::White,
            input_border: Color::DarkGray,
        }
    }

    /// Solarized Dark palette (ansi base03..base3). Warmer than
    /// default-dark; easier on the eyes for long sessions.
    pub fn solarized_dark() -> Self {
        // Solarized reference hex values.
        let base03 = Color::Rgb(0x00, 0x2b, 0x36);
        let _ = base03;
        let base01 = Color::Rgb(0x58, 0x6e, 0x75);
        let base1 = Color::Rgb(0x93, 0xa1, 0xa1);
        let yellow = Color::Rgb(0xb5, 0x89, 0x00);
        let orange = Color::Rgb(0xcb, 0x4b, 0x16);
        let red = Color::Rgb(0xdc, 0x32, 0x2f);
        let magenta = Color::Rgb(0xd3, 0x36, 0x82);
        let blue = Color::Rgb(0x26, 0x8b, 0xd2);
        let cyan = Color::Rgb(0x2a, 0xa1, 0x98);
        let green = Color::Rgb(0x85, 0x99, 0x00);

        Self {
            user: cyan,
            agent: green,
            sys: base01,
            err: red,
            meta: base01,
            dim: base01,

            info: blue,
            success: green,
            warning: yellow,
            error: red,

            tool_name: base1,
            tool_args: base01,
            tool_done: green,
            tool_running: yellow,
            tool_error: red,

            diff_add: green,
            diff_del: red,
            diff_hunk: magenta,
            diff_context: base1,

            md_heading: blue,
            md_code: yellow,
            md_link: cyan,
            md_blockquote: green,
            md_list_ordered: blue,
            md_list_unordered: base01,

            syn_keyword: magenta,
            syn_string: green,
            syn_number: orange,
            syn_comment: base01,
            syn_type: cyan,
            syn_operator: base1,

            footer_accent: cyan,
            input_text: base1,
            input_border: base01,
        }
    }

    /// WCAG-AA-leaning monochrome. Designed for screen-share, low
    /// color-gamut terminals, and users with tritanopia/protanopia —
    /// everything is reduced to white-on-black with bright yellow for
    /// "needs attention". Differentiation comes from modifiers
    /// (bold/italic/underline) rather than hue.
    pub fn high_contrast() -> Self {
        Self {
            user: Color::White,
            agent: Color::White,
            sys: Color::Gray,
            err: Color::Yellow,
            meta: Color::Gray,
            dim: Color::Gray,

            info: Color::White,
            success: Color::White,
            warning: Color::Yellow,
            error: Color::Yellow,

            tool_name: Color::White,
            tool_args: Color::Gray,
            tool_done: Color::White,
            tool_running: Color::Yellow,
            tool_error: Color::Yellow,

            diff_add: Color::White,
            diff_del: Color::Yellow,
            diff_hunk: Color::White,
            diff_context: Color::Gray,

            md_heading: Color::White,
            md_code: Color::White,
            md_link: Color::White,
            md_blockquote: Color::White,
            md_list_ordered: Color::White,
            md_list_unordered: Color::Gray,

            syn_keyword: Color::White,
            syn_string: Color::White,
            syn_number: Color::White,
            syn_comment: Color::Gray,
            syn_type: Color::White,
            syn_operator: Color::Gray,

            footer_accent: Color::White,
            input_text: Color::White,
            input_border: Color::Gray,
        }
    }

    /// Look up a built-in theme by name. Returns `None` for unknown
    /// names so the caller can surface a readable error.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "default-dark" | "default" => Some(Self::default_dark()),
            "solarized-dark" | "solarized" => Some(Self::solarized_dark()),
            "high-contrast" | "mono" => Some(Self::high_contrast()),
            _ => None,
        }
    }

    /// Load a TOML theme file. The file may set `extends = "<name>"`
    /// to start from a built-in, then override individual fields.
    /// With no `extends`, missing fields keep their `default_dark`
    /// values so partial files still produce a complete theme.
    pub fn from_toml(path: &Path) -> Result<Self> {
        let src = std::fs::read_to_string(path)
            .map_err(|e| anyhow!("failed to read theme file {}: {}", path.display(), e))?;
        Self::from_toml_str(&src)
    }

    pub fn from_toml_str(src: &str) -> Result<Self> {
        let spec: ThemeSpec =
            toml::from_str(src).map_err(|e| anyhow!("invalid theme TOML: {}", e))?;
        let mut base = match spec.extends.as_deref() {
            Some(name) => Self::from_name(name)
                .ok_or_else(|| anyhow!("unknown base theme in extends=\"{}\"", name))?,
            None => Self::default_dark(),
        };
        spec.apply(&mut base)?;
        Ok(base)
    }
}

/// Serde-facing mirror of `Theme` with every field optional, so a
/// user's TOML can mention only the roles they want to override.
#[derive(Deserialize, Default)]
struct ThemeSpec {
    extends: Option<String>,

    user: Option<String>,
    agent: Option<String>,
    sys: Option<String>,
    err: Option<String>,
    meta: Option<String>,
    dim: Option<String>,

    info: Option<String>,
    success: Option<String>,
    warning: Option<String>,
    error: Option<String>,

    tool_name: Option<String>,
    tool_args: Option<String>,
    tool_done: Option<String>,
    tool_running: Option<String>,
    tool_error: Option<String>,

    diff_add: Option<String>,
    diff_del: Option<String>,
    diff_hunk: Option<String>,
    diff_context: Option<String>,

    md_heading: Option<String>,
    md_code: Option<String>,
    md_link: Option<String>,
    md_blockquote: Option<String>,
    md_list_ordered: Option<String>,
    md_list_unordered: Option<String>,

    syn_keyword: Option<String>,
    syn_string: Option<String>,
    syn_number: Option<String>,
    syn_comment: Option<String>,
    syn_type: Option<String>,
    syn_operator: Option<String>,

    footer_accent: Option<String>,
    input_text: Option<String>,
    input_border: Option<String>,
}

impl ThemeSpec {
    fn apply(self, t: &mut Theme) -> Result<()> {
        macro_rules! set {
            ($field:ident) => {
                if let Some(s) = &self.$field {
                    t.$field = parse_color(s)?;
                }
            };
        }
        set!(user);
        set!(agent);
        set!(sys);
        set!(err);
        set!(meta);
        set!(dim);
        set!(info);
        set!(success);
        set!(warning);
        set!(error);
        set!(tool_name);
        set!(tool_args);
        set!(tool_done);
        set!(tool_running);
        set!(tool_error);
        set!(diff_add);
        set!(diff_del);
        set!(diff_hunk);
        set!(diff_context);
        set!(md_heading);
        set!(md_code);
        set!(md_link);
        set!(md_blockquote);
        set!(md_list_ordered);
        set!(md_list_unordered);
        set!(syn_keyword);
        set!(syn_string);
        set!(syn_number);
        set!(syn_comment);
        set!(syn_type);
        set!(syn_operator);
        set!(footer_accent);
        set!(input_text);
        set!(input_border);
        Ok(())
    }
}

/// Parse a color spec from a TOML field.
///
/// Accepted forms:
/// - `"red"`, `"darkgray"`, `"lightblue"` — any ratatui named color
///   (case-insensitive, dash-or-space separators normalised away).
/// - `"#rrggbb"` — truecolor hex.
/// - `"indexed:N"` where 0 ≤ N ≤ 255 — 256-color palette index.
/// - `"rgb:r,g,b"` where each is 0..=255 — explicit truecolor.
pub fn parse_color(s: &str) -> Result<Color> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix('#') {
        if rest.len() != 6 {
            return Err(anyhow!("hex color '{}' must be #rrggbb", s));
        }
        let r = u8::from_str_radix(&rest[0..2], 16)
            .map_err(|_| anyhow!("hex color '{}' has non-hex digits", s))?;
        let g = u8::from_str_radix(&rest[2..4], 16)
            .map_err(|_| anyhow!("hex color '{}' has non-hex digits", s))?;
        let b = u8::from_str_radix(&rest[4..6], 16)
            .map_err(|_| anyhow!("hex color '{}' has non-hex digits", s))?;
        return Ok(Color::Rgb(r, g, b));
    }
    if let Some(rest) = s.strip_prefix("indexed:") {
        let n: u8 = rest
            .trim()
            .parse()
            .map_err(|_| anyhow!("indexed color '{}' needs a 0..=255 integer", s))?;
        return Ok(Color::Indexed(n));
    }
    if let Some(rest) = s.strip_prefix("rgb:") {
        let parts: Vec<&str> = rest.split(',').map(str::trim).collect();
        if parts.len() != 3 {
            return Err(anyhow!(
                "rgb color '{}' needs three comma-separated values",
                s
            ));
        }
        let r: u8 = parts[0]
            .parse()
            .map_err(|_| anyhow!("rgb red '{}' not 0..=255", parts[0]))?;
        let g: u8 = parts[1]
            .parse()
            .map_err(|_| anyhow!("rgb green '{}' not 0..=255", parts[1]))?;
        let b: u8 = parts[2]
            .parse()
            .map_err(|_| anyhow!("rgb blue '{}' not 0..=255", parts[2]))?;
        return Ok(Color::Rgb(r, g, b));
    }

    // Named colors — lowercase, strip spaces/dashes so "dark gray",
    // "dark-gray", "darkgray" all resolve.
    let normalised: String = s
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_'))
        .collect();
    Ok(match normalised.as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "darkgrey" => Color::DarkGray,
        "lightred" => Color::LightRed,
        "lightgreen" => Color::LightGreen,
        "lightyellow" => Color::LightYellow,
        "lightblue" => Color::LightBlue,
        "lightmagenta" => Color::LightMagenta,
        "lightcyan" => Color::LightCyan,
        "white" => Color::White,
        "reset" => Color::Reset,
        other => return Err(anyhow!("unknown color '{}'", other)),
    })
}

// --- Process-global theme ----------------------------------------------------

static CURRENT: OnceLock<Theme> = OnceLock::new();

/// Install the active theme. Best-effort: if a theme has already been
/// locked in (e.g. because a test path read [`current`] first and got
/// the default), the call is a no-op. Production runs always call this
/// from `main` before any UI code touches [`current`].
pub fn init(theme: Theme) {
    let _ = CURRENT.set(theme);
}

/// Borrow the active theme.
///
/// In tests (and any unit call path that runs before `main` has called
/// [`init`]) returns the default theme instead of panicking. Production
/// code paths always install a theme in `main`, so the fallback only
/// matters for `#[test]`-only call sites.
pub fn current() -> &'static Theme {
    CURRENT.get_or_init(Theme::default_dark)
}

/// Where the optional user theme file lives, honouring the same
/// `SYMBIONT_SESSION_DIR` / `--profile` indirection we use for
/// sessions: both live under `$HOME/.symbi[-<profile>]/`.
pub fn user_theme_path() -> Option<PathBuf> {
    // The sessions dir env var points at `.../sessions`; its parent is
    // the profile root where `theme.toml` lives.
    if let Ok(sessions_dir) = std::env::var("SYMBIONT_SESSION_DIR") {
        let parent = PathBuf::from(sessions_dir).parent().map(Path::to_path_buf);
        if let Some(p) = parent {
            return Some(p.join("theme.toml"));
        }
    }
    if let Some(mut home) = dirs::home_dir() {
        home.push(".symbi");
        home.push("theme.toml");
        return Some(home);
    }
    None
}

/// Resolve which theme to use given a CLI flag and env var.
///
/// Order:
/// 1. User TOML file at [`user_theme_path`], if it exists.
/// 2. `cli_name` (from `--theme`) if non-empty.
/// 3. `$SYMBI_THEME` env var if set.
/// 4. Built-in default-dark.
pub fn resolve(cli_name: Option<&str>) -> Result<Theme> {
    if let Some(path) = user_theme_path() {
        if path.exists() {
            return Theme::from_toml(&path);
        }
    }
    if let Some(name) = cli_name {
        return Theme::from_name(name).ok_or_else(|| {
            anyhow!(
                "unknown theme '{}' (built-ins: {})",
                name,
                BUILTIN_NAMES.join(", ")
            )
        });
    }
    if let Ok(name) = std::env::var("SYMBI_THEME") {
        return Theme::from_name(&name).ok_or_else(|| {
            anyhow!(
                "unknown SYMBI_THEME='{}' (built-ins: {})",
                name,
                BUILTIN_NAMES.join(", ")
            )
        });
    }
    Ok(Theme::default_dark())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_all_resolvable() {
        for name in BUILTIN_NAMES {
            assert!(
                Theme::from_name(name).is_some(),
                "BUILTIN_NAMES entry {:?} missing from_name match",
                name
            );
        }
    }

    #[test]
    fn parse_color_named() {
        assert_eq!(parse_color("red").unwrap(), Color::Red);
        assert_eq!(parse_color("DarkGray").unwrap(), Color::DarkGray);
        assert_eq!(parse_color("dark-gray").unwrap(), Color::DarkGray);
        assert_eq!(parse_color("light blue").unwrap(), Color::LightBlue);
        assert_eq!(parse_color("grey").unwrap(), Color::Gray);
    }

    #[test]
    fn parse_color_hex() {
        assert_eq!(
            parse_color("#ff5a2d").unwrap(),
            Color::Rgb(0xff, 0x5a, 0x2d)
        );
        assert!(parse_color("#bad").is_err()); // wrong length
        assert!(parse_color("#zzzzzz").is_err());
    }

    #[test]
    fn parse_color_indexed() {
        assert_eq!(parse_color("indexed:214").unwrap(), Color::Indexed(214));
        assert!(parse_color("indexed:not-a-number").is_err());
    }

    #[test]
    fn parse_color_rgb() {
        assert_eq!(parse_color("rgb:10,20,30").unwrap(), Color::Rgb(10, 20, 30));
        assert!(parse_color("rgb:10,20").is_err());
    }

    #[test]
    fn parse_color_rejects_unknown() {
        assert!(parse_color("puce").is_err());
    }

    #[test]
    fn toml_extends_a_builtin_and_overrides_fields() {
        // Note: the fixture contains `"#` sequences inside the hex color,
        // so we use r##"..."## to avoid the raw-string terminator.
        let src = r##"
            extends = "high-contrast"
            user = "#ff00ff"
            tool_done = "indexed:42"
        "##;
        let t = Theme::from_toml_str(src).unwrap();
        assert_eq!(t.user, Color::Rgb(0xff, 0x00, 0xff));
        assert_eq!(t.tool_done, Color::Indexed(42));
        // Unoverridden field stays at the extended theme's value.
        assert_eq!(t.warning, Theme::high_contrast().warning);
    }

    #[test]
    fn toml_with_no_extends_fills_from_default_dark() {
        let src = r#"user = "magenta""#;
        let t = Theme::from_toml_str(src).unwrap();
        assert_eq!(t.user, Color::Magenta);
        assert_eq!(t.agent, Theme::default_dark().agent);
    }

    #[test]
    fn toml_rejects_unknown_extends() {
        let src = r#"extends = "nope""#;
        assert!(Theme::from_toml_str(src).is_err());
    }

    #[test]
    fn toml_rejects_invalid_color() {
        let src = r#"user = "puce""#;
        assert!(Theme::from_toml_str(src).is_err());
    }
}
