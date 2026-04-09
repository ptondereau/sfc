pub mod backup;
pub mod dead;
pub mod preload;
pub mod rewrite;
pub mod unreachable;
pub mod util;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum OptimizeError {
    #[error("failed to create backup: {0}")]
    Backup(#[from] std::io::Error),
    #[error("no backup found to restore")]
    NoBackup,
    #[error("analysis failed: {0}")]
    Analysis(String),
}

#[derive(Debug, Default)]
pub struct OptimizeResult {
    pub level1_files_removed: usize,
    pub level1_bytes_freed: u64,
    pub level2_files_removed: usize,
    pub level2_bytes_freed: u64,
}

impl OptimizeResult {
    #[must_use]
    pub fn total_files(&self) -> usize {
        self.level1_files_removed + self.level2_files_removed
    }

    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn total_bytes(&self) -> u64 {
        self.level1_bytes_freed + self.level2_bytes_freed
    }
}
