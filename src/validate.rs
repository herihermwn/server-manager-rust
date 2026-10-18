//! Input checks at the trust boundary. Every name or path that ends up in a command argument or a
//! file path goes through one of these first.

use crate::http::AppError;
use std::path::{Component, Path, PathBuf};

/// `^[a-z0-9-]{1,40}$`, and not starting with `-` (it would read as an option to git/systemctl).
pub fn app_name(s: &str) -> Result<&str, AppError> {
    let ok = (1..=40).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-');
    if ok {
        Ok(s)
    } else {
        Err(AppError::bad_request(
            "App name: 1-40 characters, a-z, 0-9 and -, not starting with -.",
        ))
    }
}

/// `^[A-Za-z_][A-Za-z0-9_]*$`, max 128.
pub fn env_key(s: &str) -> Result<&str, AppError> {
    let mut b = s.bytes();
    let ok = s.len() <= 128
        && b.next().is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && b.all(|c| c.is_ascii_alphanumeric() || c == b'_');
    if ok {
        Ok(s)
    } else {
        Err(AppError::bad_request(format!(
            "Invalid env key \"{s}\": use letters, digits and _, not starting with a digit."
        )))
    }
}

/// Resolve `p` and require it to be strictly inside `base` (the apps folder).
/// Rejects relative paths and any `..`; symlinks are resolved via the deepest existing ancestor,
/// so a link pointing outside `base` is caught too. `p` itself may not exist yet (new app folder).
pub fn path_under(base: &Path, p: &Path) -> Result<PathBuf, AppError> {
    let bad = |m: &str| AppError::bad_request(format!("{m} ({})", p.display()));
    if !p.is_absolute() {
        return Err(bad("Path must be absolute"));
    }
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(bad("Path must not contain .."));
    }
    let base = base.canonicalize().map_err(|_| bad("Apps folder does not exist"))?;
    let mut existing = p.to_path_buf();
    let mut rest = Vec::new();
    // symlink_metadata: a dangling link counts as existing, so canonicalize below rejects it.
    while existing.symlink_metadata().is_err() {
        rest.push(existing.file_name().ok_or_else(|| bad("Invalid path"))?.to_owned());
        existing.pop();
    }
    let mut full = existing.canonicalize().map_err(|_| bad("Path cannot be resolved"))?;
    full.extend(rest.iter().rev());
    if full.starts_with(&base) && full != base {
        Ok(full)
    } else {
        Err(bad(&format!("Path must be inside {}", base.display())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_names() {
        for ok in ["a", "radio-api-rs", "app1", &"x".repeat(40)] {
            assert!(app_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "-rf", "Radio", "a_b", "a.b", "a/b", "a b", "ä", &"x".repeat(41)] {
            assert!(app_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn env_keys() {
        for ok in ["DATABASE_URL", "_X", "port", "A1"] {
            assert!(env_key(ok).is_ok(), "{ok}");
        }
        for bad in ["", "1A", "A-B", "A B", "A=B", "Ä"] {
            assert!(env_key(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn paths() {
        let tmp = std::env::temp_dir().join(format!("sm-validate-{}", std::process::id()));
        let base = tmp.join("apps");
        std::fs::create_dir_all(base.join("one")).unwrap();
        let canon = base.canonicalize().unwrap();

        assert_eq!(path_under(&base, &base.join("one")).unwrap(), canon.join("one"));
        assert_eq!(path_under(&base, &base.join("new/sub")).unwrap(), canon.join("new/sub"));
        assert!(path_under(&base, &base).is_err(), "base itself");
        assert!(path_under(&base, &base.join("../etc")).is_err(), "..");
        assert!(path_under(&base, Path::new("apps/one")).is_err(), "relative");
        assert!(path_under(&base, Path::new("/etc/passwd")).is_err(), "outside");
        assert!(path_under(&base, &tmp.join("apps-evil")).is_err(), "prefix trick");

        std::os::unix::fs::symlink("/tmp", base.join("link")).unwrap();
        assert!(path_under(&base, &base.join("link/x")).is_err(), "symlink escape");
        std::os::unix::fs::symlink("/nonexistent-sm", base.join("dangling")).unwrap();
        assert!(path_under(&base, &base.join("dangling/x")).is_err(), "dangling symlink");
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
