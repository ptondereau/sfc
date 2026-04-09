use std::collections::HashSet;
use std::hash::BuildHasher;
use std::path::Path;

use super::OptimizeError;
use super::util::identify_factory_service;

pub struct DeadServiceResult {
    pub files_removed: usize,
    pub bytes_freed: u64,
    pub removed_ids: Vec<String>,
}

/// # Errors
/// Returns `OptimizeError` if directory cannot be read or files cannot be deleted.
pub fn remove_dead_services<S: BuildHasher>(
    container_dir: &Path,
    dead_ids: &HashSet<String, S>,
    dry_run: bool,
) -> Result<DeadServiceResult, OptimizeError> {
    let mut result = DeadServiceResult {
        files_removed: 0,
        bytes_freed: 0,
        removed_ids: Vec::new(),
    };

    let entries = std::fs::read_dir(container_dir)?;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if !name_str.starts_with("get") || !name_str.ends_with("Service.php") {
            continue;
        }

        if let Some(service_id) = identify_factory_service(&path)
            && dead_ids.contains(&service_id)
        {
            let meta = std::fs::metadata(&path)?;
            result.bytes_freed += meta.len();
            result.files_removed += 1;
            result.removed_ids.push(service_id);

            if !dry_run {
                std::fs::remove_file(&path)?;
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_container_dir() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        std::fs::write(
            dir.join("getMailerService.php"),
            "<?php $container->privates['app.mailer'] = function () {};",
        )
        .unwrap();

        std::fs::write(
            dir.join("getLoggerService.php"),
            "<?php $container->services['app.logger'] = function () {};",
        )
        .unwrap();

        std::fs::write(
            dir.join("getCacheService.php"),
            "<?php $container->privates['app.cache'] = function () {};",
        )
        .unwrap();

        std::fs::write(dir.join("unrelated.php"), "<?php // not a factory").unwrap();

        tmp
    }

    #[test]
    fn removes_dead_factory_files() {
        let tmp = setup_container_dir();
        let dead: HashSet<String> = ["app.mailer".to_owned(), "app.logger".to_owned()].into();

        let result = remove_dead_services(tmp.path(), &dead, false).unwrap();

        assert_eq!(result.files_removed, 2);
        assert!(result.bytes_freed > 0);
        assert!(!tmp.path().join("getMailerService.php").exists());
        assert!(!tmp.path().join("getLoggerService.php").exists());
        assert!(tmp.path().join("getCacheService.php").exists());
        assert!(tmp.path().join("unrelated.php").exists());
    }

    #[test]
    fn dry_run_does_not_delete() {
        let tmp = setup_container_dir();
        let dead: HashSet<String> = ["app.mailer".to_owned()].into();

        let result = remove_dead_services(tmp.path(), &dead, true).unwrap();

        assert_eq!(result.files_removed, 1);
        assert!(result.bytes_freed > 0);
        assert!(tmp.path().join("getMailerService.php").exists());
    }
}
