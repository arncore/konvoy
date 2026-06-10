//! Pure path-manipulation helpers (no filesystem I/O), shared across crates.
//!
//! These operate purely on the lexical structure of paths and never touch the
//! filesystem, so their results stay deterministic and portable across machines.

use std::path::{Component, Path, PathBuf};

/// Express `path` relative to `base`, in clean normalized form.
///
/// Both an absolute `path` under `base` and a `path` already relative to `base`
/// (possibly written with a leading `./`) collapse to the same
/// `specs/api.yaml`-style result: `path` is joined onto `base` and then the
/// `base` prefix is stripped, which also drops interior `.` components via
/// [`Path::components`]. A `path` that is absolute but *not* under `base` has no
/// common prefix to strip and is returned unchanged (a caller that requires
/// containment is expected to detect that, not have it silently rebased).
#[must_use]
pub fn relative_to(base: &Path, path: &Path) -> PathBuf {
    let joined = base.join(path);
    joined
        .strip_prefix(base)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| path.to_path_buf())
}

/// Whether `path` has any component, *below* `base`, whose name is hidden (starts
/// with `.`).
///
/// `base` is stripped first, so a hidden segment in `base` itself does not count —
/// only components below it. Useful for excluding OS/editor/VCS noise
/// (`.DS_Store`, `.git/`, swap files) and dotted output dirs (e.g. `.konvoy`) when
/// walking a directory. Purely lexical — no filesystem access.
#[must_use]
pub fn has_hidden_component_under(base: &Path, path: &Path) -> bool {
    path.strip_prefix(base)
        .unwrap_or(path)
        .components()
        .any(|c| matches!(c, Component::Normal(name) if name.to_string_lossy().starts_with('.')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_to_strips_base_from_absolute_paths_under_it() {
        assert_eq!(
            relative_to(Path::new("/proj"), Path::new("/proj/specs/api.yaml")),
            PathBuf::from("specs/api.yaml")
        );
    }

    #[test]
    fn relative_to_normalizes_relative_spellings_to_one_form() {
        let base = Path::new("/proj");
        // A leading `./`, the bare relative path, and the absolute-under-base form
        // all collapse to the same result — so callers can de-duplicate them.
        assert_eq!(
            relative_to(base, Path::new("./specs/api.yaml")),
            PathBuf::from("specs/api.yaml")
        );
        assert_eq!(
            relative_to(base, Path::new("specs/api.yaml")),
            relative_to(base, Path::new("/proj/specs/api.yaml"))
        );
    }

    #[test]
    fn relative_to_returns_absolute_outside_base_unchanged() {
        assert_eq!(
            relative_to(Path::new("/proj"), Path::new("/etc/passwd")),
            PathBuf::from("/etc/passwd")
        );
    }

    #[test]
    fn has_hidden_component_under_detects_dotfiles_below_base() {
        let base = Path::new("/proj/specs");
        assert!(has_hidden_component_under(
            base,
            Path::new("/proj/specs/.DS_Store")
        ));
        assert!(has_hidden_component_under(
            base,
            Path::new("/proj/specs/.git/HEAD")
        ));
        assert!(has_hidden_component_under(
            base,
            Path::new("/proj/specs/nested/.cache/x.yaml")
        ));
    }

    #[test]
    fn has_hidden_component_under_ignores_visible_paths_and_hidden_base() {
        let base = Path::new("/proj/specs");
        assert!(!has_hidden_component_under(
            base,
            Path::new("/proj/specs/api.yaml")
        ));
        assert!(!has_hidden_component_under(
            base,
            Path::new("/proj/specs/nested/pet.yaml")
        ));
        // A dotted segment in `base` itself must NOT count — only components below it.
        assert!(!has_hidden_component_under(
            Path::new("/proj/.hidden"),
            Path::new("/proj/.hidden/api.yaml")
        ));
    }
}
