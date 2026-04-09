pub mod collector;
pub mod generator;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PreloadError {
    #[error("failed to read directory {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to write preload file: {0}")]
    Write(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct PhpClass {
    pub fqcn: String,
    pub file_path: std::path::PathBuf,
    pub parent: Option<String>,
    pub interfaces: Vec<String>,
}

#[derive(Debug)]
pub struct ExistingPreload {
    pub require_lines: Vec<String>,
    pub required_paths: HashSet<String>,
}

impl ExistingPreload {
    /// Parse a Symfony-generated preload file, extracting all `require` lines
    /// and the file paths they reference.
    ///
    /// # Errors
    /// Returns `PreloadError::Io` if the file cannot be read.
    pub fn parse(path: &Path) -> Result<Self, PreloadError> {
        let content = std::fs::read_to_string(path).map_err(|e| PreloadError::Io {
            path: path.display().to_string(),
            source: e,
        })?;

        let mut require_lines = Vec::new();
        let mut required_paths = HashSet::new();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("require ")
                || trimmed.starts_with("require_once ")
                || trimmed.starts_with("(require ")
            {
                require_lines.push(line.to_owned());
                if let Some(file_path) = extract_require_path(trimmed) {
                    required_paths.insert(file_path);
                }
            }
        }

        Ok(Self {
            require_lines,
            required_paths,
        })
    }
}

fn extract_require_path(line: &str) -> Option<String> {
    let after_quote = line.split('\'').nth(1)?;
    Some(after_quote.to_owned())
}

/// Find the Symfony preload file (`*Container.preload.php`) in a cache directory.
#[must_use]
pub fn find_symfony_preload(cache_dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(cache_dir).ok()?;
    entries.filter_map(Result::ok).find_map(|entry| {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.ends_with(".preload.php") {
            Some(entry.path())
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parse_existing_preload() {
        let tmp = tempfile::tempdir().unwrap();
        let preload = tmp.path().join("App_Container.preload.php");
        fs::write(
            &preload,
            r"<?php
require dirname(__DIR__, 3).'/vendor/autoload.php';
(require __DIR__.'/ContainerAbc/App_Container.php')->set('foo', null);
require __DIR__.'/ContainerAbc/getMailerService.php';
require __DIR__.'/ContainerAbc/getCacheService.php';

$classes = [];
Preloader::preload($classes);
",
        )
        .unwrap();

        let existing = ExistingPreload::parse(&preload).unwrap();
        assert_eq!(existing.require_lines.len(), 4);
        assert!(
            existing
                .required_paths
                .contains("/ContainerAbc/getMailerService.php")
        );
        assert!(
            existing
                .required_paths
                .contains("/ContainerAbc/getCacheService.php")
        );
    }

    #[test]
    fn find_symfony_preload_locates_file() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("App_KernelProdContainer.preload.php"),
            "<?php\n",
        )
        .unwrap();
        let found = find_symfony_preload(tmp.path());
        assert!(found.is_some());
        assert!(found.unwrap().to_string_lossy().contains(".preload.php"));
    }

    #[test]
    fn find_symfony_preload_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(find_symfony_preload(tmp.path()).is_none());
    }

    #[test]
    fn extract_require_path_from_line() {
        let line = "require __DIR__.'/ContainerAbc/getFooService.php';";
        assert_eq!(
            extract_require_path(line),
            Some("/ContainerAbc/getFooService.php".to_owned())
        );
    }

    #[test]
    fn extract_require_path_returns_none_for_no_quotes() {
        let line = "require $something;";
        assert_eq!(extract_require_path(line), None);
    }
}
