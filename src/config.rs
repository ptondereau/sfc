use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid config: {0}")]
    Parse(#[from] toml::de::Error),
}

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub project: ProjectConfig,
    #[serde(default)]
    pub analyze: AnalyzeConfig,
    #[serde(default)]
    pub preload: PreloadConfig,
}

#[derive(Debug, Deserialize, Default)]
pub struct ProjectConfig {
    pub root: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    pub src_dir: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Terminal,
    Json,
}

#[derive(Debug, Deserialize, Default)]
pub struct AnalyzeConfig {
    #[serde(default)]
    pub format: OutputFormat,
    #[serde(default)]
    #[allow(dead_code)]
    pub exclude_bundles: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct PreloadConfig {
    #[serde(default = "default_preload_output")]
    pub output: PathBuf,
    #[serde(default)]
    pub exclude_namespaces: Vec<String>,
    #[serde(default)]
    pub max_classes: usize,
    #[serde(default = "default_true")]
    pub scan_vendor: bool,
}

impl Default for PreloadConfig {
    fn default() -> Self {
        Self {
            output: default_preload_output(),
            exclude_namespaces: Vec::new(),
            max_classes: 0,
            scan_vendor: true,
        }
    }
}

fn default_preload_output() -> PathBuf {
    PathBuf::from("var/cache/prod/preload.php")
}

fn default_true() -> bool {
    true
}

impl Config {
    /// # Errors
    /// Returns `ConfigError::Io` if the file cannot be read, or `ConfigError::Parse` on invalid TOML.
    pub fn load(project_root: &Path) -> Result<Self, ConfigError> {
        let path = project_root.join("sfc.toml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn default_config_when_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.analyze.format, OutputFormat::Terminal);
        assert!(config.project.root.is_none());
    }

    #[test]
    fn parse_minimal_toml() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("sfc.toml"),
            "[analyze]\nformat = \"json\"\n",
        )
        .unwrap();
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.analyze.format, OutputFormat::Json);
    }

    #[test]
    fn parse_full_toml() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("sfc.toml"),
            r#"
[project]
root = "/app"
cache_dir = "/app/var/cache/prod"
src_dir = "/app/src"

[analyze]
format = "terminal"
exclude_bundles = ["DebugBundle"]
"#,
        )
        .unwrap();
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.project.root, Some(PathBuf::from("/app")));
        assert_eq!(config.analyze.exclude_bundles, vec!["DebugBundle"]);
    }

    #[test]
    fn invalid_toml_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("sfc.toml"), "not valid toml {{{").unwrap();
        assert!(Config::load(dir.path()).is_err());
    }

    #[test]
    fn parse_preload_config() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("sfc.toml"),
            r#"
[preload]
output = "preload.php"
exclude_namespaces = ["App\\Tests"]
max_classes = 500
scan_vendor = false
"#,
        )
        .unwrap();
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.preload.output, PathBuf::from("preload.php"));
        assert_eq!(config.preload.exclude_namespaces, vec!["App\\Tests"]);
        assert_eq!(config.preload.max_classes, 500);
        assert!(!config.preload.scan_vendor);
    }

    #[test]
    fn preload_config_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(
            config.preload.output,
            PathBuf::from("var/cache/prod/preload.php")
        );
        assert!(config.preload.scan_vendor);
        assert_eq!(config.preload.max_classes, 0);
    }
}
