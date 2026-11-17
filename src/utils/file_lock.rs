use anyhow::{anyhow, Context, Result};
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::Error;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::utils::private_file::{PRIVATE_DIR_MODE, PRIVATE_FILE_MODE};

pub struct FileLock {
    _file: File,
}

pub fn lock_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| String::from("file"));

    path.with_file_name(format!(".{}.awsctl.lock", file_name))
}

pub fn lock_exclusive(path: &Path) -> Result<FileLock> {
    let lock_path = lock_path(path);
    let parent = lock_path
        .parent()
        .ok_or_else(|| anyhow!("The path {} has no parent directory.", path.display()))?;
    DirBuilder::new()
        .recursive(true)
        .mode(PRIVATE_DIR_MODE)
        .create(parent)
        .with_context(|| format!("Unable to create the directory {}.", parent.display()))?;

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(PRIVATE_FILE_MODE)
        .open(&lock_path)
        .with_context(|| format!("Unable to open the lock file {}.", lock_path.display()))?;

    loop {
        // SAFETY: the descriptor belongs to `file`, which outlives this call.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return Ok(FileLock { _file: file });
        }
        let error = Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINTR) {
            return Err(error).with_context(|| format!("Unable to lock {}.", lock_path.display()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{lock_exclusive, lock_path};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    fn temp_dir(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("awsctl-file-lock-{}-{}", label, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn locks_beside_the_file_without_touching_kubectls_own_lock_name() {
        assert_eq!(
            lock_path(Path::new("/home/me/.kube/config")),
            Path::new("/home/me/.kube/.config.awsctl.lock")
        );
    }

    #[test]
    fn a_second_holder_waits_until_the_first_releases() {
        let dir = temp_dir("exclusive");
        let path = dir.join("config");
        let first = lock_exclusive(&path).expect("locks");
        let acquired = Arc::new(AtomicBool::new(false));

        let waiter = {
            let path = path.clone();
            let acquired = Arc::clone(&acquired);
            std::thread::spawn(move || {
                let _second = lock_exclusive(&path).expect("locks");
                acquired.store(true, Ordering::SeqCst);
            })
        };
        std::thread::sleep(Duration::from_millis(100));
        assert!(!acquired.load(Ordering::SeqCst));

        drop(first);
        waiter.join().expect("joins");
        assert!(acquired.load(Ordering::SeqCst));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
