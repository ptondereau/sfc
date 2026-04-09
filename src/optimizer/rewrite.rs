use std::collections::HashSet;
use std::hash::BuildHasher;
use std::path::Path;

use super::OptimizeError;

/// # Errors
/// Returns `OptimizeError` if the main container file cannot be found or written.
pub fn rewrite_maps<S: BuildHasher>(
    container_dir: &Path,
    removed_ids: &HashSet<String, S>,
    dry_run: bool,
) -> Result<usize, OptimizeError> {
    let main_file = find_main_container(container_dir)?;
    let content = std::fs::read_to_string(&main_file)?;

    let prefixes: Vec<(String, String)> = removed_ids
        .iter()
        .map(|id| (format!("'{id}'"), format!("\"{id}\"")))
        .collect();

    let mut removed_count = 0;
    let mut new_lines = Vec::new();
    let mut in_map = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.contains("$this->methodMap") || trimmed.contains("$this->fileMap") {
            in_map = true;
        }

        if in_map {
            let should_remove = prefixes.iter().any(|(single, double)| {
                trimmed.starts_with(single.as_str()) || trimmed.starts_with(double.as_str())
            });

            if should_remove {
                removed_count += 1;
                continue;
            }

            if trimmed == "];" {
                in_map = false;
            }
        }

        new_lines.push(line);
    }

    if removed_count > 0 && !dry_run {
        let mut output = new_lines.join("\n");
        if content.ends_with('\n') {
            output.push('\n');
        }
        std::fs::write(&main_file, output)?;
    }

    Ok(removed_count)
}

fn find_main_container(container_dir: &Path) -> Result<std::path::PathBuf, OptimizeError> {
    let entries = std::fs::read_dir(container_dir)?;
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if (name_str.contains("KernelProdContainer")
            || name_str.contains("KernelDevDebugContainer"))
            && std::fs::metadata(entry.path())
                .map(|m| m.len() > 5000)
                .unwrap_or(false)
        {
            return Ok(entry.path());
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "main container file not found",
    )
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_removes_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        let content = r#"<?php
class AppKernelProdContainer extends Container
{
    public function __construct()
    {
        $this->methodMap = [
            'app.mailer' => 'getMailerService',
            'app.logger' => 'getLoggerService',
            'app.cache' => 'getCacheService',
        ];
        $this->fileMap = [
            'app.mailer' => 'getMailerService.php',
            'app.logger' => 'getLoggerService.php',
            'app.cache' => 'getCacheService.php',
        ];
    }
}
"#;
        let padding = "x".repeat(5001 - content.len().min(5001));
        let full_content = format!("{content}{padding}");

        std::fs::write(dir.join("AppKernelProdContainer.php"), &full_content).unwrap();

        let removed: HashSet<String> = ["app.mailer".to_owned(), "app.logger".to_owned()].into();

        let count = rewrite_maps(dir, &removed, false).unwrap();
        assert_eq!(count, 4);

        let result = std::fs::read_to_string(dir.join("AppKernelProdContainer.php")).unwrap();
        assert!(!result.contains("app.mailer"));
        assert!(!result.contains("app.logger"));
        assert!(result.contains("app.cache"));
    }
}
