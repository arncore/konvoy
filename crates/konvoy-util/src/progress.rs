//! Styled `indicatif` progress bars for downloads.
//!
//! Centralises the look-and-feel so every place that downloads an artifact
//! (toolchain, detekt, plugins, Maven klibs) renders the same way and
//! `konvoy update`'s `MultiProgress` can host any of them as a child bar.

use std::borrow::Cow;
use std::fmt::Write;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressState, ProgressStyle};

/// Default minimum prefix column width when the caller doesn't compute one
/// from the full set of in-flight downloads. 36 is wide enough to fit a
/// typical `"<dep> <version> [<target>]"` like
/// `"kotlinx-coroutines-core 1.9.0 [macos_arm64]"`.
const DEFAULT_PREFIX_WIDTH: usize = 36;

/// Logical width of the progress bar (visible columns ≈ 2× this since each
/// position is a 2-column emoji).
const BAR_LOGICAL_WIDTH: usize = 20;

/// State indicator emoji rendered by the `{indicator}` template key.
const IN_PROGRESS_MARKER: &str = "\u{2699}\u{FE0F}"; // ⚙️ (gear)
const SUCCESS_MARKER: &str = "\u{2705}"; // ✅
const FAILURE_MARKER: &str = "\u{274C}"; // ❌

/// State values driving the indicator + trail-glyph rendering.
const STATE_IN_PROGRESS: u8 = 0;
const STATE_SUCCESS: u8 = 1;
const STATE_FAILURE: u8 = 2;

/// Glyphs used inside the bar.
const ROAD_AHEAD: &str = "\u{2B1C}"; // ⬜ empty road in front of truck
const TRUCK: &str = "\u{1F69A}"; // 🚚 the truck (edge / cursor)
const CRASH: &str = "\u{1F4A5}"; // 💥 crash glyph drawn over the truck on failure
const DUST_TRAIL: &str = "\u{1F4A8}"; // 💨 dust while the truck is moving
const DELIVERED_CARGO: &str = "\u{1F7E9}"; // 🟩 green block left after delivery

/// Tick interval driving the indicator's redraw cadence. 250ms is half the
/// 500ms blink period (computed inside [`write_indicator`]) so each on/off
/// boundary is hit at least once even with indicatif's 20Hz draw throttle.
const BLINK_TICK: Duration = Duration::from_millis(250);

/// Handle bundling a bar with the atomic flag that drives its glyph state.
///
/// Returned by [`add_download_bar`] / [`new_download_bar`]. The engine layer
/// finalises each bar via [`DownloadBar::finish`] (or the lower-level
/// [`DownloadBar::mark_success`] / [`DownloadBar::mark_failure`]) once the
/// download result is known.
///
/// Both constructors render through a [`MultiProgress`]. Callers are
/// responsible for emitting a single trailing `eprintln!()` after the bar
/// block so the next stderr line doesn't get concatenated onto the last
/// bar's row.
pub struct DownloadBar {
    bar: ProgressBar,
    state: Arc<AtomicU8>,
}

impl DownloadBar {
    /// Run a download function with this bar attached and finalise based on
    /// the result.
    ///
    /// The closure receives a `&ProgressBar`. Most callers won't use this
    /// directly — prefer the higher-level helpers [`fetch`] and
    /// [`stream_with_bar`] in this module, which adapt the lower-level
    /// callback-based primitives ([`crate::artifact::download_artifact`]
    /// and [`crate::download::stream_download`]) to a bar internally.
    /// When the closure returns, this bar's state flips to ✅ on `Ok` or
    /// ❌ on `Err`, and the bar is abandoned so its final frame stays on
    /// screen.
    ///
    /// This is the recommended way to use a `DownloadBar` — callers should
    /// not need to touch the inner `ProgressBar` directly.
    ///
    /// # Errors
    /// Returns whatever `Err` the closure produced, unchanged.
    pub fn run<F, T, E>(&self, f: F) -> Result<T, E>
    where
        F: FnOnce(&ProgressBar) -> Result<T, E>,
    {
        self.finish(f(&self.bar))
    }

    /// Finalise the bar based on a pre-computed download result.
    ///
    /// Most callers should prefer [`Self::run`], which wires both halves
    /// together via a closure. Use `finish` only when the result was
    /// produced separately and you just need to flip state.
    ///
    /// # Errors
    /// Returns whatever `Err` was passed in, unchanged.
    pub fn finish<T, E>(&self, result: Result<T, E>) -> Result<T, E> {
        match &result {
            Ok(_) => self.mark_success(),
            Err(_) => self.mark_failure(),
        }
        result
    }

    /// Flip state to success (✅ + 🟩 cargo) AND stop the bar so its final
    /// frame is preserved. Order matters: state must be set before
    /// `abandon` halts further redraws.
    pub fn mark_success(&self) {
        self.state.store(STATE_SUCCESS, Ordering::Relaxed);
        self.bar.abandon();
    }

    /// Flip state to failure (❌ + 💥 over the truck, dust trail stays) AND
    /// stop the bar so its final frame is preserved.
    pub fn mark_failure(&self) {
        self.state.store(STATE_FAILURE, Ordering::Relaxed);
        self.bar.abandon();
    }
}

/// Build a progress callback that drives a [`ProgressBar`] from the
/// `(downloaded, total)` events emitted by network primitives.
///
/// `set_length` only fires on the first `Some(total)` we see — repeated
/// calls each chunk would take indicatif's state lock unnecessarily.
fn bar_progress(pb: &ProgressBar) -> impl FnMut(u64, Option<u64>) + '_ {
    let mut length_set = false;
    move |downloaded, total| {
        if !length_set {
            if let Some(t) = total {
                pb.set_length(t);
                length_set = true;
            }
        }
        pb.set_position(downloaded);
    }
}

/// No-op progress callback for the cache-hit / hidden-bar path.
const fn ignore_progress(_: u64, _: Option<u64>) {}

/// Fetch a Maven-style artifact: check the cache first, download (with the
/// supplied bar) only on miss.
///
/// Composes [`crate::artifact::check_cached`] (no UI) and
/// [`crate::artifact::download_artifact`] (callback-driven) so engine code
/// can call one function instead of writing the match-on-cache pattern at
/// every site. The `bar` parameter is `Option` so callers can suppress UI
/// entirely (e.g. for already-known-cached items in a parallel batch).
///
/// # Errors
/// - [`crate::error::UtilError::ArtifactHashMismatch`] if a cached file
///   has the wrong hash, or the downloaded file has the wrong hash.
/// - Other [`crate::error::UtilError`] variants for I/O / network failures.
pub fn fetch(
    url: &str,
    dest: &std::path::Path,
    expected_sha256: Option<&str>,
    label: &str,
    bar: Option<&DownloadBar>,
) -> Result<crate::artifact::ArtifactResult, crate::error::UtilError> {
    if let Some(cached) = crate::artifact::check_cached(dest, expected_sha256)? {
        return Ok(cached);
    }
    let download = |on_progress: &mut dyn FnMut(u64, Option<u64>)| {
        crate::artifact::download_artifact(url, dest, expected_sha256, label, on_progress)
    };
    if let Some(b) = bar {
        b.run(|pb| download(&mut bar_progress(pb)))
    } else {
        download(&mut ignore_progress)
    }
}

/// Stream a URL to `dest` with an optional bar; returns the SHA-256.
///
/// Thin UI-aware wrapper over [`crate::download::stream_download`] for
/// callers (toolchain tarballs, JRE tarballs) that just need to drop
/// bytes onto disk and get a hash back — no `expected_sha256`, no
/// atomic-rename dance, no `freshly_downloaded` flag. The bar is wired
/// up internally; pass `None` to suppress UI.
///
/// # Errors
/// Returns whatever [`crate::download::stream_download`] returns.
pub fn stream_with_bar(
    url: &str,
    dest: &std::path::Path,
    bar: Option<&DownloadBar>,
) -> Result<String, crate::error::UtilError> {
    let download = |on_progress: &mut dyn FnMut(u64, Option<u64>)| {
        crate::download::stream_download(url, dest, on_progress)
    };
    if let Some(b) = bar {
        b.run(|pb| download(&mut bar_progress(pb)))
    } else {
        download(&mut ignore_progress)
    }
}

/// Build a styled [`DownloadBar`] for a single-shot download.
///
/// Constructs a local [`MultiProgress`] and attaches one bar to it. The
/// `MultiProgress` is dropped on return — that's safe because
/// `MultiProgress::add` clones the inner `Arc<MultiState>` into the bar's
/// draw target, so the state stays alive as long as the bar does.
///
/// Caller must emit a trailing `eprintln!()` after the download finishes
/// so the next stderr line isn't concatenated onto the bar's row.
#[must_use]
pub fn new_download_bar(prefix: impl Into<Cow<'static, str>>) -> DownloadBar {
    let multi = MultiProgress::new();
    let state = Arc::new(AtomicU8::new(STATE_IN_PROGRESS));
    let pb = build_bar(&multi, prefix, DEFAULT_PREFIX_WIDTH, Arc::clone(&state));
    DownloadBar { bar: pb, state }
}

/// Attach a styled bar to a caller-owned [`MultiProgress`] container.
///
/// `prefix_width` should be the max prefix length across every bar in the
/// container so all bar columns line up vertically. The caller is
/// responsible for emitting a trailing newline after the whole bar group.
#[must_use]
pub fn add_download_bar(
    mp: &MultiProgress,
    prefix: impl Into<Cow<'static, str>>,
    prefix_width: usize,
) -> DownloadBar {
    let state = Arc::new(AtomicU8::new(STATE_IN_PROGRESS));
    let pb = build_bar(mp, prefix, prefix_width, Arc::clone(&state));
    DownloadBar { bar: pb, state }
}

/// Build a [`MultiProgress`] + a stable-ordered `Vec<DownloadBar>` from
/// per-bar labels.
///
/// Computes the max label length once so every bar gets the same prefix
/// padding (so columns line up). The returned `MultiProgress` is held by
/// the caller; each returned bar internally clones its `Arc<MultiState>`
/// so dropping the `MultiProgress` early would still leave the bars
/// rendering, but holding it is more explicit and lets the caller add
/// more bars later if needed.
///
/// Caller emits a trailing `eprintln!()` after the bar block.
#[must_use]
pub fn pre_allocate_bars(labels: Vec<String>) -> (MultiProgress, Vec<DownloadBar>) {
    let multi = MultiProgress::new();
    let prefix_width = labels.iter().map(String::len).max().unwrap_or(0);
    let bars: Vec<DownloadBar> = labels
        .into_iter()
        .map(|label| add_download_bar(&multi, label, prefix_width))
        .collect();
    (multi, bars)
}

/// Shared bar construction: attach to `mp`, apply the styled template,
/// set the prefix, and start the blink ticker.
fn build_bar(
    mp: &MultiProgress,
    prefix: impl Into<Cow<'static, str>>,
    prefix_width: usize,
    state: Arc<AtomicU8>,
) -> ProgressBar {
    let pb = mp.add(ProgressBar::new(0));
    pb.set_style(download_style(prefix_width, state));
    pb.set_prefix(prefix);
    pb.enable_steady_tick(BLINK_TICK);
    pb
}

/// Hidden bar for use in tests or non-interactive contexts.
#[must_use]
pub fn hidden_bar() -> ProgressBar {
    ProgressBar::hidden()
}

/// Custom `{elapsed_ms}` formatter — millisecond-precision elapsed time.
///
/// Indicatif's built-in `{elapsed}` rounds to seconds ("0s" for fast
/// downloads), which is too coarse for the typical sub-second klib fetches.
fn write_elapsed_ms(state: &ProgressState, w: &mut dyn Write) {
    let _ = write!(w, "{}ms", state.elapsed().as_millis());
}

/// Custom `{kbar}` renderer driven by the bar's state flag.
///
/// The bar's raw position tracks downloaded bytes (so `{bytes}` and
/// `{bytes_per_sec}` in the template stay accurate). Right-to-left fill is
/// done HERE by computing the edge index from `len - pos`:
///
///   start (pos=0):           `⬜⬜⬜⬜...⬜🚚`     (truck parked at right spot)
///   half (pos=len/2):        `⬜⬜⬜🚚💨💨💨💨`   (road ahead, truck, dust)
///   success (pos=len):       `🚚🟩🟩🟩🟩🟩🟩🟩`  (truck delivered)
///   failure (pos<len, frozen): same as half-state but truck → 💥
///
/// The truck is clamped to `bar_width - 1` so it's always visible — even at
/// pos == 0 (the moment before any bytes arrive), the truck appears at the
/// far right edge instead of being hidden off-screen.
fn render_truck_bar(state: &ProgressState, w: &mut dyn Write, current_state: u8) {
    let bar_width = BAR_LOGICAL_WIDTH;
    let len = state.len().unwrap_or(0);
    let pos = state.pos();

    if len == 0 {
        // No Content-Length recorded — either the spinner-mode case (download
        // succeeded but server didn't send a length) or an early connection
        // failure where the body never started. On failure we still want
        // 💥 visible, so park the crash glyph at the rightmost slot (the
        // truck's pre-departure position) with empty road to its left.
        let edge_glyph = if current_state == STATE_FAILURE {
            CRASH
        } else {
            ROAD_AHEAD
        };
        for _ in 0..bar_width.saturating_sub(1) {
            let _ = write!(w, "{ROAD_AHEAD}");
        }
        let _ = write!(w, "{edge_glyph}");
        return;
    }

    // Invert here: truck_at decreases as pos increases. At pos=0, truck_at =
    // bar_width-1 (far right). At pos=len, truck_at = 0 (far left). Cast via
    // u128 so multi-GB downloads don't overflow.
    let remaining = len.saturating_sub(pos);
    #[allow(clippy::cast_possible_truncation)]
    let truck_at = {
        let raw = (u128::from(remaining) * (bar_width as u128)) / u128::from(len);
        (raw as usize).min(bar_width.saturating_sub(1))
    };

    let trail = if current_state == STATE_SUCCESS {
        DELIVERED_CARGO
    } else {
        DUST_TRAIL
    };
    let edge_glyph = if current_state == STATE_FAILURE {
        CRASH
    } else {
        TRUCK
    };

    for i in 0..bar_width {
        let glyph = if i < truck_at {
            ROAD_AHEAD
        } else if i == truck_at {
            edge_glyph
        } else {
            trail
        };
        let _ = write!(w, "{glyph}");
    }
}

fn download_style(prefix_width: usize, state: Arc<AtomicU8>) -> ProgressStyle {
    // Konvoy-themed truck bar:
    //
    //   ⚙️  prefix [⬜⬜⬜🚚💨💨💨] bytes / total speed time
    //   ✅  prefix [🚚🟩🟩🟩🟩🟩🟩] bytes / total speed time
    //
    //   - {indicator} reads the shared atomic state to render ⚙️/✅/❌. While
    //     in progress, the gear blinks every 500ms (driven by
    //     `enable_steady_tick(BLINK_TICK)` on the bar). When state flips to
    //     success/failure, the renderer locks the indicator to ✅ or ❌.
    //   - {kbar} is our custom bar renderer (`render_truck_bar`) reading the
    //     same shared state to choose the trail glyph: 💨 dust while the
    //     truck is moving (or stopped on failure), 🟩 delivered cargo on
    //     success.
    //   - [`{kbar}`] uses plain `[` `]` brackets — they read as parking spots
    //     the truck pulls into when delivery completes.
    //   - `prefix_width` left-pads the dep label so columns line up.
    //   - `bytes_per_sec` min-width 12 fits "999.99 MiB/s" so the ms column
    //     stays right-aligned across rows.
    let template = format!(
        "  {{indicator}}  {{prefix:<{prefix_width}.bold}} [{{kbar}}] {{bytes:>10.cyan}} / {{total_bytes:<10.cyan}} {{bytes_per_sec:>12.yellow}} {{elapsed_ms:>7.dim}}"
    );
    let bar_state = Arc::clone(&state);
    let bar_closure = move |s: &ProgressState, w: &mut dyn Write| {
        render_truck_bar(s, w, bar_state.load(Ordering::Relaxed));
    };
    let indicator_state = state;
    let indicator_closure = move |s: &ProgressState, w: &mut dyn Write| {
        write_indicator(s, w, indicator_state.load(Ordering::Relaxed));
    };
    ProgressStyle::with_template(&template)
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .with_key("elapsed_ms", write_elapsed_ms)
        .with_key("indicator", indicator_closure)
        .with_key("kbar", bar_closure)
}

/// Render the leading state indicator: blinking ⚙️ while in-progress, static
/// ✅/❌ on completion.
///
/// The "off" frame is two spaces so the column width stays constant — the
/// emoji is 2 cells wide; blinking to "  " keeps everything to the right of
/// the indicator from shifting on each tick.
fn write_indicator(state: &ProgressState, w: &mut dyn Write, current_state: u8) {
    let glyph: &str = match current_state {
        STATE_SUCCESS => SUCCESS_MARKER,
        STATE_FAILURE => FAILURE_MARKER,
        _ => {
            // Blink every 500ms.
            let tick = state.elapsed().as_millis() / 500;
            if tick.is_multiple_of(2) {
                IN_PROGRESS_MARKER
            } else {
                "  "
            }
        }
    };
    let _ = write!(w, "{glyph}");
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn new_download_bar_sets_prefix() {
        let bar = new_download_bar("test-prefix");
        assert_eq!(bar.bar.prefix(), "test-prefix");
    }

    #[test]
    fn new_download_bar_starts_in_progress() {
        let bar = new_download_bar("x");
        assert_eq!(bar.state.load(Ordering::Relaxed), STATE_IN_PROGRESS);
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
        let bar = add_download_bar(&mp, "child", 32);
        assert_eq!(bar.bar.prefix(), "child");
        assert_eq!(bar.state.load(Ordering::Relaxed), STATE_IN_PROGRESS);
    }

    #[test]
    fn mark_success_flips_state() {
        let bar = new_download_bar("x");
        bar.mark_success();
        assert_eq!(bar.state.load(Ordering::Relaxed), STATE_SUCCESS);
    }

    #[test]
    fn mark_failure_flips_state() {
        let bar = new_download_bar("x");
        bar.mark_failure();
        assert_eq!(bar.state.load(Ordering::Relaxed), STATE_FAILURE);
    }

    #[test]
    fn finish_ok_flips_to_success_and_returns_result() {
        let bar = new_download_bar("x");
        let result: Result<u32, &str> = bar.finish(Ok(42));
        assert_eq!(result, Ok(42));
        assert_eq!(bar.state.load(Ordering::Relaxed), STATE_SUCCESS);
    }

    #[test]
    fn finish_err_flips_to_failure_and_returns_result() {
        let bar = new_download_bar("x");
        let result: Result<u32, &str> = bar.finish(Err("boom"));
        assert_eq!(result, Err("boom"));
        assert_eq!(bar.state.load(Ordering::Relaxed), STATE_FAILURE);
    }
}
