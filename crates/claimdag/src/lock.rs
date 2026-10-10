//! One mutator at a time across processes: an advisory lock on the graph
//! directory, held from load to save.

use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::path::Path;

/// The lock, released on drop.
pub struct Lock {
    _file: File,
}

/// Take the exclusive lock on `dir/lock`, creating the directory. Blocks
/// while another process holds it. A default location is made private and
/// must be this user's; see [`prepare`].
///
/// # Errors
///
/// Fails when the directory or the lock file cannot be created, the lock
/// cannot be taken, or a default location belongs to another user.
pub fn lock_dir(dir: &Path) -> Result<Lock, String> {
    // SAFETY: geteuid has no preconditions and cannot fail.
    let uid = unsafe { libc::geteuid() };
    prepare(dir, crate::seat::is_default_dir(dir), uid)?;
    let path = dir.join("lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    // SAFETY: flock on a descriptor this struct owns for its whole life.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if rc != 0 {
        return Err(format!(
            "{}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(Lock { _file: file })
}

/// Make the graph directory. A default location (the state directory,
/// `/tmp/claimdag-UID` or the old `/tmp/claimdag`) is made with mode 0700,
/// so another user cannot read the graph out of `/tmp`, and is refused when
/// another user owns it. A directory named by `CLAIMDAG_DIR` or `--dir` is
/// used as given: a team can share one on purpose.
///
/// # Errors
///
/// Fails when the directory cannot be made, or a default location belongs
/// to a user other than `uid`.
pub fn prepare(dir: &Path, default: bool, uid: u32) -> Result<(), String> {
    if !default {
        return std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()));
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

/// Refuse a graph directory another user owns. A shared default made one
/// user's claims land in, or fail on, another user's graph.
///
/// # Errors
///
/// Names the owner and the variable that picks another directory.
pub fn owned_by(dir: &Path, uid: u32) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if meta.uid() != uid {
        return Err(format!(
            "{} belongs to uid {}, not this user (uid {uid}); its graph is not this seat's. \
             Set CLAIMDAG_DIR or pass --dir to use a directory of your own",
            dir.display(),
            meta.uid()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Another user's directory is refused by name.
    #[test]
    fn a_directory_another_user_owns_is_refused() {
        use std::os::unix::fs::MetadataExt;
        let dir = std::env::temp_dir().join(format!("claimdag-own-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mine = std::fs::metadata(&dir).unwrap().uid();
        assert!(owned_by(&dir, mine).is_ok());
        let err = owned_by(&dir, mine + 1).unwrap_err();
        assert!(err.contains("not this user"), "{err}");
        assert!(err.contains("CLAIMDAG_DIR"), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A default location is made private, and refused when another user
    /// owns it.
    #[test]
    fn a_default_directory_is_private_and_must_be_ours() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = std::env::temp_dir().join(format!("claimdag-mode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let graph = dir.join("graph");
        // SAFETY: geteuid has no preconditions and cannot fail.
        let mine = unsafe { libc::geteuid() };
        prepare(&graph, true, mine).unwrap();
        let meta = std::fs::metadata(&graph).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o700);
        let err = prepare(&graph, true, meta.uid() + 1).unwrap_err();
        assert!(err.contains("not this user"), "{err}");
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
}
