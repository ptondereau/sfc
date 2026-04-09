mod analyzer;
mod cli;
mod config;
mod init;
mod model;
mod optimizer;
mod parser;
mod preload;
mod project;
mod report;

use std::path::Path;
use std::time::Instant;

use analyzer::AnalysisPass;
use analyzer::dead::DeadServicesPass;
use analyzer::listeners::UnusedListenersPass;
use analyzer::routes::DeadRoutesPass;
use analyzer::voters::AlwaysLoadedVotersPass;
use analyzer::weight::ContainerWeightPass;
use config::OutputFormat;
use report::Report;

fn main() {
    let matches = cli::build().get_matches();

    let result = match matches.subcommand() {
        Some(("analyze", args)) => cmd_analyze(args),
        Some(("init", args)) => {
            let project_path = Path::new(args.get_one::<String>("path").unwrap());
            cmd_init(project_path)
        }
        Some(("preload", args)) => cmd_preload(args),
        Some(("optimize", args)) => cmd_optimize(args),
        _ => unreachable!(),
    };

    match result {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    }
}

fn cmd_init(project_path: &Path) -> Result<i32, Box<dyn std::error::Error>> {
    init::run(project_path)?;
    println!("Created sfc.toml");
    Ok(0)
}

fn cmd_analyze(args: &clap::ArgMatches) -> Result<i32, Box<dyn std::error::Error>> {
    let project_path = Path::new(args.get_one::<String>("path").unwrap());
    let format_str = args.get_one::<String>("format").unwrap();
    let cache_dir_override = args.get_one::<String>("cache-dir").map(Path::new);

    let start = Instant::now();

    let mut config = config::Config::load(project_path)?;

    config.analyze.format = match format_str.as_str() {
        "json" => OutputFormat::Json,
        _ => OutputFormat::Terminal,
    };
    if let Some(cache_dir) = cache_dir_override {
        config.project.cache_dir = Some(cache_dir.to_path_buf());
    }

    let project = project::detect(project_path, &config)?;
    let container = parser::parse_container(&project.cache_dir)?;

    let passes: Vec<Box<dyn AnalysisPass>> = vec![
        Box::new(DeadServicesPass),
        Box::new(ContainerWeightPass),
        Box::new(UnusedListenersPass::new(&project.src_dir)),
        Box::new(AlwaysLoadedVotersPass),
        Box::new(DeadRoutesPass),
    ];

    let findings = analyzer::run_passes(&container, &passes);

    let report = Report {
        project_path: project.root,
        findings,
        duration: start.elapsed(),
    };

    let exit_code = report.exit_code();

    match config.analyze.format {
        OutputFormat::Terminal => report::terminal::render(&report)?,
        OutputFormat::Json => report::json::render(&report),
    }

    Ok(exit_code)
}

fn cmd_preload(args: &clap::ArgMatches) -> Result<i32, Box<dyn std::error::Error>> {
    let project_path = Path::new(args.get_one::<String>("path").unwrap());
    let output_override = args.get_one::<String>("output");
    let no_vendor = args.get_flag("no-vendor");

    let mut config = config::Config::load(project_path)?;

    if let Some(output) = output_override {
        config.preload.output = std::path::PathBuf::from(output);
    }
    if no_vendor {
        config.preload.scan_vendor = false;
    }

    let project = project::detect(project_path, &config)?;

    let mut scan_dirs: Vec<&Path> = vec![&project.cache_dir];
    let vendor_dir = project.root.join("vendor");
    if config.preload.scan_vendor {
        scan_dirs.push(&vendor_dir);
    }

    let classes =
        preload::collector::collect_classes(&scan_dirs, &config.preload.exclude_namespaces)?;

    let output_path = if config.preload.output.is_relative() {
        project.root.join(&config.preload.output)
    } else {
        config.preload.output.clone()
    };

    let count = preload::generator::generate(&classes, &output_path, config.preload.max_classes)?;

    println!("Generated {} with {count} classes", output_path.display());
    Ok(0)
}

#[allow(clippy::cast_precision_loss)]
fn cmd_optimize(args: &clap::ArgMatches) -> Result<i32, Box<dyn std::error::Error>> {
    let project_path = Path::new(args.get_one::<String>("path").unwrap());
    let level: u8 = args.get_one::<String>("level").unwrap().parse().unwrap();
    let dry_run = args.get_flag("dry-run");
    let restore = args.get_flag("restore");

    let config = config::Config::load(project_path)?;
    let project = project::detect(project_path, &config)?;

    if restore {
        let backup = optimizer::backup::restore_latest(&project.cache_dir)?;
        println!("Restored from {}", backup.display());
        return Ok(0);
    }

    let container_dir =
        project::find_container_dir(&project.cache_dir).ok_or("no Container directory found")?;

    if !dry_run {
        let backup = optimizer::backup::create_backup(&container_dir)?;
        println!("Backup created: {}", backup.display());
    }

    let container = parser::parse_container(&project.cache_dir)?;
    let dead_passes: Vec<Box<dyn analyzer::AnalysisPass>> =
        vec![Box::new(analyzer::dead::DeadServicesPass)];
    let dead_findings = analyzer::run_passes(&container, &dead_passes);

    let dead_ids: std::collections::HashSet<String> = dead_findings
        .iter()
        .filter(|f| f.pass == "dead_services")
        .filter_map(|f| f.service_id.as_ref().map(|id| id.0.clone()))
        .collect();

    let mut result = optimizer::OptimizeResult::default();

    let level1 = optimizer::dead::remove_dead_services(&container_dir, &dead_ids, dry_run)?;
    result.level1_files_removed = level1.files_removed;
    result.level1_bytes_freed = level1.bytes_freed;
    let mut all_removed: std::collections::HashSet<String> =
        level1.removed_ids.into_iter().collect();

    if level >= 2 {
        let unreachable =
            optimizer::unreachable::find_unreachable_factories(&container_dir, &all_removed)?;
        let level2 = optimizer::dead::remove_dead_services(&container_dir, &unreachable, dry_run)?;
        result.level2_files_removed = level2.files_removed;
        result.level2_bytes_freed = level2.bytes_freed;
        all_removed.extend(level2.removed_ids);
    }

    if !all_removed.is_empty() {
        optimizer::rewrite::rewrite_maps(&container_dir, &all_removed, dry_run)?;
    }

    let prefix = if dry_run { "[dry-run] " } else { "" };
    if result.level1_files_removed > 0 {
        println!(
            "{prefix}Level 1: Removed {} dead service files ({:.1} KB)",
            result.level1_files_removed,
            result.level1_bytes_freed as f64 / 1024.0
        );
    }
    if result.level2_files_removed > 0 {
        println!(
            "{prefix}Level 2: Removed {} unreachable factory files ({:.1} KB)",
            result.level2_files_removed,
            result.level2_bytes_freed as f64 / 1024.0
        );
    }
    println!(
        "{prefix}Total: {} files removed, {:.1} KB freed",
        result.total_files(),
        result.total_bytes() as f64 / 1024.0
    );

    Ok(0)
}
