//! One mutator at a time across processes: an advisory lock on the graph
//! directory, held from load to save.

use std::fs::{File, OpenOptions};
use std::os::unix::fs::FileExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::time::{Duration, Instant};

/// How long a writer waits for the lock before it names the holder and
/// fails. A holder keeps the lock for one load-to-save section, which at the
/// 4096-node cap took a median of 4.1 to 4.9 ms a run and 52 ms at the very
/// worst of 35,000 (`measurements/lock-wait.md`). A second is 200 median
/// sections back to back and 19 of the slowest, so a holder still at work
/// outlasts it only at the back of a queue that long; a stuck one costs each
/// call a second instead of stalling every seat behind it.
const WAIT: Duration = Duration::from_secs(1);

/// How long a waiting writer sleeps between tries. Under a quarter of the
/// median section, so a freed lock sits idle less than a quarter of a
/// section before the next writer has it; each try is one `flock` call.
const RETRY: Duration = Duration::from_millis(1);

/// The lock, released on drop. While it is held the file holds this
/// process's pid, so a writer left waiting can name it.
pub struct Lock {
    file: File,
}

impl Drop for Lock {
    fn drop(&mut self) {
        // Emptied before the descriptor closes, so a waiter is not sent
        // after a writer that has already finished.
        let _ = self.file.set_len(0);
    }
}

/// Take the exclusive lock on `dir/lock`, creating the directory, and write
/// this process's pid into it. Waits a bounded time while another process
/// holds it. A default location is made private. One another user owns is
/// still locked, and the warning from [`prepare`] is recorded.
///
/// # Errors
///
/// Fails when the directory or the lock file cannot be created, the lock
/// cannot be taken or is still held when the wait runs out (the one line
/// names the holder's pid and how to release it). A default location another
/// user owns is still locked, and [`prepare`] records `not-owner`.
pub fn lock_dir(dir: &Path) -> Result<Lock, String> {
    // SAFETY: geteuid has no preconditions and cannot fail.
    let uid = unsafe { libc::geteuid() };
    if let Some(line) = prepare(dir, crate::seat::is_default_dir(dir), uid)? {
        crate::label::emit(dir, &line);
    }
    let path = dir.join("lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let started = Instant::now();
    // SAFETY: flock on a descriptor this function owns, then the Lock does.
    while unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::EWOULDBLOCK) {
            return Err(format!("{}: {err}", path.display()));
        }
        if started.elapsed() >= WAIT {
            return Err(held(&path));
        }
        std::thread::sleep(RETRY);
    }
    // The pid only names the holder to a waiter: the lock is held whether or
    // not it was written.
    let pid = format!("{}\n", std::process::id());
    let _ = file
        .set_len(0)
        .and_then(|()| file.write_all_at(pid.as_bytes(), 0));
    Ok(Lock { file })
}

/// The failure for a lock still held when the wait runs out: the pid the
/// holder wrote, and what lets the lock go.
fn held(path: &Path) -> String {
    let pid = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok());
    match pid {
        Some(pid) => format!(
            "{}: held by pid {pid} past the {WAIT:?} wait; the lock goes when it finishes: \
             kill -CONT {pid} if it is stopped, kill {pid} to end it",
            path.display()
        ),
        None => format!(
            "{}: held past the {WAIT:?} wait by a process that wrote no pid; \
             the lock goes when that process exits",
            path.display()
        ),
    }
}

/// Make the graph directory. A default location (the state directory,
/// `/tmp/claimdag-UID` or the old `/tmp/claimdag`) is made with mode 0700,
/// so another user cannot read a graph this seat created under `/tmp`. A
/// default another user already owns is still used: the returned line is
/// `not-owner`, and the caller records it. A directory named by
/// `CLAIMDAG_DIR` or `--dir` is used as given, with no such line: a team
/// can share one on purpose.
///
/// # Errors
///
/// Fails when the directory cannot be made or its owner cannot be read.
pub fn prepare(dir: &Path, default: bool, uid: u32) -> Result<Option<String>, String> {
    if !default {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        return Ok(None);
    }
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    owned_by(dir, uid)
}

/// The warning when a default graph directory belongs to someone else.
///
/// A shared default made one user's claims land in another user's graph.
/// The directory is still the one to open: the line names the owner, and
/// the call proceeds. `Ok(None)` when `uid` owns it.
///
/// # Errors
///
/// Fails when the directory's owner cannot be read.
pub fn owned_by(dir: &Path, uid: u32) -> Result<Option<String>, String> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if meta.uid() != uid {
        return Ok(Some(crate::label::line(
            "not-owner",
            &format!("{} owner {} uid {uid}", dir.display(), meta.uid()),
        )));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Another user's directory is labeled and still usable.
    #[test]
    fn a_directory_another_user_owns_is_labeled() {
        use std::os::unix::fs::MetadataExt;
        let dir = std::env::temp_dir().join(format!("claimdag-own-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mine = std::fs::metadata(&dir).unwrap().uid();
        assert_eq!(owned_by(&dir, mine).unwrap(), None);
        let line = owned_by(&dir, mine + 1).unwrap().unwrap();
        assert_eq!(
            line,
            format!(
                "allow\twarn\tnot-owner\t{} owner {mine} uid {}",
                dir.display(),
                mine + 1
            )
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A default location is made private. Another user's directory is
    /// labeled, and the directory is still there to open.
    #[test]
    fn a_default_directory_is_private_and_another_owner_is_labeled() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = std::env::temp_dir().join(format!("claimdag-mode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let graph = dir.join("graph");
        // SAFETY: geteuid has no preconditions and cannot fail.
        let mine = unsafe { libc::geteuid() };
        assert_eq!(prepare(&graph, true, mine).unwrap(), None);
        let meta = std::fs::metadata(&graph).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o700);
        let line = prepare(&graph, true, meta.uid() + 1).unwrap().unwrap();
        assert!(line.starts_with("allow\twarn\tnot-owner\t"), "{line}");
        assert!(graph.is_dir());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A directory named by `CLAIMDAG_DIR` or `--dir` is used as given,
    /// whoever owns it.
    #[test]
    fn a_named_directory_is_trusted() {
        use std::os::unix::fs::MetadataExt;
        let dir = std::env::temp_dir().join(format!("claimdag-named-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let other = std::fs::metadata(&dir).unwrap().uid() + 1;
        assert!(prepare(&dir, false, other).is_ok());
        assert!(prepare(&dir.join("new"), false, other).is_ok());
        assert!(dir.join("new").is_dir());
        drop(lock_dir(&dir).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A second lock waits for the first to drop.
    #[test]
    fn the_lock_is_exclusive_across_handles() {
        let dir = std::env::temp_dir().join(format!("claimdag-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = lock_dir(&dir).unwrap();
        let dir2 = dir.clone();
        let waited = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let _second = lock_dir(&dir2).unwrap();
            started.elapsed()
        });
        std::thread::sleep(std::time::Duration::from_millis(150));
        drop(first);
        let elapsed = waited.join().unwrap();
        assert!(
            elapsed >= std::time::Duration::from_millis(100),
            "{elapsed:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Set on this test binary run again as the holder: take the lock on
    /// the directory it names and sleep until stdin closes.
    const HOLD: &str = "CLAIMDAG_TEST_HOLD";
    /// Set on this test binary run again as a writer: take the lock on the
    /// directory it names once, and exit as a command-line write does.
    const TAKE: &str = "CLAIMDAG_TEST_TAKE";

    /// This test binary again, running only the named test.
    fn this_test(name: &str) -> std::process::Command {
        let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
        cmd.args(["--exact", name, "--nocapture"]);
        cmd
    }

    /// A holder that hangs costs the next writer the wait and no more: it
    /// fails with one line naming the holder's pid, and goes through once
    /// the holder is gone.
    #[test]
    fn a_hung_holder_is_named_when_the_wait_runs_out() {
        use std::io::{BufRead, Read};
        use std::process::Stdio;

        const NAME: &str = "lock::tests::a_hung_holder_is_named_when_the_wait_runs_out";
        if let Some(dir) = std::env::var_os(HOLD) {
            let _lock = lock_dir(Path::new(&dir)).unwrap();
            println!("held");
            let _ = std::io::stdin().read_to_end(&mut Vec::new());
            return;
        }
        if let Some(dir) = std::env::var_os(TAKE) {
            if let Err(e) = lock_dir(Path::new(&dir)) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            std::process::exit(0);
        }

        let dir = std::env::temp_dir().join(format!("claimdag-hung-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut holder = this_test(NAME)
            .env(HOLD, &dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut said = std::io::BufReader::new(holder.stdout.take().unwrap()).lines();
        assert!(
            said.any(|line| line.is_ok_and(|l| l == "held")),
            "the holder never took the lock"
        );

        // Starting a process takes milliseconds, so a second bound's worth of
        // room past the wait runs out only on a writer that did not stop at it.
        let started = Instant::now();
        let mut waiter = this_test(NAME)
            .env(TAKE, &dir)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        while waiter.try_wait().unwrap().is_none() && started.elapsed() < 2 * WAIT {
            std::thread::sleep(RETRY);
        }
        let elapsed = started.elapsed();
        let _ = waiter.kill();
        let waited = waiter.wait_with_output().unwrap();
        assert!(elapsed >= WAIT && elapsed < 2 * WAIT, "{elapsed:?}");
        let stderr = String::from_utf8_lossy(&waited.stderr);
        let pid = holder.id();
        assert!(!waited.status.success(), "{stderr}");
        assert_eq!(stderr.lines().count(), 1, "{stderr}");
        assert!(stderr.contains(&format!("held by pid {pid}")), "{stderr}");
        assert!(stderr.contains(&format!("kill {pid}")), "{stderr}");

        // Ended the way the line says.
        // SAFETY: kill has no memory preconditions; the pid is our child's.
        assert_eq!(
            unsafe { libc::kill(pid.try_into().unwrap(), libc::SIGTERM) },
            0
        );
        holder.wait().unwrap();
        let again = this_test(NAME)
            .env(TAKE, &dir)
            .stdout(Stdio::null())
            .output()
            .unwrap();
        assert!(
            again.status.success(),
            "{}",
            String::from_utf8_lossy(&again.stderr)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The section the wait bound rests on, as one command-line write runs
    /// it: lock, load, one upsert, save, unlock, on a graph at the node cap.
    /// Prints the distribution that `measurements/lock-wait.md` records.
    #[test]
    #[ignore = "a measurement; cargo test --release -p claimdag --lib \
                section_at_the_cap -- --ignored --nocapture"]
    fn section_at_the_cap() {
        use crate::graph::{WorkFields, WorkGraph, WorkKind, WorkRole, WorkStatus};
        use crate::id::WorkId;

        // A thousand runs put ten samples above the 99th percentile.
        const RUNS: usize = 1000;

        fn todo(summary: &str) -> WorkFields<'_> {
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Todo,
                role: WorkRole::Unset,
                parent: WorkId::ZERO,
                actor: WorkId::ZERO,
                summary,
            }
        }

        let dir = std::env::temp_dir().join(format!("claimdag-section-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let seat = WorkId::from_hex("0000000000000000000000000000000f").unwrap();

        // Finished work until prune stops the count growing: that count is
        // the cap, and a seat's graph sits there once it has run long enough.
        let mut graph = WorkGraph::default();
        let mut made = 0usize;
        loop {
            let before = graph.list().len();
            let id = graph
                .upsert(WorkId::ZERO, todo(&format!("work item {made}")))
                .unwrap();
            made += 1;
            graph.claim(id, seat, None).unwrap();
            graph
                .complete(id, WorkStatus::Done, "", seat, None)
                .unwrap();
            if graph.list().len() == before {
                break;
            }
        }
        let nodes = graph.list().len();
        graph.save_dir(&dir).unwrap();
        let bytes = std::fs::metadata(dir.join(crate::snap::SNAP_BIN))
            .unwrap()
            .len();

        let mut times: Vec<f64> = (0..RUNS)
            .map(|n| {
                let start = Instant::now();
                let lock = lock_dir(&dir).unwrap();
                let mut graph = WorkGraph::load_for_write(&dir).unwrap();
                graph
                    .upsert(WorkId::ZERO, todo(&format!("work item {}", made + n)))
                    .unwrap();
                graph.save_dir(&dir).unwrap();
                drop(lock);
                start.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        times.sort_by(f64::total_cmp);
        let at = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
        println!(
            "nodes {nodes}  work.bin {bytes} B  runs {RUNS}  \
             median {:.2} ms  p99 {:.2} ms  max {:.2} ms",
            at(0.5),
            at(0.99),
            at(1.0)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
