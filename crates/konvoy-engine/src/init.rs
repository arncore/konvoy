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
/// - A `konvoy.toml` already exists in `dir`
/// - The directory or files cannot be created
/// - The manifest cannot be serialized
pub fn init_project(name: &str, dir: &Path) -> Result<(), EngineError> {
    init_project_with_kind(name, dir, PackageKind::Bin)
}

/// Scaffold a new Konvoy project with a specific package kind.
///
/// # Errors
/// Returns an error if the project directory cannot be created or a manifest already exists.
pub fn init_project_with_kind(
    name: &str,
    dir: &Path,
    kind: PackageKind,
) -> Result<(), EngineError> {
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
            entrypoint: "src/main.kt".to_owned(),
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
}
