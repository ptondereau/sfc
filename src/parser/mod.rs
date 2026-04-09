pub mod container;
pub mod roles;
pub mod routes;
pub mod util;

use std::path::Path;
use thiserror::Error;

use crate::model::Container;
use crate::project::find_container_dir;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("no Container* directory found in {0}")]
    NoContainerDir(String),
    #[error("failed to read file {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("PHP parse error in {file}: {message}")]
    Php { file: String, message: String },
    #[error("unexpected container structure in {file}: {detail}")]
    Structure { file: String, detail: String },
}

/// # Errors
/// Returns `ParseError` if no container directory is found or PHP parsing fails.
pub fn parse_container(cache_dir: &Path) -> Result<Container, ParseError> {
    let container_dir = find_container_dir(cache_dir)
        .ok_or_else(|| ParseError::NoContainerDir(cache_dir.display().to_string()))?;

    let mut container = Container::new(cache_dir.to_path_buf());
    container::parse_main_container(&container_dir, &mut container)?;
    container::parse_service_factories(&container_dir, &mut container)?;
    container.routes = routes::parse_routes(cache_dir)?;
    container::resolve_string_references(&container_dir, &mut container)?;
    roles::infer_roles(&container_dir, &mut container)?;
    Ok(container)
}
