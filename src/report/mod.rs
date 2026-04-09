pub mod json;
pub mod terminal;

use std::path::PathBuf;
use std::time::Duration;

use crate::analyzer::{Finding, Severity};

pub struct Report {
    pub project_path: PathBuf,
    pub findings: Vec<Finding>,
    pub duration: Duration,
}

impl Report {
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        i32::from(
            self.findings
                .iter()
                .any(|f| matches!(f.severity, Severity::Critical)),
        )
    }

    #[must_use]
    pub fn count_by_severity(&self, severity: Severity) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == severity)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::Impact;

    fn make_finding(severity: Severity) -> Finding {
        Finding {
            pass: "test",
            severity,
            message: "test".to_owned(),
            service_id: None,
            file: None,
            span: None,
            impact: Impact::None,
            fix: None,
        }
    }

    #[test]
    fn exit_code_zero_without_critical() {
        let r = Report {
            project_path: PathBuf::from("/tmp"),
            findings: vec![
                make_finding(Severity::Warning),
                make_finding(Severity::Info),
            ],
            duration: Duration::from_millis(100),
        };
        assert_eq!(r.exit_code(), 0);
    }

    #[test]
    fn exit_code_one_with_critical() {
        let r = Report {
            project_path: PathBuf::from("/tmp"),
            findings: vec![make_finding(Severity::Critical)],
            duration: Duration::from_millis(100),
        };
        assert_eq!(r.exit_code(), 1);
    }
}
