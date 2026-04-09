use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::config::Config;

#[derive(Debug, Error)]
pub enum DetectError {
    #[error("not a Symfony project: no composer.json with symfony/framework-bundle found")]
    NotSymfonyProject,
    #[error("cache not warmed: run `bin/console cache:warmup --env=prod` first")]
    CacheNotWarmed,
    #[error("failed to read composer.json: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug)]
pub struct SymfonyProject {
    pub root: PathBuf,
    pub cache_dir: PathBuf,
    #[allow(dead_code)]
    pub src_dir: PathBuf,
}

/// # Errors
/// Returns `DetectError` if the project is not a Symfony project or the cache is not warmed.
pub fn detect(project_path: &Path, config: &Config) -> Result<SymfonyProject, DetectError> {
    let root = config
        .project
        .root
        .clone()
        .or_else(|| find_symfony_root(project_path))
        .ok_or(DetectError::NotSymfonyProject)?;

    let cache_dir = config
        .project
        .cache_dir
        .clone()
        .unwrap_or_else(|| root.join("var/cache/prod"));

    if !has_compiled_container(&cache_dir) {
        return Err(DetectError::CacheNotWarmed);
    }

    let src_dir = config
        .project
        .src_dir
        .clone()
        .unwrap_or_else(|| root.join("src"));

    Ok(SymfonyProject {
        root,
        cache_dir,
        src_dir,
    })
}

fn find_symfony_root(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        let composer = current.join("composer.json");
        if composer.exists() && is_symfony_project(&composer) {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn is_symfony_project(composer_path: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(composer_path) else {
        return false;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    let has_in_require = json
        .get("require")
        .and_then(|r| r.get("symfony/framework-bundle"))
        .is_some();
    let has_in_require_dev = json
        .get("require-dev")
        .and_then(|r| r.get("symfony/framework-bundle"))
        .is_some();
    has_in_require || has_in_require_dev
}

fn has_compiled_container(cache_dir: &Path) -> bool {
    if !cache_dir.is_dir() {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(cache_dir) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|e| {
        let name = e.file_name();
        let name = name.to_string_lossy();
        name.starts_with("Container") && e.file_type().is_ok_and(|t| t.is_dir())
    })
}

#[must_use]
pub fn find_container_dir(cache_dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(cache_dir).ok()?;
    entries
        .filter_map(Result::ok)
        .find(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with("Container") && e.file_type().is_ok_and(|t| t.is_dir())
        })
        .map(|e| e.path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_symfony_project(dir: &Path) {
        fs::write(
            dir.join("composer.json"),
            r#"{"require":{"symfony/framework-bundle":"^7.0"}}"#,
        )
        .unwrap();
        let container_dir = dir.join("var/cache/prod/ContainerAbcDef");
        fs::create_dir_all(&container_dir).unwrap();
        fs::write(
            container_dir.join("App_KernelProdContainer.php"),
            "<?php // compiled container",
        )
        .unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
    }

    #[test]
    fn detect_valid_symfony_project() {
        let dir = tempfile::tempdir().unwrap();
        setup_symfony_project(dir.path());
        let config = Config::default();
        let project = detect(dir.path(), &config).unwrap();
        assert_eq!(project.root, dir.path());
        assert!(project.cache_dir.ends_with("var/cache/prod"));
    }

    #[test]
    fn detect_fails_without_composer() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::default();
        let err = detect(dir.path(), &config).unwrap_err();
        assert!(matches!(err, DetectError::NotSymfonyProject));
    }

    #[test]
    fn detect_fails_without_warmed_cache() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("composer.json"),
            r#"{"require":{"symfony/framework-bundle":"^7.0"}}"#,
        )
        .unwrap();
        let config = Config::default();
        let err = detect(dir.path(), &config).unwrap_err();
        assert!(matches!(err, DetectError::CacheNotWarmed));
    }

    #[test]
    fn detect_with_config_override() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("custom-cache");
        let container_dir = cache_dir.join("ContainerXyz");
        fs::create_dir_all(&container_dir).unwrap();
        fs::write(container_dir.join("dummy.php"), "<?php").unwrap();

        let config = Config {
            project: crate::config::ProjectConfig {
                root: Some(dir.path().to_path_buf()),
                cache_dir: Some(cache_dir),
                src_dir: None,
            },
            analyze: Default::default(),
            preload: Default::default(),
        };
        let project = detect(dir.path(), &config).unwrap();
        assert!(project.cache_dir.ends_with("custom-cache"));
    }

    #[test]
    fn find_container_dir_returns_path() {
        let dir = tempfile::tempdir().unwrap();
        let container = dir.path().join("ContainerABC");
        fs::create_dir_all(&container).unwrap();
        assert_eq!(find_container_dir(dir.path()), Some(container));
    }

    #[test]
    fn non_symfony_composer_rejected() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("composer.json"),
            r#"{"require":{"laravel/framework":"^10"}}"#,
        )
        .unwrap();
        let config = Config::default();
        let err = detect(dir.path(), &config).unwrap_err();
        assert!(matches!(err, DetectError::NotSymfonyProject));
    }
}
