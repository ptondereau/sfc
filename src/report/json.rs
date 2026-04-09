use serde::Serialize;

use super::Report;
use crate::analyzer::{Impact, Severity};

#[derive(Serialize)]
struct JsonReport<'a> {
    project: String,
    duration_ms: u64,
    summary: JsonSummary,
    findings: Vec<JsonFinding<'a>>,
}

#[derive(Serialize)]
struct JsonSummary {
    critical: usize,
    warnings: usize,
    info: usize,
}

#[derive(Serialize)]
struct JsonFinding<'a> {
    pass: &'a str,
    severity: &'static str,
    message: &'a str,
    service_id: Option<&'a str>,
    file: Option<String>,
    impact: Option<JsonImpact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fix: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum JsonImpact {
    #[serde(rename = "memory")]
    Memory { estimated_bytes: u64 },
    #[serde(rename = "startup")]
    Startup { estimated_ms: u32 },
}

/// # Panics
/// Panics if JSON serialization fails (should not happen with valid data).
pub fn render(report: &Report) {
    let json = JsonReport {
        project: report.project_path.display().to_string(),
        #[allow(clippy::cast_possible_truncation)]
        duration_ms: report.duration.as_millis() as u64,
        summary: JsonSummary {
            critical: report.count_by_severity(Severity::Critical),
            warnings: report.count_by_severity(Severity::Warning),
            info: report.count_by_severity(Severity::Info),
        },
        findings: report.findings.iter().map(to_json_finding).collect(),
    };

    let out = serde_json::to_string_pretty(&json).expect("serialization should not fail");
    println!("{out}");
}

fn to_json_finding(f: &crate::analyzer::Finding) -> JsonFinding<'_> {
    JsonFinding {
        pass: f.pass,
        severity: match f.severity {
            Severity::Critical => "critical",
            Severity::Warning => "warning",
            Severity::Info => "info",
        },
        message: &f.message,
        service_id: f.service_id.as_ref().map(|s| s.0.as_str()),
        file: f.file.as_ref().map(|p| p.display().to_string()),
        impact: match &f.impact {
            Impact::Memory { estimated_bytes } => Some(JsonImpact::Memory {
                estimated_bytes: *estimated_bytes,
            }),
            Impact::Startup { estimated_ms } => Some(JsonImpact::Startup {
                estimated_ms: *estimated_ms,
            }),
            Impact::None => None,
        },
        fix: f.fix.as_deref(),
    }
}
