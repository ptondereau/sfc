pub mod collector;
pub mod generator;

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
