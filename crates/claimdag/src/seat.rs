//! Where the graph lives, for every front end.
//!
//! A command line and a pane that disagree about which directory they mean are
//! two graphs, and the disagreement is silent: both open a `work.bin`, both
//! succeed, and neither shows the other's work.

use std::ffi::OsString;
use std::path::PathBuf;

/// Where the work graph lives when the caller does not say.
///
/// `CLAIMDAG_DIR`, then the runtime directory. The current directory is not a
/// default: one graph per seat is the point, and `.` puts a `work.bin` in
/// whichever checkout somebody happened to be standing in.
#[must_use]
pub fn resolve_dir(explicit: Option<PathBuf>) -> PathBuf {
    dir_from(
        explicit,
        std::env::var_os("CLAIMDAG_DIR"),
        std::env::var_os("XDG_RUNTIME_DIR"),
    )
}

/// The rule itself, with the environment passed in so it can be checked.
fn dir_from(
    explicit: Option<PathBuf>,
    named: Option<OsString>,
    runtime: Option<OsString>,
) -> PathBuf {
    if let Some(dir) = explicit {
        return dir;
    }
    if let Some(raw) = named.filter(|raw| !raw.is_empty()) {
        return PathBuf::from(raw);
    }
    let base = runtime
        .filter(|raw| !raw.is_empty())
        .map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
    base.join("claimdag")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_directory_wins() {
        let asked = PathBuf::from("/somewhere/else");
        assert_eq!(
            dir_from(
                Some(asked.clone()),
                Some("/named".into()),
                Some("/run".into())
            ),
            asked
        );
    }

    #[test]
    fn the_named_directory_comes_next() {
        assert_eq!(
            dir_from(None, Some("/named".into()), Some("/run".into())),
            PathBuf::from("/named")
        );
    }

    #[test]
    fn an_empty_name_is_not_a_name() {
        assert_eq!(
            dir_from(None, Some(OsString::new()), Some("/run".into())),
            PathBuf::from("/run/claimdag")
        );
    }

    #[test]
    fn the_last_resort_is_never_the_current_directory() {
        let picked = dir_from(None, None, None);
        assert_eq!(picked, PathBuf::from("/tmp/claimdag"));
        assert_ne!(picked, PathBuf::from("."));
    }

    /// Run a closure with the two variables this resolver reads set.
    fn with_env<T>(claimdag: Option<&str>, runtime: Option<&str>, f: impl FnOnce() -> T) -> T {
        let old_c = std::env::var_os("CLAIMDAG_DIR");
        let old_r = std::env::var_os("XDG_RUNTIME_DIR");
        match claimdag {
            Some(v) => std::env::set_var("CLAIMDAG_DIR", v),
            None => std::env::remove_var("CLAIMDAG_DIR"),
        }
        match runtime {
            Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
            None => std::env::remove_var("XDG_RUNTIME_DIR"),
        }
        let out = f();
        match old_c {
            Some(v) => std::env::set_var("CLAIMDAG_DIR", v),
            None => std::env::remove_var("CLAIMDAG_DIR"),
        }
        match old_r {
            Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
            None => std::env::remove_var("XDG_RUNTIME_DIR"),
        }
        out
    }

    #[test]
    fn the_flag_wins() {
        with_env(Some("/from/env"), Some("/run"), || {
            assert_eq!(
                resolve_dir(Some(PathBuf::from("/from/flag"))),
                PathBuf::from("/from/flag")
            );
        });
    }

    #[test]
    fn the_environment_comes_next() {
        with_env(Some("/from/env"), Some("/run"), || {
            assert_eq!(resolve_dir(None), PathBuf::from("/from/env"));
        });
    }

    #[test]
    fn the_runtime_directory_is_the_default_and_never_the_working_one() {
        // A work.bin in whichever checkout somebody was standing in is two
        // graphs, and the pane would be reading the other one.
        with_env(None, Some("/run/user/1000"), || {
            assert_eq!(resolve_dir(None), PathBuf::from("/run/user/1000/claimdag"));
        });
        with_env(None, None, || {
            assert_eq!(resolve_dir(None), PathBuf::from("/tmp/claimdag"));
        });
        with_env(Some(""), Some(""), || {
            assert_eq!(resolve_dir(None), PathBuf::from("/tmp/claimdag"));
        });
    }
}
