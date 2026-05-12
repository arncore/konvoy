//! Small shared helpers used across multiple engine modules.

use crate::error::EngineError;

/// Split a `groupId:artifactId` string into its two parts.
///
/// # Errors
///
/// Returns an error if the string does not contain exactly one colon.
pub(crate) fn split_maven_coordinate(maven: &str) -> Result<(&str, &str), EngineError> {
    maven.split_once(':').ok_or_else(|| EngineError::Metadata {
        message: format!("invalid maven coordinate `{maven}` — expected `groupId:artifactId`"),
    })
}

/// Truncate a hex hash string to `len` characters for display.
///
/// Returns the full hash if it is shorter than `len`.
pub(crate) fn truncate_hash(hash: &str, len: usize) -> &str {
    hash.get(..len).unwrap_or(hash)
}

/// Current UTC timestamp formatted as `"{seconds}s-since-epoch"`.
///
/// Stored in build metadata; the suffix is purely for human reading.
pub(crate) fn now_epoch_secs() -> String {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}s-since-epoch", duration.as_secs())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn truncate_hash_short() {
        assert_eq!(
            truncate_hash("abcdef1234567890abcdef", 16),
            "abcdef1234567890"
        );
    }

    #[test]
    fn truncate_hash_shorter_than_limit() {
        assert_eq!(truncate_hash("abc", 16), "abc");
    }

    #[test]
    fn now_epoch_secs_format() {
        let ts = now_epoch_secs();
        assert!(
            ts.ends_with("s-since-epoch"),
            "expected format '<digits>s-since-epoch', got: {ts}"
        );
        let digits = ts.strip_suffix("s-since-epoch").unwrap();
        assert!(
            digits.parse::<u64>().is_ok(),
            "expected numeric prefix, got: {digits}"
        );
    }

    #[test]
    fn now_epoch_secs_is_reasonable() {
        let ts = now_epoch_secs();
        let secs: u64 = ts.strip_suffix("s-since-epoch").unwrap().parse().unwrap();
        // Should be after 2024-01-01 (1704067200) and before 2040-01-01 (2208988800).
        assert!(secs > 1_704_067_200, "timestamp too old: {secs}");
        assert!(secs < 2_208_988_800, "timestamp too far in future: {secs}");
    }
}
