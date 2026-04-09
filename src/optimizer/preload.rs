use std::collections::HashSet;
use std::hash::BuildHasher;
use std::path::Path;

use super::OptimizeError;

/// Strips `require` lines for removed factory files from the Symfony preload.
///
/// The preload file (`App_*Container.preload.php`) hardcodes `require` lines
/// for factory files. If optimize removes a factory file, the preload must be
/// patched or PHP-FPM will crash at startup with a "Failed opening required"
/// fatal error.
///
/// # Errors
/// Returns `OptimizeError` if the preload cannot be read or written.
pub fn rewrite_preload<S: BuildHasher>(
    cache_dir: &Path,
    removed_method_names: &HashSet<String, S>,
    dry_run: bool,
) -> Result<usize, OptimizeError> {
    let Some(preload_path) = find_preload(cache_dir) else {
        return Ok(0);
    };

    let content = std::fs::read_to_string(&preload_path)?;
    let line_count = content.lines().count();
    let mut stripped = 0;
    let mut new_lines = Vec::with_capacity(line_count);

    // Pre-build filename suffixes to avoid format!() per line
    let suffixes: Vec<String> = removed_method_names
        .iter()
        .map(|m| format!("{m}.php"))
        .collect();

    for line in content.lines() {
        if is_removed_require(line, &suffixes) {
            stripped += 1;
            continue;
        }
        new_lines.push(line);
    }

    if stripped > 0 && !dry_run {
        let mut output = new_lines.join("\n");
        if content.ends_with('\n') {
            output.push('\n');
        }
        std::fs::write(&preload_path, output)?;
    }

    Ok(stripped)
}

fn find_preload(cache_dir: &Path) -> Option<std::path::PathBuf> {
    let entries = std::fs::read_dir(cache_dir).ok()?;
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.ends_with(".preload.php") {
            return Some(entry.path());
        }
    }
    None
}

fn is_removed_require(line: &str, suffixes: &[String]) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with("require") {
        return false;
    }
    // Match patterns like:
    //   require __DIR__.'/Container.../getXService.php';
    //   require_once './var/cache/prod/Container.../getXService.php';
    suffixes
        .iter()
        .any(|suffix| trimmed.contains(suffix.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_removed_factory_requires() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        let preload = r#"<?php
use Symfony\Component\DependencyInjection\Dumper\Preloader;

require dirname(__DIR__, 3).'/vendor/autoload.php';
(require __DIR__.'/ContainerAbc123/App_AppKernelProdContainer.php')->set(\ContainerAbc123\App_AppKernelProdContainer::class, null);
require __DIR__.'/ContainerAbc123/getMailerService.php';
require __DIR__.'/ContainerAbc123/getLoggerService.php';
require __DIR__.'/ContainerAbc123/getCacheService.php';

$classes = [];
$classes[] = 'App\Kernel';

Preloader::preload($classes);
"#;

        std::fs::write(dir.join("App_AppKernelProdContainer.preload.php"), preload).unwrap();

        let removed: HashSet<String> =
            ["getMailerService".to_owned(), "getLoggerService".to_owned()].into();

        let count = rewrite_preload(dir, &removed, false).unwrap();
        assert_eq!(count, 2);

        let result =
            std::fs::read_to_string(dir.join("App_AppKernelProdContainer.preload.php")).unwrap();
        assert!(!result.contains("getMailerService"));
        assert!(!result.contains("getLoggerService"));
        assert!(result.contains("getCacheService"));
        assert!(result.contains("vendor/autoload.php"));
        assert!(result.contains("Preloader::preload"));
    }

    #[test]
    fn no_preload_file_returns_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let removed: HashSet<String> = ["getMailerService".to_owned()].into();
        let count = rewrite_preload(tmp.path(), &removed, false).unwrap();
        assert_eq!(count, 0);
    }
}
