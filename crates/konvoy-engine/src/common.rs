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
