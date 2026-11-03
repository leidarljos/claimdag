//! Where the graph lives, for every front end.
//!
//! A command line and a pane that disagree about which directory they mean are
//! two graphs, and the disagreement is silent: both open a `work.bin`, both
//! succeed, and neither shows the other's work.

use std::ffi::OsString;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::snap::SNAP_BIN;

/// Where the work graph lives when the caller does not say.
///
/// `CLAIMDAG_DIR`, then `$XDG_RUNTIME_DIR/claimdag`. Without either the
/// defaults decide: `$XDG_STATE_HOME/claimdag`, `~/.local/state/claimdag`,
/// `/tmp/claimdag-UID` and the old `/tmp/claimdag`, tried in that order. The
/// first that holds a graph this user owns is used, else the first that
/// applies, so a seat whose environment lacks one of them still reads the
/// graph another seat started in a later one. Every default is this user's
/// own: a shared `/tmp/claimdag` made one user's first claim fail on another
/// user's directory. The current directory is not a default: one graph per
/// seat is the point, and `.` puts a `work.bin` in whichever checkout
/// somebody happened to be standing in.
///
/// Unless `--dir` or `CLAIMDAG_DIR` named the directory, says on stderr which
/// graph it uses when another location holds one too, once.
#[must_use]
pub fn resolve_dir(explicit: Option<PathBuf>) -> PathBuf {
    let located = locate(explicit);
    located.tell_once(&mut std::io::stderr());
    located.dir().to_path_buf()
}

/// Every location this seat would look in, and the one it uses.
#[must_use]
pub fn locate(explicit: Option<PathBuf>) -> Located {
    locate_from(
        explicit,
        std::env::var_os("CLAIMDAG_DIR"),
        Fallbacks {
            runtime: std::env::var_os("XDG_RUNTIME_DIR"),
            state: std::env::var_os("XDG_STATE_HOME"),
            home: std::env::var_os("HOME"),
            uid: euid(),
            tmp: PathBuf::from(TMP),
        },
    )
}

/// Whether `dir` is the graph directory the defaults pick. Only such a
/// directory is checked for its owner; one named by `CLAIMDAG_DIR`, `--dir`
/// or the runtime directory is used as given.
#[must_use]
pub fn is_default_dir(dir: &Path) -> bool {
    let located = locate(None);
    located.used().source.is_default() && located.dir() == dir
}

/// Where `/tmp/claimdag-UID` and the old `/tmp/claimdag` live. Not `TMPDIR`:
/// that differs from one seat to the next on the same host, and every seat
/// has to arrive at the same directory.
const TMP: &str = "/tmp";

/// The file in the graph directory that lists the other locations holding
/// work when this seat last said so, so the line is said once, and again
/// only when that list changes.
const ELSEWHERE: &str = "elsewhere";

/// What a graph directory keeps besides its lock: the snapshot, the log of
/// forced calls the command line appends to, and the pane's names.
const KEPT: [&str; 3] = [SNAP_BIN, "forced.log", "handles.json"];

fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// The rule that put a location on the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Flag,
    Named,
    Runtime,
    StateHome,
    Home,
    Uid,
    Old,
}

impl Source {
    fn as_str(self) -> &'static str {
        match self {
            Self::Flag => "--dir",
            Self::Named => "CLAIMDAG_DIR",
            Self::Runtime => "XDG_RUNTIME_DIR",
            Self::StateHome => "XDG_STATE_HOME",
            Self::Home => "HOME",
            Self::Uid => "uid",
            Self::Old => "old default",
        }
    }

    /// A location the defaults pick, made private and checked for its
    /// owner, rather than one a flag or a variable names.
    fn is_default(self) -> bool {
        matches!(self, Self::StateHome | Self::Home | Self::Uid | Self::Old)
    }
}

#[derive(Debug, Clone)]
struct Place {
    dir: PathBuf,
    source: Source,
    /// Holds a `work.bin`; for a default, only in a directory this user owns.
    graph: bool,
}

/// Every location a seat would look in, in the order it tries them, and the
/// one it uses. Printed, one line per location: the directory, whether it is
/// in use, holds work or has no graph, and the rule that named it.
#[derive(Debug, Clone)]
pub struct Located {
    places: Vec<Place>,
    used: usize,
}

impl Located {
    /// The graph directory in use.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.used().dir
    }

    fn used(&self) -> &Place {
        &self.places[self.used]
    }

    /// The other directories that hold a graph, each once.
    fn others(&self) -> Vec<&Path> {
        let mut others: Vec<&Path> = Vec::new();
        for place in self.places.iter().filter(|p| p.graph) {
            if !same_dir(&place.dir, self.dir()) && !others.iter().any(|o| same_dir(o, &place.dir))
            {
                others.push(&place.dir);
            }
        }
        others
    }

    /// The first default, when the defaults picked a later one because only
    /// it held a graph: the old `/tmp/claimdag`, or a `/tmp/claimdag-UID` a
    /// seat with no home started.
    fn move_target(&self) -> Option<&Path> {
        let first = self.places.iter().find(|p| p.source.is_default())?;
        (self.used().source.is_default() && !same_dir(&first.dir, self.dir()))
            .then_some(first.dir.as_path())
    }

    /// Say which graph is in use and which other locations hold work, in one
    /// line, unless those are the ones this graph was already told about. A
    /// directory `--dir` or `CLAIMDAG_DIR` names is the caller's choice, and
    /// nothing is said about it.
    fn tell_once(&self, out: &mut impl Write) {
        if matches!(self.used().source, Source::Flag | Source::Named) {
            return;
        }
        let told = self.dir().join(ELSEWHERE);
        let others = self.others();
        if others.is_empty() {
            let _ = std::fs::remove_file(&told);
            return;
        }
        let list: String = others
            .iter()
            .map(|o| format!("{}\n", o.display()))
            .collect();
        if std::fs::read_to_string(&told).is_ok_and(|seen| seen == list) {
            return;
        }
        let names: Vec<String> = others.iter().map(|o| o.display().to_string()).collect();
        let _ = writeln!(
            out,
            "claimdag: using {}; also holding work: {}. claimdag where lists them",
            self.dir().display(),
            names.join(", ")
        );
        // A graph directory that does not exist yet keeps nothing, and the
        // next call says it again.
        let _ = std::fs::write(&told, list);
    }

    /// Move the graph in use into the first default, when the defaults
    /// picked a later one because only it held a graph. Returns where it was
    /// and where it is, or `None` when there was nothing to move.
    ///
    /// # Errors
    ///
    /// Fails when either directory cannot be made or locked, or a file in it
    /// cannot be moved.
    pub fn move_graph(&self) -> Result<Option<(PathBuf, PathBuf)>, String> {
        let Some(to) = self.move_target() else {
            return Ok(None);
        };
        let from = self.dir();
        let _from = crate::lock::lock_dir(from)?;
        crate::lock::prepare(to, true, euid())?;
        let _to = crate::lock::lock_dir(to)?;
        // Another seat may have moved it, or started a graph there, while
        // this one waited for the locks.
        if !from.join(SNAP_BIN).is_file() || to.join(SNAP_BIN).exists() {
            return Ok(None);
        }
        for name in KEPT {
            move_file(&from.join(name), &to.join(name))
                .map_err(|e| format!("{}: {e}", from.join(name).display()))?;
        }
        std::fs::File::open(to)
            .and_then(|d| d.sync_all())
            .map_err(|e| format!("{}: {e}", to.display()))?;
        Ok(Some((from.to_path_buf(), to.to_path_buf())))
    }
}

impl fmt::Display for Located {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for place in &self.places {
            let held = if same_dir(&place.dir, self.dir()) {
                "in use"
            } else if place.graph {
                "holds work"
            } else {
                "no graph"
            };
            writeln!(
                f,
                "{}  {held}  {}",
                place.dir.display(),
                place.source.as_str()
            )?;
        }
        if let Some(to) = self.move_target() {
            writeln!(
                f,
                "claimdag where --move moves the graph in use to {}",
                to.display()
            )?;
        }
        Ok(())
    }
}

/// Rename, or copy and remove where the two are on different filesystems, as
/// `/tmp` and the home directory often are. A file that is not there is
/// nothing to move, and one already at the far end is left where both are.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if !from.exists() || to.exists() {
        return Ok(());
    }
    match std::fs::rename(from, to) {
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            std::fs::copy(from, to)?;
            std::fs::File::open(to)?.sync_all()?;
            std::fs::remove_file(from)
        }
        moved => moved,
    }
}

/// Whether `dir` is a directory this user owns that holds a graph. A default
/// another user owns is theirs, whatever it holds.
fn holds_ours(dir: &Path, uid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(dir).is_ok_and(|m| m.is_dir() && m.uid() == uid)
        && dir.join(SNAP_BIN).is_file()
}

/// Whether two paths are one directory: the same path, or two that resolve to
/// it, as an old `/tmp/claimdag` linked to a newer default does.
fn same_dir(a: &Path, b: &Path) -> bool {
    a == b || std::fs::canonicalize(a).is_ok_and(|a| std::fs::canonicalize(b).is_ok_and(|b| a == b))
}

/// The places a default can come from, passed in so the rule can be checked.
struct Fallbacks {
    runtime: Option<OsString>,
    state: Option<OsString>,
    home: Option<OsString>,
    uid: u32,
    tmp: PathBuf,
}

/// A variable that names an absolute path; the XDG rules ignore the rest.
fn absolute(raw: Option<OsString>) -> Option<PathBuf> {
    raw.filter(|r| !r.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// The rule itself, with the environment passed in so it can be checked.
fn locate_from(explicit: Option<PathBuf>, named: Option<OsString>, env: Fallbacks) -> Located {
    let place = |dir: PathBuf, source: Source| {
        let graph = if source.is_default() {
            holds_ours(&dir, env.uid)
        } else {
            dir.join(SNAP_BIN).is_file()
        };
        Place { dir, source, graph }
    };
    let mut places: Vec<Place> = Vec::new();
    places.extend(explicit.map(|dir| place(dir, Source::Flag)));
    places.extend(
        named
            .filter(|raw| !raw.is_empty())
            .map(|raw| place(raw.into(), Source::Named)),
    );
    places.extend(absolute(env.runtime).map(|run| place(run.join("claimdag"), Source::Runtime)));
    let named_first = !places.is_empty();
    places.extend(absolute(env.state).map(|s| place(s.join("claimdag"), Source::StateHome)));
    places.extend(absolute(env.home).map(|h| place(h.join(".local/state/claimdag"), Source::Home)));
    places.push(place(
        env.tmp.join(format!("claimdag-{}", env.uid)),
        Source::Uid,
    ));
    places.extend(Some(place(env.tmp.join("claimdag"), Source::Old)).filter(|p| p.graph));
    let used = if named_first {
        0
    } else {
        places.iter().position(|p| p.graph).unwrap_or(0)
    };
    Located { places, used }
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
            tmp: PathBuf::from(TMP),
        }
    }

    fn dir_from(explicit: Option<PathBuf>, named: Option<OsString>, env: Fallbacks) -> PathBuf {
        locate_from(explicit, named, env).dir().to_path_buf()
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

    /// A home, a runtime directory and a `/tmp` of its own, so a test can put
    /// a graph in any default without touching the real ones.
    struct Host {
        root: PathBuf,
    }

    impl Host {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("claimdag-host-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("tmp")).unwrap();
            Self { root }
        }

        fn env(&self, runtime: bool, uid: u32) -> Fallbacks {
            Fallbacks {
                runtime: runtime.then(|| self.root.join("run").into()),
                state: None,
                home: Some(self.root.join("home").into()),
                uid,
                tmp: self.root.join("tmp"),
            }
        }

        fn dir(&self, which: &str) -> PathBuf {
            match which {
                "runtime" => self.root.join("run/claimdag"),
                "state" => self.root.join("home/.local/state/claimdag"),
                "uid" => self.root.join(format!("tmp/claimdag-{}", euid())),
                "old" => self.root.join("tmp/claimdag"),
                _ => unreachable!("no default called {which}"),
            }
        }

        fn graph(&self, which: &str) {
            let dir = self.dir(which);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(SNAP_BIN), b"").unwrap();
        }
    }

    impl Drop for Host {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn told(located: &Located) -> String {
        let mut out = Vec::new();
        located.tell_once(&mut out);
        String::from_utf8(out).unwrap()
    }

    /// Each combination of defaults holding a graph: the first that holds one
    /// is used, and the line names every other one, in the order they are
    /// tried.
    #[test]
    fn each_combination_of_default_graphs_picks_the_first_and_names_the_rest() {
        let cases: [(&[&str], &str); 8] = [
            (&[], "state"),
            (&["state"], "state"),
            (&["uid"], "uid"),
            (&["old"], "old"),
            (&["state", "uid"], "state"),
            (&["state", "old"], "state"),
            (&["uid", "old"], "uid"),
            (&["state", "uid", "old"], "state"),
        ];
        for (n, (held, used)) in cases.iter().enumerate() {
            let host = Host::new(&format!("combo-{n}"));
            for which in *held {
                host.graph(which);
            }
            let located = locate_from(None, None, host.env(false, euid()));
            assert_eq!(located.dir(), host.dir(used), "{held:?}");
            let others: Vec<String> = held
                .iter()
                .filter(|w| *w != used)
                .map(|w| host.dir(w).display().to_string())
                .collect();
            let line = if others.is_empty() {
                String::new()
            } else {
                format!(
                    "claimdag: using {}; also holding work: {}. claimdag where lists them\n",
                    host.dir(used).display(),
                    others.join(", ")
                )
            };
            assert_eq!(told(&located), line, "{held:?}");
        }
    }

    /// The line is said once, and again when another location starts
    /// holding work.
    #[test]
    fn the_line_is_said_once_until_another_location_holds_work() {
        let host = Host::new("once");
        host.graph("state");
        host.graph("old");
        let locate = || locate_from(None, None, host.env(false, euid()));
        assert!(told(&locate()).contains("also holding work"));
        assert_eq!(told(&locate()), "");
        host.graph("uid");
        let again = told(&locate());
        assert!(
            again.ends_with(&format!(
                "{}, {}. claimdag where lists them\n",
                host.dir("uid").display(),
                host.dir("old").display()
            )),
            "{again}"
        );
        assert_eq!(told(&locate()), "");
    }

    /// The runtime directory still comes first, and the seat that uses it
    /// hears about the graph a seat outside the session keeps.
    #[test]
    fn a_seat_in_a_session_hears_about_the_graph_outside_it() {
        let host = Host::new("runtime");
        host.graph("state");
        let located = locate_from(None, None, host.env(true, euid()));
        assert_eq!(located.dir(), host.dir("runtime"));
        assert_eq!(
            told(&located),
            format!(
                "claimdag: using {}; also holding work: {}. claimdag where lists them\n",
                host.dir("runtime").display(),
                host.dir("state").display()
            )
        );
        assert_eq!(located.move_graph(), Ok(None));
    }

    /// A directory the caller named is used, and nothing is said about the
    /// defaults.
    #[test]
    fn a_named_directory_hears_nothing_about_the_defaults() {
        let host = Host::new("named");
        host.graph("state");
        host.graph("old");
        let named = host.root.join("team");
        for (explicit, var) in [
            (Some(named.clone()), None),
            (None, Some(named.clone().into())),
        ] {
            let located = locate_from(explicit, var, host.env(true, euid()));
            assert_eq!(located.dir(), named);
            assert_eq!(told(&located), "");
            assert!(located.to_string().contains(&format!(
                "{}  holds work  HOME\n",
                host.dir("state").display()
            )));
        }
    }

    /// A default another user owns is theirs, whatever it holds.
    #[test]
    fn a_graph_another_user_owns_is_not_this_seats() {
        let host = Host::new("theirs");
        let other = euid() + 1;
        for which in ["state", "old"] {
            host.graph(which);
        }
        let located = locate_from(None, None, host.env(false, other));
        assert_eq!(located.dir(), host.dir("state"));
        assert_eq!(told(&located), "");
        assert!(!located.to_string().contains("old default"));
    }

    /// An old `/tmp/claimdag` linked to the new default is one graph, not
    /// two.
    #[test]
    fn a_link_to_the_graph_in_use_is_the_same_graph() {
        let host = Host::new("link");
        host.graph("state");
        std::os::unix::fs::symlink(host.dir("state"), host.dir("old")).unwrap();
        let located = locate_from(None, None, host.env(false, euid()));
        assert_eq!(located.dir(), host.dir("state"));
        assert_eq!(told(&located), "");
        assert!(located.to_string().contains(&format!(
            "{}  in use  old default\n",
            host.dir("old").display()
        )));
    }

    /// The graph under the old default moves into the first one, with the
    /// files beside it, and is then the graph in use.
    #[test]
    fn the_old_default_moves_into_the_first() {
        use std::os::unix::fs::PermissionsExt;
        let host = Host::new("move");
        host.graph("old");
        std::fs::write(host.dir("old").join("forced.log"), "a line\n").unwrap();
        let located = locate_from(None, None, host.env(false, euid()));
        assert_eq!(located.dir(), host.dir("old"));
        assert_eq!(
            located.move_graph(),
            Ok(Some((host.dir("old"), host.dir("state"))))
        );
        let after = locate_from(None, None, host.env(false, euid()));
        assert_eq!(after.dir(), host.dir("state"));
        assert_eq!(told(&after), "");
        assert!(!after.to_string().contains("old default"));
        assert!(!after.to_string().contains("--move"));
        assert_eq!(
            std::fs::read_to_string(host.dir("state").join("forced.log")).unwrap(),
            "a line\n"
        );
        let mode = std::fs::metadata(host.dir("state"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        assert_eq!(after.move_graph(), Ok(None));
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
        let state = home.join(".local/state/claimdag");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(state.join("work.bin"), b"").unwrap();
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

    /// Only the directory the defaults pick is checked for its owner; one
    /// a variable names is used as given, even where it matches a default.
    #[test]
    fn only_a_default_directory_is_a_default() {
        with_env(None, None, |home| {
            assert!(is_default_dir(&home.join(".local/state/claimdag")));
            assert!(!is_default_dir(std::path::Path::new("/shared/team/claims")));
        });
        with_env(Some("/shared/team/claims"), None, |home| {
            assert!(!is_default_dir(std::path::Path::new("/shared/team/claims")));
            assert!(!is_default_dir(&home.join(".local/state/claimdag")));
        });
        with_env(None, Some("/run/user/1000"), |_| {
            assert!(!is_default_dir(std::path::Path::new(
                "/run/user/1000/claimdag"
            )));
        });
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
            assert_eq!(locate(None).dir(), Path::new("/run/user/1000/claimdag"));
        });
        with_env(None, None, |home| {
            assert_eq!(resolve_dir(None), home.join(".local/state/claimdag"));
        });
        with_env(Some(""), Some(""), |home| {
            assert_eq!(resolve_dir(None), home.join(".local/state/claimdag"));
        });
    }
}
