pub mod dead;
pub mod listeners;
pub mod routes;
pub mod voters;
pub mod weight;

use std::path::PathBuf;

use crate::model::ServiceId;

pub trait AnalysisPass {
    fn name(&self) -> &'static str;
    fn run(&self, container: &crate::model::Container) -> Vec<Finding>;
}

#[derive(Debug)]
pub struct Finding {
    pub pass: &'static str,
    pub severity: Severity,
    pub message: String,
    pub service_id: Option<ServiceId>,
    pub file: Option<PathBuf>,
    pub impact: Impact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Impact {
    Memory {
        estimated_bytes: u64,
    },
    #[allow(dead_code)]
    Startup {
        estimated_ms: u32,
    },
    #[allow(dead_code)]
    None,
}

#[must_use]
pub fn run_passes(
    container: &crate::model::Container,
    passes: &[Box<dyn AnalysisPass>],
) -> Vec<Finding> {
    passes.iter().flat_map(|p| p.run(container)).collect()
}
