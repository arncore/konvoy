//! Project scaffolding for `konvoy init`.

use std::path::Path;

use konvoy_config::manifest::{Manifest, Package, PackageKind, Toolchain};

use crate::error::EngineError;

/// Scaffold a new Konvoy project.
///
/// Creates the project directory (if it doesn't exist), a `konvoy.toml` manifest,
/// and a `src/main.kt` with a hello-world program.
///
/// # Errors
/// Returns an error if:
/// - The project name is invalid (empty, contains special characters, etc.)
/// - A `konvoy.toml` already exists in `dir`
/// - The directory or files cannot be created
/// - The manifest cannot be serialized
pub fn init_project(name: &str, dir: &Path) -> Result<(), EngineError> {
    init_project_with_kind(name, dir, PackageKind::Bin)
}

/// Validate that a project name is well-formed.
///
/// A valid project name:
/// - Is not empty
/// - Contains only ASCII alphanumeric characters, hyphens (`-`), or underscores (`_`)
/// - Starts with a letter or underscore (not a digit or hyphen)
fn validate_project_name(name: &str) -> Result<(), EngineError> {
    let Some(first) = name.chars().next() else {
        return Err(EngineError::InvalidProjectName {
            name: name.to_owned(),
            reason: "name must not be empty".to_owned(),
        });
    };

    if !first.is_ascii_alphabetic() && first != '_' {
        return Err(EngineError::InvalidProjectName {
            name: name.to_owned(),
            reason: format!("must start with a letter or underscore, found '{first}'"),
        });
    }

    if let Some(bad) = name
        .chars()
        .find(|c| !c.is_ascii_alphanumeric() && *c != '-' && *c != '_')
    {
        return Err(EngineError::InvalidProjectName {
            name: name.to_owned(),
            reason: format!(
                "contains invalid character '{bad}' — only ASCII letters, digits, hyphens, and underscores are allowed"
            ),
        });
    }

    Ok(())
}

/// Scaffold a new Konvoy project with a specific package kind.
///
/// # Errors
/// Returns an error if the project name is invalid, the project directory cannot be created,
/// or a manifest already exists.
pub fn init_project_with_kind(
    name: &str,
    dir: &Path,
    kind: PackageKind,
) -> Result<(), EngineError> {
    validate_project_name(name)?;

    let manifest_path = dir.join("konvoy.toml");

    if manifest_path.exists() {
        return Err(EngineError::ProjectExists {
            path: manifest_path.display().to_string(),
        });
    }

    // Create project directory and src/ subdirectory.
    let src_dir = dir.join("src");
    konvoy_util::fs::ensure_dir(&src_dir)?;

    // Generate and write konvoy.toml.
    let manifest = Manifest {
        package: Package {
            name: name.to_owned(),
            kind,
            version: if kind == PackageKind::Lib {
                Some("0.1.0".to_owned())
            } else {
                None
            },
            entrypoint: if kind == PackageKind::Lib {
                "src/lib.kt".to_owned()
            } else {
                "src/main.kt".to_owned()
            },
        },
        toolchain: Toolchain {
            kotlin: "2.1.0".to_owned(),
            detekt: None,
        },
        dependencies: std::collections::BTreeMap::new(),
    };
    let toml_content = manifest.to_toml()?;
    std::fs::write(&manifest_path, toml_content).map_err(|source| EngineError::Io {
        path: manifest_path.display().to_string(),
        source,
    })?;

    // Generate and write source file.
    let (source_name, source_content) = if kind == PackageKind::Lib {
        (
            "lib.kt",
            format!("// {name} library\n\nfun greet(who: String): String {{\n    return \"Hello, $who!\"\n}}\n"),
        )
    } else {
        (
            "main.kt",
            format!("fun main() {{\n    println(\"Hello, {name}!\")\n}}\n"),
        )
    };
    let source_path = src_dir.join(source_name);
    std::fs::write(&source_path, source_content).map_err(|source| EngineError::Io {
        path: source_path.display().to_string(),
        source,
    })?;

    // Generate .gitignore.
    let gitignore_path = dir.join(".gitignore");
    std::fs::write(&gitignore_path, ".konvoy/\n").map_err(|source| EngineError::Io {
        path: gitignore_path.display().to_string(),
        source,
    })?;

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn creates_project_structure() {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().join("my-app");

        init_project("my-app", &project_dir).unwrap();

        assert!(project_dir.join("konvoy.toml").exists());
        assert!(project_dir.join("src").join("main.kt").exists());
        assert!(project_dir.join(".gitignore").exists());
        let gitignore = fs::read_to_string(project_dir.join(".gitignore")).unwrap();
        assert!(gitignore.contains(".konvoy/"));
    }

    #[test]
    fn manifest_parses_back() {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().join("test-proj");

        init_project("test-proj", &project_dir).unwrap();

        let manifest = Manifest::from_path(&project_dir.join("konvoy.toml")).unwrap();
        assert_eq!(manifest.package.name, "test-proj");
        assert_eq!(manifest.package.entrypoint, "src/main.kt");
        assert_eq!(manifest.toolchain.kotlin, "2.1.0");
    }

    #[test]
    fn main_kt_contains_name() {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().join("hello");

        init_project("hello", &project_dir).unwrap();

        let content = fs::read_to_string(project_dir.join("src").join("main.kt")).unwrap();
        assert!(content.contains("Hello, hello!"));
        assert!(content.contains("fun main()"));
    }

    #[test]
    fn refuses_existing_project() {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().join("existing");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(project_dir.join("konvoy.toml"), "").unwrap();

        let result = init_project("existing", &project_dir);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("already exists"));
    }

    #[test]
    fn creates_parent_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().join("deep").join("nested").join("project");

        init_project("project", &project_dir).unwrap();

        assert!(project_dir.join("konvoy.toml").exists());
    }

    #[test]
    fn init_lib_creates_lib_kt() {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().join("my-lib");

        init_project_with_kind("my-lib", &project_dir, PackageKind::Lib).unwrap();

        assert!(project_dir.join("konvoy.toml").exists());
        assert!(project_dir.join("src").join("lib.kt").exists());
        assert!(!project_dir.join("src").join("main.kt").exists());

        let manifest = Manifest::from_path(&project_dir.join("konvoy.toml")).unwrap();
        assert_eq!(manifest.package.kind, PackageKind::Lib);
        assert_eq!(manifest.package.version.as_deref(), Some("0.1.0"));
        assert_eq!(manifest.package.entrypoint, "src/lib.kt");
    }

    #[test]
    fn init_bin_has_no_version() {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().join("my-bin");

        init_project("my-bin", &project_dir).unwrap();

        let manifest = Manifest::from_path(&project_dir.join("konvoy.toml")).unwrap();
        assert_eq!(manifest.package.kind, PackageKind::Bin);
        assert!(manifest.package.version.is_none());
    }

    // --- Project name validation tests ---

    #[test]
    fn valid_names_accepted() {
        let valid = ["my-app", "hello_world", "a", "test123", "_private", "App"];
        for name in valid {
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path().join(name);
            init_project(name, &dir).unwrap();
        }
    }

    #[test]
    fn rejects_empty_name() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("empty");
        let err = init_project("", &dir).unwrap_err().to_string();
        assert!(err.contains("must not be empty"), "got: {err}");
    }

    #[test]
    fn rejects_name_with_spaces() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("spaces");
        let err = init_project("hello world", &dir).unwrap_err().to_string();
        assert!(err.contains("invalid character"), "got: {err}");
    }

    #[test]
    fn rejects_name_with_special_chars() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("special");
        let err = init_project("test@#$", &dir).unwrap_err().to_string();
        assert!(err.contains("invalid character"), "got: {err}");
    }

    #[test]
    fn rejects_path_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("traversal");
        let err = init_project("../../../etc/test", &dir)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("invalid character") || err.contains("must start with"),
            "got: {err}"
        );
    }

    #[test]
    fn rejects_path_separator() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("sep");
        let err = init_project("a/b", &dir).unwrap_err().to_string();
        assert!(err.contains("invalid character"), "got: {err}");
    }

    #[test]
    fn rejects_name_starting_with_digit() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("digit");
        let err = init_project("1abc", &dir).unwrap_err().to_string();
        assert!(err.contains("must start with"), "got: {err}");
    }

    #[test]
    fn rejects_name_starting_with_hyphen() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("hyphen");
        let err = init_project("-start", &dir).unwrap_err().to_string();
        assert!(err.contains("must start with"), "got: {err}");
    }

    #[test]
    fn rejects_name_with_null_byte() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("null");
        let err = init_project("ab\0cd", &dir).unwrap_err().to_string();
        assert!(err.contains("invalid character"), "got: {err}");
    }

    #[test]
    fn rejects_name_with_backslash() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("backslash");
        let err = init_project("a\\b", &dir).unwrap_err().to_string();
        assert!(err.contains("invalid character"), "got: {err}");
    }

    #[test]
    fn validation_prevents_file_creation() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("should-not-exist");
        let _ = init_project("", &dir);
        assert!(!dir.exists(), "directory should not have been created");
    }
}
