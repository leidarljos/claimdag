//! Where the graph lives, for every front end.
//!
//! A command line and a pane that disagree about which directory they mean are
//! two graphs, and the disagreement is silent: both open a `work.bin`, both
//! succeed, and neither shows the other's work.

use std::ffi::OsString;
use std::path::PathBuf;

/// Where the work graph lives when the caller does not say.
///
/// `CLAIMDAG_DIR`, then the runtime directory, then the user's state
/// directory (`$XDG_STATE_HOME`, else `~/.local/state`), and only then
/// `/tmp/claimdag-UID`. Every default is this user's own: a shared
/// `/tmp/claimdag` made one user's first claim fail on another user's
/// directory. The current directory is not a default: one graph per seat is
/// the point, and `.` puts a `work.bin` in whichever checkout somebody
/// happened to be standing in.
///
/// A `/tmp/claimdag` this user already owns is still read while the state
/// directory has no graph, so a seat that claimed under the old default
/// keeps its graph.
#[must_use]
pub fn resolve_dir(explicit: Option<PathBuf>) -> PathBuf {
    // SAFETY: geteuid has no preconditions and cannot fail.
    let uid = unsafe { libc::geteuid() };
    let runtime = std::env::var_os("XDG_RUNTIME_DIR");
    let defaulted = explicit.is_none()
        && std::env::var_os("CLAIMDAG_DIR").is_none_or(|v| v.is_empty())
        && absolute(runtime.clone()).is_none();
    let picked = dir_from(
        explicit,
        std::env::var_os("CLAIMDAG_DIR"),
        Fallbacks {
            runtime,
            state: std::env::var_os("XDG_STATE_HOME"),
            home: std::env::var_os("HOME"),
            uid,
        },
    );
    if defaulted && !picked.exists() && legacy_is_ours(uid) {
        return PathBuf::from(LEGACY_DIR);
    }
    picked
}

/// The default before every default was per user.
const LEGACY_DIR: &str = "/tmp/claimdag";

/// Whether the old shared default is a directory this user owns.
fn legacy_is_ours(uid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(LEGACY_DIR).is_ok_and(|m| m.is_dir() && m.uid() == uid)
}

/// The places a default can come from, passed in so the rule can be checked.
struct Fallbacks {
    runtime: Option<OsString>,
    state: Option<OsString>,
    home: Option<OsString>,
    uid: u32,
}

/// A variable that names an absolute path; the XDG rules ignore the rest.
fn absolute(raw: Option<OsString>) -> Option<PathBuf> {
    raw.filter(|r| !r.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// The rule itself, with the environment passed in so it can be checked.
fn dir_from(explicit: Option<PathBuf>, named: Option<OsString>, env: Fallbacks) -> PathBuf {
    if let Some(dir) = explicit {
        return dir;
    }
    if let Some(raw) = named.filter(|raw| !raw.is_empty()) {
        return PathBuf::from(raw);
    }
    absolute(env.runtime)
        .or_else(|| absolute(env.state))
        .or_else(|| absolute(env.home).map(|h| h.join(".local").join("state")))
        .map_or_else(
            || PathBuf::from(format!("/tmp/claimdag-{}", env.uid)),
            |base| base.join("claimdag"),
        )
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn env(runtime: Option<&str>, state: Option<&str>, home: Option<&str>) -> Fallbacks {
        Fallbacks {
            runtime: runtime.map(Into::into),
            state: state.map(Into::into),
            home: home.map(Into::into),
            uid: 1234,
        }
    }

    #[test]
    fn an_explicit_directory_wins() {
        let asked = PathBuf::from("/somewhere/else");
        assert_eq!(
            dir_from(
                Some(asked.clone()),
                Some("/named".into()),
                env(Some("/run"), None, None)
            ),
            asked
        );
    }

    #[test]
    fn the_named_directory_comes_next() {
        assert_eq!(
            dir_from(None, Some("/named".into()), env(Some("/run"), None, None)),
            PathBuf::from("/named")
        );
    }

    #[test]
    fn an_empty_name_is_not_a_name() {
        assert_eq!(
            dir_from(None, Some(OsString::new()), env(Some("/run"), None, None)),
            PathBuf::from("/run/claimdag")
        );
    }

    /// With no runtime directory the default is still this
    /// user's own, never a `/tmp/claimdag` two users share.
    #[test]
    fn without_a_runtime_directory_the_default_is_still_per_user() {
        assert_eq!(
            dir_from(None, None, env(None, Some("/s"), Some("/home/u"))),
            PathBuf::from("/s/claimdag")
        );
        assert_eq!(
            dir_from(None, None, env(Some(""), Some("rel"), Some("/home/u"))),
            PathBuf::from("/home/u/.local/state/claimdag")
        );
        let picked = dir_from(None, None, env(None, None, None));
        assert_eq!(picked, PathBuf::from("/tmp/claimdag-1234"));
        assert_ne!(picked, PathBuf::from("/tmp/claimdag"));
        assert_ne!(picked, PathBuf::from("."));
    }

    /// Serialises the tests that write the two variables the resolver reads.
    ///
    /// Cargo runs these in one process across threads, and the variables are
    /// process globals, so without this three tests take turns writing the
    /// same two values and read whatever the interleaving leaves. That does
    /// not only invent failures: it hides real ones just as easily, since a
    /// resolver that ignored its input would still see the value some other
    /// test happened to set.
    static ENV: Mutex<()> = Mutex::new(());

    /// Run a closure with the two variables this resolver reads set.
    ///
    /// The lock is held across the set, the read, and the restore. Holding it
    /// only for the set would leave the read racing the next test's set, which
    /// is the whole failure.
    fn with_env<T>(
        claimdag: Option<&str>,
        runtime: Option<&str>,
        f: impl FnOnce(&std::path::Path) -> T,
    ) -> T {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let keys = ["CLAIMDAG_DIR", "XDG_RUNTIME_DIR", "XDG_STATE_HOME", "HOME"];
        let old: Vec<Option<OsString>> = keys.iter().map(std::env::var_os).collect();
        let set = |k: &str, v: Option<&str>| match v {
            Some(v) => std::env::set_var(k, v),
            None => std::env::remove_var(k),
        };
        set("CLAIMDAG_DIR", claimdag);
        set("XDG_RUNTIME_DIR", runtime);
        set("XDG_STATE_HOME", None);
        let home = std::env::temp_dir().join(format!("claimdag-seat-{}", std::process::id()));
        // A graph already in the state directory, so a `/tmp/claimdag` this
        // user happens to own does not stand in for it.
        std::fs::create_dir_all(home.join(".local/state/claimdag")).unwrap();
        set("HOME", home.to_str());
        let out = f(&home);
        let _ = std::fs::remove_dir_all(&home);
        for (k, v) in keys.iter().zip(old) {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
        out
    }

    #[test]
    fn the_flag_wins() {
        with_env(Some("/from/env"), Some("/run"), |_| {
            assert_eq!(
                resolve_dir(Some(PathBuf::from("/from/flag"))),
                PathBuf::from("/from/flag")
            );
        });
    }

    #[test]
    fn the_environment_comes_next() {
        with_env(Some("/from/env"), Some("/run"), |_| {
            assert_eq!(resolve_dir(None), PathBuf::from("/from/env"));
        });
    }

    #[test]
    fn the_runtime_directory_is_the_default_and_never_the_working_one() {
        // A work.bin in whichever checkout somebody was standing in is two
        // graphs, and the pane would be reading the other one.
        with_env(None, Some("/run/user/1000"), |_| {
            assert_eq!(resolve_dir(None), PathBuf::from("/run/user/1000/claimdag"));
        });
        with_env(None, None, |home| {
            assert_eq!(resolve_dir(None), home.join(".local/state/claimdag"));
        });
        with_env(Some(""), Some(""), |home| {
            assert_eq!(resolve_dir(None), home.join(".local/state/claimdag"));
        });
    }
}
