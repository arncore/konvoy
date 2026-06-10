//! Small text helpers shared across crates.

/// The first non-blank line of `text`, trimmed — or `None` if every line is blank
/// (or `text` is empty).
///
/// Handy for surfacing a concise one-line hint from a tool's captured output
/// without dumping the whole log: leading blank lines are skipped and the chosen
/// line's surrounding whitespace is trimmed.
#[must_use]
pub fn first_non_blank_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_leading_blanks_and_trims_the_chosen_line() {
        assert_eq!(
            first_non_blank_line("\n   \n  hello world \nmore"),
            Some("hello world")
        );
        assert_eq!(first_non_blank_line("first\nsecond"), Some("first"));
    }

    #[test]
    fn none_when_all_blank_or_empty() {
        assert_eq!(first_non_blank_line("   \n\t\n  "), None);
        assert_eq!(first_non_blank_line(""), None);
    }
}
