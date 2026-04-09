use std::io::Write;

use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

use super::Report;
use crate::analyzer::{Impact, Severity};

/// # Errors
/// Returns an error if writing to stdout fails.
pub fn render(report: &Report) -> std::io::Result<()> {
    let mut stdout = StandardStream::stdout(ColorChoice::Auto);

    stdout.set_color(ColorSpec::new().set_bold(true))?;
    writeln!(stdout, "sfc analyze — {}", report.project_path.display())?;
    stdout.reset()?;
    writeln!(stdout, "Completed in {:.0?}\n", report.duration)?;

    if report.findings.is_empty() {
        stdout.set_color(ColorSpec::new().set_fg(Some(Color::Green)).set_bold(true))?;
        writeln!(stdout, "No findings.")?;
        stdout.reset()?;
        return Ok(());
    }

    for severity in [Severity::Critical, Severity::Warning, Severity::Info] {
        let findings: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.severity == severity)
            .collect();
        if findings.is_empty() {
            continue;
        }

        let (color, label) = match severity {
            Severity::Critical => (Color::Red, "CRITICAL"),
            Severity::Warning => (Color::Yellow, "WARNING"),
            Severity::Info => (Color::Cyan, "INFO"),
        };

        stdout.set_color(ColorSpec::new().set_fg(Some(color)).set_bold(true))?;
        writeln!(stdout, "  {label} ({} findings)", findings.len())?;
        stdout.reset()?;

        for f in findings {
            write!(stdout, "    ")?;
            if let Some(ref sid) = f.service_id {
                stdout.set_color(ColorSpec::new().set_bold(true))?;
                write!(stdout, "{sid}")?;
                stdout.reset()?;
                write!(stdout, " — ")?;
            }
            writeln!(stdout, "{}", f.message)?;

            if let Impact::Memory { estimated_bytes } = &f.impact {
                stdout.set_color(ColorSpec::new().set_dimmed(true))?;
                #[allow(clippy::cast_precision_loss)]
                writeln!(stdout, "      ~{:.1} KB", *estimated_bytes as f64 / 1024.0)?;
                stdout.reset()?;
            }
        }
        writeln!(stdout)?;
    }

    stdout.set_color(ColorSpec::new().set_bold(true))?;
    write!(stdout, "Summary: ")?;
    stdout.reset()?;
    writeln!(
        stdout,
        "{} critical, {} warnings, {} info",
        report.count_by_severity(Severity::Critical),
        report.count_by_severity(Severity::Warning),
        report.count_by_severity(Severity::Info),
    )?;

    Ok(())
}
