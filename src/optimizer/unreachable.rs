use std::collections::HashSet;
use std::hash::BuildHasher;
use std::path::Path;

use super::OptimizeError;
use super::rewrite::find_main_container;
use super::util::identify_factory_service;

/// Extracts method names from `$this->fileMap` in the main container.
///
/// Services in `fileMap` are public non-hot-path services resolved by
/// `Container::make()` via `$this->load($this->fileMap[$id])`. This is a
/// dynamic dispatch — the method name never appears as a literal
/// `load('methodName')` call, so the string search alone misses them.
fn extract_filemap_methods(container_dir: &Path) -> HashSet<String> {
    let mut methods = HashSet::new();

    let Ok(main_file) = find_main_container(container_dir) else {
        return methods;
    };
    let Ok(content) = std::fs::read_to_string(&main_file) else {
        return methods;
    };

    let mut in_filemap = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.contains("$this->fileMap") {
            in_filemap = true;
            continue;
        }
        if in_filemap {
            if trimmed == "];" {
                break;
            }
            // Lines look like: 'service.id' => 'getServiceIdService',
            if let Some(arrow) = trimmed.find("=> '") {
                let after_arrow = &trimmed[arrow + 4..];
                if let Some(end) = after_arrow.find('\'') {
                    methods.insert(after_arrow[..end].to_owned());
                }
            }
        }
    }

    methods
}

/// # Errors
/// Returns `OptimizeError` if the container directory cannot be read.
pub fn find_unreachable_factories<S: BuildHasher>(
    container_dir: &Path,
    already_removed: &HashSet<String, S>,
) -> Result<HashSet<String>, OptimizeError> {
    let mut factories: Vec<(String, String)> = Vec::new();
    let mut all_content = String::new();

    // Single pass: collect factory metadata and concatenate PHP content
    let entries = std::fs::read_dir(container_dir)?;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("php") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };

        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with("get") && name_str.ends_with("Service.php") {
            let method_name = name_str.trim_end_matches(".php").to_owned();
            if let Some(id) = identify_factory_service(&path)
                && !already_removed.contains(&id)
            {
                factories.push((method_name, id));
            }
        }

        all_content.push_str(&content);
    }

    // fileMap entries are reachable via Container::make() dynamic dispatch
    let filemap_methods = extract_filemap_methods(container_dir);

    // A factory method name that appears as a quoted string anywhere in the
    // container code is reachable. Known call sites:
    //   - load('getXService')            — lazy inline pattern
    //   - fileMap entries                 — Container::make() dynamic dispatch
    //   - ServiceLocator constructor     — ['privates', 'id', 'getXService', true]
    //   - getService() third argument    — Container::getService(..., 'getXService', ...)
    let mut unreachable = HashSet::new();
    for (method_name, service_id) in &factories {
        if filemap_methods.contains(method_name) {
            continue;
        }
        let quoted_ref = format!("'{method_name}'");
        if !all_content.contains(&quoted_ref) {
            unreachable.insert(service_id.clone());
        }
    }

    Ok(unreachable)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;

    use super::*;

    #[test]
    fn detects_unreachable_factory() {
        let dir = tempfile::tempdir().unwrap();

        fs::write(
            dir.path().join("getUnreachableService.php"),
            "<?php\n$container->privates['unreachable'] = new \\Foo();",
        )
        .unwrap();

        fs::write(
            dir.path().join("getReachableService.php"),
            "<?php\n$container->privates['reachable'] = new \\Bar();",
        )
        .unwrap();

        fs::write(
            dir.path().join("getConsumerService.php"),
            "<?php\n$container->privates['consumer'] = new \\Baz($container->privates['reachable'] ?? $container->load('getReachableService'));",
        )
        .unwrap();

        let unreachable = find_unreachable_factories(dir.path(), &HashSet::new()).unwrap();
        assert!(unreachable.contains("unreachable"));
        assert!(!unreachable.contains("reachable"));
        assert!(unreachable.contains("consumer"));
    }

    #[test]
    fn filemap_entries_are_reachable() {
        let dir = tempfile::tempdir().unwrap();

        // Factory for a public service listed in fileMap
        fs::write(
            dir.path().join("getPublicCacheService.php"),
            "<?php\n$container->services['cache.app'] = new \\CachePool();",
        )
        .unwrap();

        // Factory for a private service NOT in fileMap and NOT load()'d
        fs::write(
            dir.path().join("getInternalHelperService.php"),
            "<?php\n$container->privates['internal.helper'] = new \\Helper();",
        )
        .unwrap();

        // Main container with fileMap referencing the public service
        let main = format!(
            "{}{}",
            r#"<?php
class App_AppKernelProdContainer extends Container
{
    public function __construct()
    {
        $this->fileMap = [
            'cache.app' => 'getPublicCacheService',
        ];
    }
}
"#,
            "x".repeat(5000)
        );
        fs::write(dir.path().join("App_AppKernelProdContainer.php"), &main).unwrap();

        let unreachable = find_unreachable_factories(dir.path(), &HashSet::new()).unwrap();
        assert!(
            !unreachable.contains("cache.app"),
            "fileMap service must be treated as reachable"
        );
        assert!(
            unreachable.contains("internal.helper"),
            "private service without load() reference is unreachable"
        );
    }
}
