//! Styled `indicatif` progress bars for downloads.
//!
//! Centralises the look-and-feel so every place that downloads an artifact
//! (toolchain, detekt, plugins, Maven klibs) renders the same way and
//! `konvoy update`'s `MultiProgress` can host any of them as a child bar.

use std::borrow::Cow;
use std::fmt::Write;

use indicatif::{MultiProgress, ProgressBar, ProgressState, ProgressStyle};

/// Default minimum prefix column width when the caller doesn't compute one
/// from the full set of in-flight downloads. 36 is wide enough to fit a
/// typical `"<dep> <version> [<target>]"` like
/// `"kotlinx-coroutines-core 1.9.0 [macos_arm64]"` (42 chars overflows but
/// that's only for single-download paths where alignment doesn't matter).
const DEFAULT_PREFIX_WIDTH: usize = 36;

/// Build a styled standalone [`ProgressBar`] for an HTTP download.
///
/// Used by single-shot download paths (toolchain install, detekt JAR, plugin
/// JAR) that render one bar at a time.
#[must_use]
pub fn new_download_bar(prefix: impl Into<Cow<'static, str>>) -> ProgressBar {
    let pb = ProgressBar::new(0);
    pb.set_style(download_style(DEFAULT_PREFIX_WIDTH));
    pb.set_prefix(prefix);
    pb
}

/// Attach a styled bar to a [`MultiProgress`] container with a caller-supplied
/// prefix column width.
///
/// `prefix_width` should be the max prefix length across every bar in the
/// container so all bar columns line up vertically. Bars added later than this
/// call still render correctly — they just don't influence the width.
#[must_use]
pub fn add_download_bar(
    mp: &MultiProgress,
    prefix: impl Into<Cow<'static, str>>,
    prefix_width: usize,
) -> ProgressBar {
    let pb = mp.add(ProgressBar::new(0));
    pb.set_style(download_style(prefix_width));
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
    pb.set_style(spinner_style(DEFAULT_PREFIX_WIDTH));
}

/// Custom `{elapsed_ms}` formatter — millisecond-precision elapsed time.
///
/// Indicatif's built-in `{elapsed}` rounds to seconds ("0s" for fast
/// downloads), which is too coarse for the typical sub-second klib fetches.
fn write_elapsed_ms(state: &ProgressState, w: &mut dyn Write) {
    let _ = write!(w, "{}ms", state.elapsed().as_millis());
}

fn download_style(prefix_width: usize) -> ProgressStyle {
    // 2-space indent matches Konvoy's existing eprintln output (e.g.
    // "  Resolved N dependencies..."). Bar width is capped so wide terminals
    // don't stretch a 50 KiB download across 200 columns. The prefix column
    // is left-padded to `prefix_width` so MultiProgress children line up.
    // Brackets around the bar give each row a visible boundary so a stack of
    // fully-filled bars doesn't merge into one solid block.
    // `bytes_per_sec` minimum width 12 fits "999.99 MiB/s" (longest typical
    // value) so the elapsed-time column stays right-aligned across rows.
    let template = format!(
        "  {{spinner:.green}} {{prefix:<{prefix_width}.bold}} [{{bar:40.green/dim}}] {{bytes:>10.cyan}} / {{total_bytes:<10.cyan}} {{bytes_per_sec:>12.yellow}} {{elapsed_ms:>7.dim}}"
    );
    ProgressStyle::with_template(&template)
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .with_key("elapsed_ms", write_elapsed_ms)
        .progress_chars("=> ")
}

fn spinner_style(prefix_width: usize) -> ProgressStyle {
    let template = format!(
        "  {{spinner:.green}} {{prefix:<{prefix_width}.bold}} {{bytes:>10.cyan}} {{bytes_per_sec:>12.yellow}} {{elapsed_ms:>7.dim}}"
    );
    ProgressStyle::with_template(&template)
        .unwrap_or_else(|_| ProgressStyle::default_spinner())
        .with_key("elapsed_ms", write_elapsed_ms)
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
    }

    #[test]
    fn add_download_bar_attaches_to_multiprogress() {
        let mp = MultiProgress::new();
        let pb = add_download_bar(&mp, "child", 32);
        assert_eq!(pb.prefix(), "child");
    }

    #[test]
    fn switch_to_spinner_changes_style() {
        let pb = new_download_bar("a");
        switch_to_spinner(&pb);
        pb.set_position(100);
    }
}
