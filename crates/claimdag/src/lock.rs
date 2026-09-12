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
/// while another process holds it.
///
/// # Errors
///
/// Fails when the directory or the lock file cannot be created, or the lock
/// cannot be taken.
pub fn lock_dir(dir: &Path) -> Result<Lock, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
