//! OSC-8 hyperlink escape sequences.
//!
//! OSC-8 is a terminal escape sequence that makes a span of text
//! clickable in supporting terminals (iTerm2, kitty, recent xterm,
//! WezTerm, Alacritty, VTE-based). The format is:
//!
//! ```text
//! ESC ]8;;<url> ESC \ <text> ESC ]8;; ESC \
//! ```
//!
//! # Scope in symbi-shell
//!
//! The sequences contain escape chars that [`unicode_width`] (and
//! therefore ratatui) counts as printable. Embedding OSC-8 inside a
//! ratatui `Span` breaks column widths, truncation, and popup
//! alignment — so we deliberately **do not** use OSC-8 inside the
//! live viewport or `insert_before`-rendered lines.
//!
//! We only use OSC-8 for plain `println!` paths that run *after* the
//! ratatui terminal has been dropped — the session-saved resume hint,
//! CLI early-exit listings, and similar one-shot output. In those
//! contexts the terminal handles the escape as intended and the extra
//! chars don't collide with any TUI layout math.
//!
//! [`unicode_width`]: https://docs.rs/unicode-width/

use std::path::Path;

/// Emit text wrapped in an OSC-8 hyperlink to `url`. Supporting
/// terminals render `text` as clickable; others display `text` as
/// plain text (the escape sequence is invisible even when unsupported,
/// so it's safe to always include).
///
/// Caller is responsible for deciding whether this output will end up
/// somewhere safe to interpret OSC-8 (see module docs — do NOT feed
/// this into a ratatui Span).
pub fn hyperlink(url: &str, text: &str) -> String {
    // "ESC ]8;; <url> ESC \\ <text> ESC ]8;; ESC \\"
    // Where ESC = \x1b and \\ is the literal backslash String Terminator.
    // The ESC\\ is the 7-bit form of the ST (0x9c) terminator; we use it
    // over the bare BEL variant because it's better-supported.
    format!("\x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\", url, text)
}

/// Convenience: turn a filesystem path into a `file://` hyperlink.
/// If the path can't be made absolute, emit `text` unwrapped.
pub fn file_link(path: &Path, text: &str) -> String {
    match path.canonicalize() {
        Ok(abs) => hyperlink(&format!("file://{}", abs.display()), text),
        Err(_) => text.to_string(),
    }
}

/// Best-effort check that stdout is a terminal that will render OSC-8.
///
/// We treat a non-TTY stdout as "don't emit" because escape sequences
/// leak into pipes/logs as junk. Terminals without OSC-8 support see
/// the sequences as no-ops so true/true is safe; false/true would clog
/// a pipe.
pub fn stdout_is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyperlink_format_is_well_formed() {
        let out = hyperlink("https://example.com", "click me");
        assert!(out.starts_with("\x1b]8;;https://example.com\x1b\\"));
        assert!(out.ends_with("click me\x1b]8;;\x1b\\"));
    }

    #[test]
    fn file_link_falls_back_to_plain_text_for_missing_path() {
        let out = file_link(Path::new("/no/such/path/for/sure"), "label");
        assert_eq!(out, "label");
    }
}
