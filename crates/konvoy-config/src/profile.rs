//! Build profile (debug vs release).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Build profile.
///
/// Serialized as the wire strings `"debug"` / `"release"` via
/// `#[serde(rename_all = "lowercase")]` so `metadata.toml` and any future
/// config files stay compatible with the pre-enum stringly-typed format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    Debug,
    Release,
}

impl Profile {
    /// Returns the canonical wire string (`"debug"` or `"release"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Profile::Debug => "debug",
            Profile::Release => "release",
        }
    }
}

impl fmt::Display for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Errors returned when parsing a profile string.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("unknown profile `{name}` — expected `debug` or `release`")]
    Invalid { name: String },
}

impl FromStr for Profile {
    type Err = ProfileError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "debug" => Ok(Profile::Debug),
            "release" => Ok(Profile::Release),
            other => Err(ProfileError::Invalid {
                name: other.to_owned(),
            }),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn display_matches_wire_format() {
        assert_eq!(Profile::Debug.to_string(), "debug");
        assert_eq!(Profile::Release.to_string(), "release");
    }

    #[test]
    fn parse_round_trips() {
        assert_eq!("debug".parse::<Profile>().unwrap(), Profile::Debug);
        assert_eq!("release".parse::<Profile>().unwrap(), Profile::Release);
    }

    #[test]
    fn parse_rejects_unknown() {
        assert!("opt".parse::<Profile>().is_err());
        assert!("".parse::<Profile>().is_err());
    }

    #[test]
    fn serializes_as_plain_string_via_toml() {
        #[derive(Serialize, Deserialize)]
        struct Wrap {
            profile: Profile,
        }
        let s = toml::to_string(&Wrap {
            profile: Profile::Release,
        })
        .unwrap();
        assert!(
            s.contains("profile = \"release\""),
            "expected wire string `release`, got: {s}"
        );

        let back: Wrap = toml::from_str("profile = \"debug\"").unwrap();
        assert_eq!(back.profile, Profile::Debug);
    }
}
