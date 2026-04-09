use std::io::Write;

use ariadne::{Color, Label, Report as AriadneReport, ReportKind, Source};

use super::Report;
use crate::analyzer::{Impact, Severity};

/// # Errors
/// Returns an error if writing to stdout fails.
pub fn render(report: &Report) -> std::io::Result<()> {
    let mut stderr = std::io::stderr();

    writeln!(
        stderr,
        "\x1b[1msfc analyze — {}\x1b[0m",
        report.project_path.display()
    )?;
    writeln!(stderr, "Completed in {:.0?}\n", report.duration)?;

    if report.findings.is_empty() {
        writeln!(stderr, "\x1b[1;32mNo findings.\x1b[0m")?;
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

        let (ansi, label) = match severity {
            Severity::Critical => ("\x1b[1;31m", "CRITICAL"),
            Severity::Warning => ("\x1b[1;33m", "WARNING"),
            Severity::Info => ("\x1b[1;36m", "INFO"),
        };
        writeln!(
            stderr,
            "  {ansi}{label} ({} findings)\x1b[0m",
            findings.len()
        )?;

        for f in findings {
            if let Some(path) = &f.file
                && let Some(span) = &f.span
            {
                render_annotated(f, path, span.clone())?;
            } else {
                render_plain(&mut stderr, f)?;
            }
        }

        writeln!(stderr)?;
    }

    writeln!(
        stderr,
        "\x1b[1mSummary:\x1b[0m {} critical, {} warnings, {} info",
        report.count_by_severity(Severity::Critical),
        report.count_by_severity(Severity::Warning),
        report.count_by_severity(Severity::Info),
    )?;

    Ok(())
}

fn render_annotated(
    f: &crate::analyzer::Finding,
    path: &std::path::Path,
    span: std::ops::Range<usize>,
) -> std::io::Result<()> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return render_plain(&mut std::io::stderr(), f);
    };

    let file_id = path.display().to_string();
    let kind = match f.severity {
        Severity::Critical => ReportKind::Error,
        Severity::Warning => ReportKind::Warning,
        Severity::Info => ReportKind::Advice,
    };

    let color = match f.severity {
        Severity::Critical => Color::Red,
        Severity::Warning => Color::Yellow,
        Severity::Info => Color::Cyan,
    };

    let ariadne_span = (file_id.clone(), span);

    let mut builder = AriadneReport::build(kind, ariadne_span.clone())
        .with_message(&f.message)
        .with_label(
            Label::new(ariadne_span)
                .with_message(&f.message)
                .with_color(color),
        );

    if let Some(ref fix) = f.fix {
        builder = builder.with_help(fix);
    }

    builder
        .finish()
        .eprint((file_id, Source::from(content)))
        .map_err(std::io::Error::other)
}

fn render_plain(w: &mut impl Write, f: &crate::analyzer::Finding) -> std::io::Result<()> {
    write!(w, "    ")?;
    if let Some(ref sid) = f.service_id {
        write!(w, "\x1b[1m{sid}\x1b[0m — ")?;
    }
    writeln!(w, "{}", f.message)?;

    if let Impact::Memory { estimated_bytes } = &f.impact {
        #[allow(clippy::cast_precision_loss)]
        writeln!(
            w,
            "\x1b[2m      ~{:.1} KB\x1b[0m",
            *estimated_bytes as f64 / 1024.0
        )?;
    }

    if let Some(ref fix) = f.fix {
        for line in fix.lines() {
            writeln!(w, "\x1b[2m      {line}\x1b[0m")?;
        }
    }

    Ok(())
}
