//! Small text helpers shared across PDT's messages and notes (PDT-660): Markdown code blocks that hold any text, and
//! text cut short with "…".

/// `text` in a Markdown code block tagged `lang` (empty for none), its fence longer than any run of backticks in the
/// text, so a fence inside it (a Markdown file's, a diff of one) can't end the block early.
pub fn code_block(lang: &str, text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}{lang}\n{}\n{fence}", text.trim_end_matches('\n'))
}

/// `text` cut to `max` characters, the last one "…" (with no space before it). Text within `max` stays whole.
pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// Like [`clip`], but cut at the last space when one is in the second half, so no word is cut in two.
pub fn clip_words(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    let cut = match cut.rfind(char::is_whitespace) {
        Some(i) if i > max / 2 => cut[..i].trim_end(),
        _ => cut.trim_end(),
    };
    format!("{cut}…")
}

/// The start of `text` of at most `bytes` bytes, ending on a character's boundary.
pub fn head_bytes(text: &str, bytes: usize) -> &str {
    &text[..text.floor_char_boundary(bytes)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_block_outlasts_the_fences_inside_it() {
        assert_eq!(code_block("diff", "-a\n+b\n"), "```diff\n-a\n+b\n```");
        assert_eq!(code_block("", "x"), "```\nx\n```");
        assert_eq!(code_block("diff", "+````x"), "`````diff\n+````x\n`````");
        // A Markdown file's fence in a conflict hunk stays inside the block.
        let b = code_block("diff", "+```rust\n+let x = 1;\n+```");
        assert!(b.starts_with("````diff\n") && b.ends_with("\n````"), "{b}");
    }

    #[test]
    fn clipped_text_is_at_most_max_characters() {
        assert_eq!(clip("short", 5), "short");
        assert_eq!(clip("abcdef", 5), "abcd…");
        assert_eq!(clip("ab   cdef", 5), "ab…", "no space before the …");
        assert_eq!(clip("éééééé", 3), "éé…");
        assert_eq!(clip("abc", 0), "…");
        assert_eq!(clip_words("Fix the login page on Safari", 20), "Fix the login page…");
        assert_eq!(clip_words("Supercalifragilistic word", 10), "Supercali…", "no space late enough: cut the word");
        assert_eq!(clip_words("fits", 10), "fits");
    }

    #[test]
    fn head_bytes_ends_on_a_character() {
        assert_eq!(head_bytes("ééé", 3), "é");
        assert_eq!(head_bytes("abc", 5), "abc");
        assert_eq!(head_bytes("abc", 0), "");
    }
}
