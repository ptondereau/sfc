use std::fmt::Write;
use std::path::Path;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum InitError {
    #[error("sfc.toml already exists at {0}")]
    AlreadyExists(String),
    #[error("not a Symfony project: {0}")]
    NotSymfony(String),
    #[error("failed to write sfc.toml: {0}")]
    Io(#[from] std::io::Error),
}

/// # Errors
/// Returns `InitError` if sfc.toml exists, project is not Symfony, or writing fails.
pub fn run(project_path: &Path) -> Result<(), InitError> {
    let sfc_toml = project_path.join("sfc.toml");
    if sfc_toml.exists() {
        return Err(InitError::AlreadyExists(sfc_toml.display().to_string()));
    }

    let composer = project_path.join("composer.json");
    if !composer.exists() {
        return Err(InitError::NotSymfony("no composer.json found".to_owned()));
    }

    let composer_content = std::fs::read_to_string(&composer)?;
    let has_bundle = serde_json::from_str::<serde_json::Value>(&composer_content)
        .ok()
        .is_some_and(|json| {
            json.get("require")
                .and_then(|r| r.get("symfony/framework-bundle"))
                .is_some()
                || json
                    .get("require-dev")
                    .and_then(|r| r.get("symfony/framework-bundle"))
                    .is_some()
        });
    if !has_bundle {
        return Err(InitError::NotSymfony(
            "composer.json does not require symfony/framework-bundle".to_owned(),
        ));
    }

    let symfony_version = extract_symfony_version(&composer_content);

    let mut content = String::new();
    content.push_str("# sfc configuration\n");
    if let Some(version) = &symfony_version {
        let _ = writeln!(content, "# Detected Symfony {version}");
    }
    content.push('\n');
    content.push_str("[project]\n");
    content.push_str("# root = \".\"\n");
    content.push_str("# cache_dir = \"var/cache/prod\"\n");
    content.push_str("# src_dir = \"src\"\n");
    content.push('\n');
    content.push_str("[analyze]\n");
    content.push_str("format = \"terminal\"\n");
    content.push_str("exclude_bundles = []\n");
    content.push('\n');
    content.push_str("[preload]\n");
    content.push_str("# output = \"var/cache/prod/preload.php\"\n");
    content.push_str("# exclude_namespaces = []\n");
    content.push_str("# max_classes = 0\n");
    content.push_str("# scan_vendor = true\n");

    std::fs::write(&sfc_toml, content)?;
    Ok(())
}

fn extract_symfony_version(composer_content: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(composer_content).ok()?;
    let version = json
        .get("require")
        .and_then(|r| r.get("symfony/framework-bundle"))
        .or_else(|| {
            json.get("require-dev")
                .and_then(|r| r.get("symfony/framework-bundle"))
        })
        .and_then(|v| v.as_str())?;
    let cleaned = version.trim_start_matches(|c: char| !c.is_ascii_digit());
    let end = cleaned
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(cleaned.len());
    Some(cleaned[..end].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn init_creates_sfc_toml() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("composer.json"),
            r#"{"require":{"symfony/framework-bundle":"^7.0"}}"#,
        )
        .unwrap();
        run(dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join("sfc.toml")).unwrap();
        assert!(content.contains("[analyze]"));
        assert!(content.contains("Symfony 7.0"));
    }

    #[test]
    fn init_fails_if_exists() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("composer.json"),
            r#"{"require":{"symfony/framework-bundle":"^7.0"}}"#,
        )
        .unwrap();
        fs::write(dir.path().join("sfc.toml"), "existing").unwrap();
        let err = run(dir.path()).unwrap_err();
        assert!(matches!(err, InitError::AlreadyExists(_)));
    }

    #[test]
    fn init_fails_without_symfony() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("composer.json"),
            r#"{"require":{"laravel/framework":"^10"}}"#,
        )
        .unwrap();
        let err = run(dir.path()).unwrap_err();
        assert!(matches!(err, InitError::NotSymfony(_)));
    }

    #[test]
    fn init_fails_without_composer() {
        let dir = tempfile::tempdir().unwrap();
        let err = run(dir.path()).unwrap_err();
        assert!(matches!(err, InitError::NotSymfony(_)));
    }

    #[test]
    fn extract_version() {
        let content = r#"{"require":{"symfony/framework-bundle":"^8.0"}}"#;
        assert_eq!(extract_symfony_version(content), Some("8.0".to_owned()));
    }
}
