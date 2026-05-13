//! Styled `indicatif` progress bars for downloads.
//!
//! Centralises the look-and-feel so every place that downloads an artifact
//! (toolchain, detekt, plugins, Maven klibs) renders the same way and
//! `konvoy update`'s `MultiProgress` can host any of them as a child bar.

use std::borrow::Cow;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

/// Template used for download bars: prefix, bar, bytes / total, speed.
const DOWNLOAD_TEMPLATE: &str =
    "  {prefix:<32} [{bar:30.cyan/blue}] {bytes:>10}/{total_bytes:<10} {bytes_per_sec:>12}";

/// Template used when the server didn't send `Content-Length`.
const SPINNER_TEMPLATE: &str = "  {prefix:<32} {spinner} {bytes:>10} {bytes_per_sec:>12}";

/// Build a styled [`ProgressBar`] for an HTTP download.
///
/// The `prefix` is shown left of the bar (e.g. `"kotlinx-coroutines 1.9.0"`).
/// The length is set later by the downloader once `Content-Length` is known;
/// callers don't need to know it upfront.
#[must_use]
pub fn new_download_bar(prefix: impl Into<Cow<'static, str>>) -> ProgressBar {
    let pb = ProgressBar::new(0);
    pb.set_style(download_style());
    pb.set_prefix(prefix);
    pb
}

/// Build the same kind of bar but attach it to a [`MultiProgress`] container.
///
/// Used by `konvoy update` to render N parallel downloads in a vertical row.
#[must_use]
pub fn add_download_bar(mp: &MultiProgress, prefix: impl Into<Cow<'static, str>>) -> ProgressBar {
    let pb = mp.add(ProgressBar::new(0));
    pb.set_style(download_style());
    pb.set_prefix(prefix);
    pb
}

/// Hidden bar for use in tests or non-interactive contexts.
#[must_use]
pub fn hidden_bar() -> ProgressBar {
    ProgressBar::hidden()
}

/// Switch a bar to spinner mode (no known total).
///
/// Call from the downloader when the response has no `Content-Length`.
pub fn switch_to_spinner(pb: &ProgressBar) {
    pb.set_style(spinner_style());
}

fn download_style() -> ProgressStyle {
    // The template strings are static and well-formed; falling back to the
    // default style is fine if a future indicatif version tightens parsing.
    ProgressStyle::with_template(DOWNLOAD_TEMPLATE)
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars("=> ")
}

fn spinner_style() -> ProgressStyle {
    ProgressStyle::with_template(SPINNER_TEMPLATE)
        .unwrap_or_else(|_| ProgressStyle::default_spinner())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn new_download_bar_sets_prefix() {
        let pb = new_download_bar("test-prefix");
        assert_eq!(pb.prefix(), "test-prefix");
    }

    #[test]
    fn hidden_bar_does_not_render() {
        let pb = hidden_bar();
        pb.set_position(50);
        pb.finish();
        // No assertion possible on stderr; hidden() is a no-op renderer.
    }

    #[test]
    fn add_download_bar_attaches_to_multiprogress() {
        let mp = MultiProgress::new();
        let pb = add_download_bar(&mp, "child");
        assert_eq!(pb.prefix(), "child");
    }

    #[test]
    fn switch_to_spinner_changes_style() {
        let pb = new_download_bar("a");
        switch_to_spinner(&pb);
        // No public API to read the style; just verify it doesn't panic.
        pb.set_position(100);
    }
}
