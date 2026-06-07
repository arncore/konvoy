//! Styled `indicatif` progress bars for downloads.
//!
//! Centralises the look-and-feel so every place that downloads an artifact
//! (toolchain, detekt, plugins, Maven klibs) renders the same way and
//! `konvoy update`'s `MultiProgress` can host any of them as a child bar.

use std::borrow::Cow;
use std::fmt::{Debug, Write};
use std::io::{IsTerminal, Write as IoWrite};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use indicatif::{
    MultiProgress, ProgressBar, ProgressDrawTarget, ProgressState, ProgressStyle, TermLike,
};

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

const KONVOY_PROGRESS_ENV: &str = "KONVOY_PROGRESS";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProgressMode {
    Auto,
    Ansi,
    Plain,
}

struct StderrTerm {
    stderr: Mutex<std::io::Stderr>,
}

impl StderrTerm {
    fn new() -> Self {
        Self {
            stderr: Mutex::new(std::io::stderr()),
        }
    }

    fn width_from_env() -> u16 {
        terminal_width_from_columns(std::env::var("COLUMNS").ok().as_deref())
    }

    fn write_escape(&self, escape: &str) -> std::io::Result<()> {
        self.write_str(escape)
    }
}

impl Debug for StderrTerm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StderrTerm").finish_non_exhaustive()
    }
}

impl TermLike for StderrTerm {
    fn width(&self) -> u16 {
        Self::width_from_env()
    }

    fn height(&self) -> u16 {
        24
    }

    fn move_cursor_up(&self, n: usize) -> std::io::Result<()> {
        if n == 0 {
            return Ok(());
        }
        self.write_escape(&format!("\x1b[{n}A"))
    }

    fn move_cursor_down(&self, n: usize) -> std::io::Result<()> {
        if n == 0 {
            return Ok(());
        }
        self.write_escape(&format!("\x1b[{n}B"))
    }

    fn move_cursor_right(&self, n: usize) -> std::io::Result<()> {
        if n == 0 {
            return Ok(());
        }
        self.write_escape(&format!("\x1b[{n}C"))
    }

    fn move_cursor_left(&self, n: usize) -> std::io::Result<()> {
        if n == 0 {
            return Ok(());
        }
        self.write_escape(&format!("\x1b[{n}D"))
    }

    fn write_line(&self, s: &str) -> std::io::Result<()> {
        let mut stderr = self
            .stderr
            .lock()
            .map_err(|_| std::io::Error::other("stderr lock poisoned"))?;
        stderr.write_all(s.as_bytes())?;
        stderr.write_all(b"\n")
    }

    fn write_str(&self, s: &str) -> std::io::Result<()> {
        let mut stderr = self
            .stderr
            .lock()
            .map_err(|_| std::io::Error::other("stderr lock poisoned"))?;
        stderr.write_all(s.as_bytes())
    }

    fn clear_line(&self) -> std::io::Result<()> {
        self.write_escape("\r\x1b[2K")
    }

    fn flush(&self) -> std::io::Result<()> {
        self.stderr
            .lock()
            .map_err(|_| std::io::Error::other("stderr lock poisoned"))?
            .flush()
    }
}

fn progress_draw_target() -> ProgressDrawTarget {
    if current_progress_mode() == ProgressMode::Plain {
        return ProgressDrawTarget::hidden();
    }

    let default = ProgressDrawTarget::stderr();
    if !default.is_hidden() {
        return default;
    }

    if should_force_ansi_progress(
        std::io::stderr().is_terminal(),
        std::env::var(KONVOY_PROGRESS_ENV).ok().as_deref(),
    ) {
        return ProgressDrawTarget::term_like_with_hz(Box::new(StderrTerm::new()), 20);
    }

    ProgressDrawTarget::hidden()
}

fn current_progress_mode() -> ProgressMode {
    progress_mode_from_env(std::env::var(KONVOY_PROGRESS_ENV).ok().as_deref())
}

fn progress_mode_from_env(value: Option<&str>) -> ProgressMode {
    value.map_or(ProgressMode::Auto, |value| {
        if progress_env_requests_plain(value) {
            ProgressMode::Plain
        } else if progress_env_requests_ansi(value) {
            ProgressMode::Ansi
        } else {
            ProgressMode::Auto
        }
    })
}

fn terminal_width_from_columns(value: Option<&str>) -> u16 {
    value
        .and_then(|value| value.trim().parse::<u16>().ok())
        .filter(|width| *width > 0)
        .unwrap_or(80)
}

fn should_force_ansi_progress(stderr_is_terminal: bool, env_value: Option<&str>) -> bool {
    stderr_is_terminal || env_value.is_some_and(progress_env_requests_ansi)
}

fn progress_env_requests_ansi(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on" | "ansi" | "always"
    )
}

fn progress_env_requests_plain(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "plain" | "line" | "lines" | "log"
    )
}

#[derive(Clone, Debug)]
struct PlainProgressLine {
    prefix: String,
    prefix_width: usize,
}

impl PlainProgressLine {
    fn new(prefix: &str, prefix_width: usize) -> Option<Self> {
        (current_progress_mode() == ProgressMode::Plain).then(|| Self {
            prefix: prefix.to_owned(),
            prefix_width,
        })
    }

    fn print(&self, bar: &ProgressBar, state: u8) {
        eprintln!(
            "{}",
            plain_progress_line(
                &self.prefix,
                self.prefix_width,
                bar.position(),
                bar.length(),
                state,
            )
        );
    }
}

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
    plain_line: Option<PlainProgressLine>,
}

impl DownloadBar {
    /// Run a download function with this bar attached and finalise based on
    /// the result.
    ///
    /// The closure receives a `&ProgressBar`. Most callers won't use this
    /// directly — prefer the higher-level helpers [`fetch`] and
    /// [`stream_with_bar`] in this module, which adapt the lower-level
    /// callback-based primitives ([`crate::artifact::download_artifact`]
    /// and `crate::download::stream_download`) to a bar internally.
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
        self.bar.force_draw();
        self.print_plain_line(STATE_SUCCESS);
        self.bar.abandon();
    }

    /// Flip state to failure (❌ + 💥 over the truck, dust trail stays) AND
    /// stop the bar so its final frame is preserved.
    pub fn mark_failure(&self) {
        self.state.store(STATE_FAILURE, Ordering::Relaxed);
        self.bar.force_draw();
        self.print_plain_line(STATE_FAILURE);
        self.bar.abandon();
    }

    fn print_plain_line(&self, state: u8) {
        if let Some(line) = &self.plain_line {
            line.print(&self.bar, state);
        }
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
/// Thin UI-aware wrapper over `crate::download::stream_download` for
/// callers (toolchain tarballs, JRE tarballs) that just need to drop
/// bytes onto disk and get a hash back — no `expected_sha256`, no
/// atomic-rename dance, no `freshly_downloaded` flag. The bar is wired
/// up internally; pass `None` to suppress UI.
///
/// # Errors
/// Returns whatever `crate::download::stream_download` returns.
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
    let multi = new_multi_progress();
    let state = Arc::new(AtomicU8::new(STATE_IN_PROGRESS));
    let prefix = prefix.into();
    let plain_line = PlainProgressLine::new(prefix.as_ref(), DEFAULT_PREFIX_WIDTH);
    let pb = build_bar(&multi, prefix, DEFAULT_PREFIX_WIDTH, Arc::clone(&state));
    DownloadBar {
        bar: pb,
        state,
        plain_line,
    }
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
    let prefix = prefix.into();
    let plain_line = PlainProgressLine::new(prefix.as_ref(), prefix_width);
    let pb = build_bar(mp, prefix, prefix_width, Arc::clone(&state));
    DownloadBar {
        bar: pb,
        state,
        plain_line,
    }
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
    let multi = new_multi_progress();
    let prefix_width = labels.iter().map(String::len).max().unwrap_or(0);
    let bars: Vec<DownloadBar> = labels
        .into_iter()
        .map(|label| add_download_bar(&multi, label, prefix_width))
        .collect();
    (multi, bars)
}

/// Build a [`MultiProgress`] with Konvoy's draw target policy.
///
/// Indicatif hides its default terminal target when `TERM=dumb` or `NO_COLOR`
/// is set. IDE consoles commonly present that way while still supporting
/// cursor movement, so Konvoy forces a terminal-like stderr target for attended
/// stderr or when `KONVOY_PROGRESS=ansi` explicitly opts in. Progress stays
/// hidden when stderr is a real pipe/file and no opt-in is present.
#[must_use]
pub fn new_multi_progress() -> MultiProgress {
    MultiProgress::with_draw_target(progress_draw_target())
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
    let _ = write!(
        w,
        "{}",
        truck_bar_string(state.len().unwrap_or(0), state.pos(), current_state)
    );
}

fn truck_bar_string(len: u64, pos: u64, current_state: u8) -> String {
    let bar_width = BAR_LOGICAL_WIDTH;
    let mut output = String::new();

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
            output.push_str(ROAD_AHEAD);
        }
        output.push_str(edge_glyph);
        return output;
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
        output.push_str(glyph);
    }

    output
}

fn plain_progress_line(
    prefix: &str,
    prefix_width: usize,
    downloaded: u64,
    total: Option<u64>,
    state: u8,
) -> String {
    let indicator = match state {
        STATE_SUCCESS => SUCCESS_MARKER,
        STATE_FAILURE => FAILURE_MARKER,
        _ => IN_PROGRESS_MARKER,
    };
    let prefix = format!("{prefix:<prefix_width$}");
    let total_len = total.filter(|total| *total > 0);
    let total_label = total_len
        .map(format_bytes)
        .unwrap_or_else(|| "unknown".to_owned());
    format!(
        "  {indicator}  {prefix} [{}] {} / {total_label}",
        truck_bar_string(total_len.unwrap_or(downloaded), downloaded, state),
        format_bytes(downloaded),
    )
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    if bytes < KIB {
        format!("{bytes} B")
    } else if bytes < MIB {
        format_decimal_bytes(bytes, KIB, "KiB")
    } else {
        format_decimal_bytes(bytes, MIB, "MiB")
    }
}

fn format_decimal_bytes(bytes: u64, unit: u64, suffix: &str) -> String {
    let whole = bytes / unit;
    let fractional = ((bytes % unit) * 100) / unit;
    format!("{whole}.{fractional:02} {suffix}")
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
    fn progress_env_requests_ansi_for_explicit_truthy_values() {
        for value in ["1", "true", "TRUE", " yes ", "on", "ansi", "always"] {
            assert!(
                progress_env_requests_ansi(value),
                "{value:?} should opt into ANSI progress"
            );
        }
    }

    #[test]
    fn progress_env_does_not_request_ansi_for_empty_or_falsey_values() {
        for value in ["", "0", "false", "no", "off", "never", "auto", "plain"] {
            assert!(
                !progress_env_requests_ansi(value),
                "{value:?} should not opt into ANSI progress"
            );
        }
    }

    #[test]
    fn progress_env_requests_plain_for_plain_line_values() {
        for value in ["plain", "PLAIN", " line ", "lines", "log"] {
            assert!(
                progress_env_requests_plain(value),
                "{value:?} should opt into plain progress"
            );
        }
    }

    #[test]
    fn progress_env_does_not_request_plain_for_ansi_or_falsey_values() {
        for value in ["", "0", "false", "ansi", "always", "true"] {
            assert!(
                !progress_env_requests_plain(value),
                "{value:?} should not opt into plain progress"
            );
        }
    }

    #[test]
    fn should_force_ansi_progress_for_attended_stderr_without_env() {
        assert!(should_force_ansi_progress(true, None));
    }

    #[test]
    fn should_force_ansi_progress_for_explicit_ide_opt_in_without_tty() {
        assert!(should_force_ansi_progress(false, Some("ansi")));
    }

    #[test]
    fn should_not_force_ansi_progress_for_unattended_stderr_without_opt_in() {
        assert!(!should_force_ansi_progress(false, None));
        assert!(!should_force_ansi_progress(false, Some("")));
        assert!(!should_force_ansi_progress(false, Some("false")));
    }

    #[test]
    fn terminal_width_parses_valid_columns() {
        assert_eq!(terminal_width_from_columns(Some("120")), 120);
        assert_eq!(terminal_width_from_columns(Some(" 132 ")), 132);
    }

    #[test]
    fn terminal_width_falls_back_for_missing_invalid_or_zero_columns() {
        for value in [None, Some(""), Some("0"), Some("-1"), Some("wide")] {
            assert_eq!(terminal_width_from_columns(value), 80);
        }
    }

    #[test]
    fn plain_progress_line_renders_success_with_truck_bar() {
        let line = plain_progress_line("dep", 8, 1024, Some(1024), STATE_SUCCESS);

        assert!(line.contains("✅"));
        assert!(line.contains("dep     "));
        assert!(line.contains("🚚"));
        assert!(line.contains("🟩"));
        assert!(line.contains("1.00 KiB / 1.00 KiB"));
    }

    #[test]
    fn plain_progress_line_treats_zero_total_as_unknown() {
        let line = plain_progress_line("dep", 3, 512, Some(0), STATE_SUCCESS);

        assert!(line.contains("512 B / unknown"));
        assert!(
            !line.contains("512 B / 0 B"),
            "zero-length progress bars are placeholders, not real totals"
        );
    }

    #[test]
    fn plain_progress_line_renders_failure_with_crash_bar() {
        let line = plain_progress_line("dep", 3, 128, Some(1024), STATE_FAILURE);

        assert!(line.contains("❌"));
        assert!(line.contains("💥"));
        assert!(line.contains("128 B / 1.00 KiB"));
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

    /// `fetch` should short-circuit on a cache hit and never touch the
    /// (bogus) URL. The dest file is pre-written so `check_cached` returns
    /// `Some` before any network attempt.
    #[test]
    fn fetch_returns_cached_when_hash_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("artifact.jar");
        let content = b"cached artifact content";
        std::fs::write(&dest, content).unwrap();
        let expected = crate::hash::sha256_bytes(content);

        let result = fetch(
            "http://127.0.0.1:1/should-not-be-hit",
            &dest,
            Some(&expected),
            "test",
            None,
        )
        .unwrap();

        assert!(!result.freshly_downloaded);
        assert_eq!(result.sha256, expected);
        assert_eq!(result.path, dest);
    }

    /// With no `expected_sha256`, an existing file is still served from
    /// cache — `check_cached` hashes it and returns `Some` with the
    /// computed hash. The bogus URL must not be hit.
    #[test]
    fn fetch_returns_cached_without_expected_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("artifact.jar");
        let content = b"unverified cached content";
        std::fs::write(&dest, content).unwrap();
        let computed = crate::hash::sha256_bytes(content);

        let result = fetch(
            "http://127.0.0.1:1/should-not-be-hit",
            &dest,
            None,
            "test",
            None,
        )
        .unwrap();

        assert!(!result.freshly_downloaded);
        assert_eq!(result.sha256, computed);
    }

    /// Cached file with the wrong hash returns `ArtifactHashMismatch`
    /// without falling through to the download path.
    #[test]
    fn fetch_errors_on_cached_hash_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("artifact.jar");
        std::fs::write(&dest, b"any content").unwrap();
        let bogus = "0".repeat(64);

        let result = fetch(
            "http://127.0.0.1:1/should-not-be-hit",
            &dest,
            Some(&bogus),
            "test",
            None,
        );
        assert!(matches!(
            result,
            Err(crate::error::UtilError::ArtifactHashMismatch { .. })
        ));
    }

    /// Cache-hit path doesn't touch the bar (no `run`, no state flip).
    /// The supplied `DownloadBar`'s state must stay `STATE_IN_PROGRESS`
    /// after a cache-hit fetch.
    #[test]
    fn fetch_cache_hit_leaves_bar_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("artifact.jar");
        let content = b"cached";
        std::fs::write(&dest, content).unwrap();
        let expected = crate::hash::sha256_bytes(content);

        let bar = new_download_bar("test");
        let result = fetch(
            "http://127.0.0.1:1/should-not-be-hit",
            &dest,
            Some(&expected),
            "test",
            Some(&bar),
        )
        .unwrap();

        assert!(!result.freshly_downloaded);
        assert_eq!(bar.state.load(Ordering::Relaxed), STATE_IN_PROGRESS);
    }
}
