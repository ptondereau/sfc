use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::OptimizeError;
use crate::project::find_container_dir;

/// # Errors
/// Returns `OptimizeError` if the parent directory is missing or the copy fails.
pub fn create_backup(container_dir: &Path) -> Result<PathBuf, OptimizeError> {
    let parent = container_dir
        .parent()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no parent directory"))?;

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let backup_dir = parent.join(format!(".sfc-backup-{ts}"));
    copy_dir_recursive(container_dir, &backup_dir)?;

    Ok(backup_dir)
}

/// # Errors
/// Returns `OptimizeError` if no backup exists or the restore fails.
pub fn restore_latest(cache_dir: &Path) -> Result<PathBuf, OptimizeError> {
    let backup_dir = find_latest_backup(cache_dir).ok_or(OptimizeError::NoBackup)?;
    let container_dir = find_container_dir(cache_dir)
        .ok_or_else(|| OptimizeError::Analysis("no Container directory found".into()))?;

    std::fs::remove_dir_all(&container_dir)?;
    copy_dir_recursive(&backup_dir, &container_dir)?;
    std::fs::remove_dir_all(&backup_dir)?;

    Ok(container_dir)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(dst)?;

    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());

        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }

    Ok(())
}

fn find_latest_backup(cache_dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(cache_dir).ok()?;
    entries
        .filter_map(Result::ok)
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with(".sfc-backup-") && e.file_type().is_ok_and(|t| t.is_dir())
        })
        .max_by_key(std::fs::DirEntry::file_name)
        .map(|e| e.path())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_and_restore() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path();
        let container_dir = cache_dir.join("ContainerABC");
        std::fs::create_dir_all(&container_dir).unwrap();
        std::fs::write(container_dir.join("service.php"), b"original").unwrap();

        let backup_path = create_backup(&container_dir).unwrap();
        assert!(backup_path.exists());

        std::fs::write(container_dir.join("service.php"), b"modified").unwrap();

        let restored = restore_latest(cache_dir).unwrap();
        assert_eq!(restored, container_dir);

        let content = std::fs::read_to_string(container_dir.join("service.php")).unwrap();
        assert_eq!(content, "original");

        assert!(!backup_path.exists());
    }

    #[test]
    fn no_backup_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path();
        std::fs::create_dir_all(cache_dir.join("ContainerXYZ")).unwrap();

        let result = restore_latest(cache_dir);
        assert!(result.is_err());
        assert!(
            matches!(result, Err(OptimizeError::NoBackup)),
            "expected NoBackup error"
        );
    }
}
