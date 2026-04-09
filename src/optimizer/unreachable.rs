use std::collections::HashSet;
use std::hash::BuildHasher;
use std::path::Path;

use super::OptimizeError;
use super::util::identify_factory_service;

/// # Errors
/// Returns `OptimizeError` if the container directory cannot be read.
pub fn find_unreachable_factories<S: BuildHasher>(
    container_dir: &Path,
    already_removed: &HashSet<String, S>,
) -> Result<HashSet<String>, OptimizeError> {
    let mut factories: Vec<(String, String)> = Vec::new();

    let entries = std::fs::read_dir(container_dir)?;
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with("get") && name_str.ends_with("Service.php") {
            let method_name = name_str.trim_end_matches(".php").to_owned();
            if let Some(id) = identify_factory_service(&entry.path())
                && !already_removed.contains(&id)
            {
                factories.push((method_name, id));
            }
        }
    }

    let mut all_content = String::new();
    let entries = std::fs::read_dir(container_dir)?;
    for entry in entries.filter_map(Result::ok) {
        if entry.path().extension().and_then(|e| e.to_str()) == Some("php")
            && let Ok(content) = std::fs::read_to_string(entry.path())
        {
            all_content.push_str(&content);
        }
    }

    let mut unreachable = HashSet::new();
    for (method_name, service_id) in &factories {
        let load_pattern = format!("load('{method_name}')");
        if !all_content.contains(&load_pattern) {
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
}
