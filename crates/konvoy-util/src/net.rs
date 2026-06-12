//! The single funnel for outbound HTTP requests.
//!
//! Every byte Konvoy sends or receives over the network goes through
//! `NetworkClient::get` — toolchain tarballs, plugin JARs, dependency
//! klibs, detekt releases, POMs, and `.module` metadata alike. The raw
//! request method is `pub(crate)`, so code outside `konvoy-util` *cannot*
//! make an HTTP request except via this crate's higher-level fetchers
//! (`progress::fetch`, `pom::fetch_pom`, …), all of which take a
//! `&NetworkClient`.
//!
//! This is what makes `--offline` a structural guarantee rather than a
//! per-call-site convention: an offline client refuses the request at the
//! wire, before any connection attempt, no matter which path asked.
//! Engine-level gates still fail fast earlier with friendlier, artifact-
//! specific errors — this is the floor beneath them.

/// Owns Konvoy's outbound network access and the offline policy.
///
/// Construct ONCE at the program entry point (`konvoy-cli`'s `main`) and thread
/// it down to everything that may fetch — builds, lint, `konvoy update`, and
/// `konvoy toolchain install` all receive the same client. Do not construct
/// ad-hoc clients deep inside resolution paths (or per-command): that
/// reintroduces the per-site-policy problem this type exists to end.
#[derive(Debug, Clone, Copy)]
pub struct NetworkClient {
    offline: bool,
}

/// Outcome of a refused or failed request, before any domain mapping.
///
/// Kept `pub(crate)` (like [`NetworkClient::get`]) so ureq never leaks out of
/// this crate; call sites map these onto `UtilError` with their own context.
/// `Status`/`Transport` carry the original error's display string so existing
/// error-message text is preserved exactly.
#[derive(Debug)]
pub(crate) enum RequestError {
    /// The client is offline; the request was refused before any network I/O.
    Offline,
    /// The server answered with a non-success HTTP status.
    Status { code: u16, message: String },
    /// DNS/connect/read or any other transport-level failure.
    Transport { message: String },
}

impl NetworkClient {
    /// Create a client. `offline = true` makes every `get` fail with
    /// `RequestError::Offline` without touching the network.
    #[must_use]
    pub const fn new(offline: bool) -> Self {
        Self { offline }
    }

    /// Whether this client refuses network access. Engine gates read this to
    /// fail fast with artifact-specific errors before the wire-level refusal.
    #[must_use]
    pub const fn is_offline(&self) -> bool {
        self.offline
    }

    /// Issue a GET request — the one place in the workspace where an outbound
    /// HTTP request is made.
    ///
    /// Uses a 30-second connect timeout and the supplied global timeout
    /// (artifact downloads pass 600s, metadata fetches 60s, matching the agent
    /// configuration this replaces). A fresh agent per call mirrors the
    /// previous behavior — no connection pooling existed before either.
    pub(crate) fn get(
        &self,
        url: &str,
        global_timeout_secs: u64,
    ) -> Result<ureq::http::Response<ureq::Body>, RequestError> {
        if self.offline {
            return Err(RequestError::Offline);
        }
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .timeout_connect(Some(std::time::Duration::from_secs(30)))
                .timeout_global(Some(std::time::Duration::from_secs(global_timeout_secs)))
                .build(),
        );
        agent.get(url).call().map_err(|e| {
            let message = e.to_string();
            match e {
                ureq::Error::StatusCode(code) => RequestError::Status { code, message },
                _ => RequestError::Transport { message },
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn offline_client_refuses_get_without_touching_network() {
        // The URL points at a port that would hang or refuse; an offline client
        // must return Offline immediately, before any connection attempt.
        let client = NetworkClient::new(true);
        let result = client.get("http://127.0.0.1:1/never-contacted", 5);
        assert!(
            matches!(result, Err(RequestError::Offline)),
            "offline client must refuse with RequestError::Offline"
        );
    }

    #[test]
    fn online_client_maps_transport_errors() {
        // Port 1 refuses connections — an online client surfaces a Transport
        // error (not Offline, not a panic).
        let client = NetworkClient::new(false);
        let result = client.get("http://127.0.0.1:1/unreachable", 5);
        assert!(
            matches!(result, Err(RequestError::Transport { .. })),
            "connection refused must map to Transport, got: {result:?}"
        );
    }

    #[test]
    fn is_offline_reports_construction_flag() {
        assert!(NetworkClient::new(true).is_offline());
        assert!(!NetworkClient::new(false).is_offline());
    }
}
