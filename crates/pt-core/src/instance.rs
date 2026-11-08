//! Only one copy of each program per user (background app, profile manager).
//! Uses an OS file lock (std's `File::try_lock`), which works the same on
//! Windows and Linux and is released automatically if the process exits or
//! crashes. No OS-specific code needed.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

/// Holds the lock for as long as it is alive.
#[derive(Debug)]
pub struct InstanceGuard {
    _file: File,
}

/// Lock `<root>/<name>.lock`. `Ok(None)` means another copy holds it.
pub fn acquire(root: &Path, name: &str) -> std::io::Result<Option<InstanceGuard>> {
    std::fs::create_dir_all(root)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join(format!("{name}.lock")))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(InstanceGuard { _file: file })),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_instance_is_refused_until_first_exits() {
        let dir = tempfile::tempdir().unwrap();
        let first = acquire(dir.path(), "app").unwrap();
        assert!(first.is_some());
        assert!(acquire(dir.path(), "app").unwrap().is_none());
        assert!(
            acquire(dir.path(), "manager").unwrap().is_some(),
            "different name, different lock"
        );
        drop(first);
        assert!(acquire(dir.path(), "app").unwrap().is_some());
    }
}
