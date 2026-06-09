//! Naming / identifier validation helpers shared across crates.

/// Validate a dot-separated Kotlin/Java package name.
///
/// Each segment must be a non-empty identifier starting with an ASCII letter or
/// underscore and containing only ASCII alphanumerics or underscores (e.g.
/// `com.example.api`). An empty string, an empty segment (`com..example`), or a
/// segment starting with a digit (`com.1api`) is rejected.
#[must_use]
pub fn is_valid_package_name(pkg: &str) -> bool {
    pkg.split('.').all(|segment| {
        let mut chars = segment.chars();
        match chars.next() {
            Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
            _ => return false,
        }
        chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_package_names() {
        assert!(is_valid_package_name("com.example.api"));
        assert!(is_valid_package_name("com.example._internal.api2"));
        assert!(is_valid_package_name("a"));
        assert!(is_valid_package_name("_a.b_c.D9"));
    }

    #[test]
    fn rejects_invalid_package_names() {
        assert!(!is_valid_package_name("")); // empty
        assert!(!is_valid_package_name("com..example")); // empty segment
        assert!(!is_valid_package_name("com.example.")); // trailing dot -> empty segment
        assert!(!is_valid_package_name("com.1api")); // segment starts with a digit
        assert!(!is_valid_package_name("-com.example")); // starts with a hyphen
        assert!(!is_valid_package_name("com.exa mple")); // contains a space
    }
}
