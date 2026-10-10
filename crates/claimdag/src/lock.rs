//! One mutator at a time across processes: an advisory lock on the graph
//! directory, held from load to save.

use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::path::Path;

/// The lock, released on drop.
pub struct Lock {
    _file: File,
}

/// Take the exclusive lock on `dir/lock`, creating the directory (mode
/// 0700, so another user cannot read the graph out of `/tmp`). Blocks while
/// another process holds it.
///
/// # Errors
///
/// Fails when the directory or the lock file cannot be created, or the lock
/// cannot be taken.
pub fn lock_dir(dir: &Path) -> Result<Lock, String> {
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    // SAFETY: geteuid has no preconditions and cannot fail.
    owned_by(dir, unsafe { libc::geteuid() })?;
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

    /// A graph directory the lock makes is this user's alone.
    #[test]
    fn a_new_graph_directory_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("claimdag-mode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        drop(lock_dir(&dir.join("graph")).unwrap());
        let mode = std::fs::metadata(dir.join("graph"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
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
