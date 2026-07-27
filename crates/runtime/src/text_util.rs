//! Small, dependency-free text helpers shared across the runtime crate.
//!
//! These are intentionally unconditional (no feature gate): callers that are
//! always compiled (e.g. `reasoning::knowledge_bridge`) and callers that are
//! feature-gated (e.g. `http_input::server`) both need the same behavior, so
//! it lives here rather than being duplicated per call site.

/// Truncate a string to at most `max_bytes` bytes without splitting a UTF-8
/// character. Returns the full string when it is already within the limit.
///
/// Byte-index slicing (`&s[..n]`) panics if `n` does not land on a UTF-8
/// character boundary. Any text that can contain multi-byte characters and
/// is not fully caller-controlled (model output, user input, remote content)
/// must go through this instead of a raw slice.
pub fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // Find the largest char boundary <= max_bytes
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_utf8_ascii_within_limit() {
        assert_eq!(truncate_utf8("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_utf8_ascii_at_limit() {
        assert_eq!(truncate_utf8("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_utf8_ascii_over_limit() {
        assert_eq!(truncate_utf8("hello world", 5), "hello");
    }

    #[test]
    fn test_truncate_utf8_multibyte_boundary() {
        // "é" is 2 bytes (0xC3 0xA9). "café" is 5 bytes.
        // Truncating at 4 bytes would land inside the 'é', should back up to 3.
        let s = "café";
        assert_eq!(s.len(), 5);
        let t = truncate_utf8(s, 4);
        assert_eq!(t, "caf");
    }

    #[test]
    fn test_truncate_utf8_emoji() {
        // "👍" is 4 bytes. Truncating at 2 should yield empty.
        let s = "👍";
        assert_eq!(s.len(), 4);
        assert_eq!(truncate_utf8(s, 2), "");
    }

    #[test]
    fn test_truncate_utf8_cjk() {
        // Each CJK character is 3 bytes. "你好" is 6 bytes.
        let s = "你好";
        assert_eq!(s.len(), 6);
        // Truncate at 4 should yield "你" (3 bytes)
        assert_eq!(truncate_utf8(s, 4), "你");
    }

    #[test]
    fn test_truncate_utf8_empty() {
        assert_eq!(truncate_utf8("", 10), "");
    }

    #[test]
    fn test_truncate_utf8_zero_limit() {
        assert_eq!(truncate_utf8("hello", 0), "");
    }

    #[test]
    fn test_truncate_utf8_never_panics_on_random_boundaries() {
        // Regression guard: every limit from 0..=len must produce valid,
        // non-panicking output regardless of where multi-byte characters fall.
        let s = "a é 你好 👍 b";
        for limit in 0..=s.len() + 2 {
            let t = truncate_utf8(s, limit);
            assert!(t.len() <= limit.min(s.len()));
        }
    }
}
