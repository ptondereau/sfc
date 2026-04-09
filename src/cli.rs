use clap::{Arg, Command};

#[must_use]
pub fn build() -> Command {
    Command::new("sfc")
        .version("0.1.0")
        .about("Symfony Companion CLI — post-build analysis for Symfony projects")
        .subcommand_required(true)
        .subcommand(
            Command::new("analyze")
                .about("Read-only static audit of the prod build")
                .arg(
                    Arg::new("path")
                        .help("Path to Symfony project root")
                        .default_value("."),
                )
                .arg(
                    Arg::new("format")
                        .long("format")
                        .help("Output format")
                        .value_parser(["terminal", "json"])
                        .default_value("terminal"),
                )
                .arg(
                    Arg::new("cache-dir")
                        .long("cache-dir")
                        .help("Override path to var/cache/prod"),
                ),
        )
        .subcommand(
            Command::new("init")
                .about("Bootstrap sfc.toml with auto-detected values")
                .arg(
                    Arg::new("path")
                        .help("Path to Symfony project root")
                        .default_value("."),
                ),
        )
        .subcommand(
            Command::new("preload")
                .about("Generate smart preload.php from the prod cache")
                .arg(
                    Arg::new("path")
                        .help("Path to Symfony project root")
                        .default_value("."),
                )
                .arg(
                    Arg::new("output")
                        .long("output")
                        .help("Output path for preload.php"),
                )
                .arg(
                    Arg::new("no-vendor")
                        .long("no-vendor")
                        .help("Skip scanning vendor/ directory")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("optimize")
                .about("Post-warmup optimization of compiled cache files")
                .arg(
                    Arg::new("path")
                        .help("Path to Symfony project root")
                        .default_value("."),
                )
                .arg(
                    Arg::new("level")
                        .short('O')
                        .long("level")
                        .help("Optimization level (1=dead code, 2=dead+unreachable)")
                        .value_parser(["1", "2"])
                        .default_value("1"),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .help("Show what would change without modifying files")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("restore")
                        .long("restore")
                        .help("Restore from the latest backup")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_subcommand_parses() {
        let m = build()
            .try_get_matches_from(["sfc", "analyze"])
            .expect("should parse");
        let (name, _) = m.subcommand().expect("should have subcommand");
        assert_eq!(name, "analyze");
    }

    #[test]
    fn analyze_with_format_json() {
        let m = build()
            .try_get_matches_from(["sfc", "analyze", "--format", "json"])
            .expect("should parse");
        let (_, sub) = m.subcommand().unwrap();
        assert_eq!(sub.get_one::<String>("format").unwrap(), "json");
    }

    #[test]
    fn analyze_with_custom_path() {
        let m = build()
            .try_get_matches_from(["sfc", "analyze", "/tmp/myapp"])
            .expect("should parse");
        let (_, sub) = m.subcommand().unwrap();
        assert_eq!(sub.get_one::<String>("path").unwrap(), "/tmp/myapp");
    }

    #[test]
    fn no_subcommand_fails() {
        let result = build().try_get_matches_from(["sfc"]);
        assert!(result.is_err());
    }

    #[test]
    fn init_subcommand_parses() {
        let m = build()
            .try_get_matches_from(["sfc", "init"])
            .expect("should parse");
        let (name, _) = m.subcommand().unwrap();
        assert_eq!(name, "init");
    }

    #[test]
    fn preload_subcommand_parses() {
        let m = build()
            .try_get_matches_from(["sfc", "preload"])
            .expect("should parse");
        let (name, _) = m.subcommand().unwrap();
        assert_eq!(name, "preload");
    }

    #[test]
    fn preload_with_no_vendor() {
        let m = build()
            .try_get_matches_from(["sfc", "preload", "--no-vendor"])
            .expect("should parse");
        let (_, sub) = m.subcommand().unwrap();
        assert!(sub.get_flag("no-vendor"));
    }

    #[test]
    fn optimize_subcommand_parses() {
        let m = build()
            .try_get_matches_from(["sfc", "optimize"])
            .expect("should parse");
        let (name, sub) = m.subcommand().unwrap();
        assert_eq!(name, "optimize");
        assert_eq!(sub.get_one::<String>("level").unwrap(), "1");
    }

    #[test]
    fn optimize_with_level_2() {
        let m = build()
            .try_get_matches_from(["sfc", "optimize", "-O", "2"])
            .expect("should parse");
        let (_, sub) = m.subcommand().unwrap();
        assert_eq!(sub.get_one::<String>("level").unwrap(), "2");
    }

    #[test]
    fn optimize_dry_run() {
        let m = build()
            .try_get_matches_from(["sfc", "optimize", "--dry-run"])
            .expect("should parse");
        let (_, sub) = m.subcommand().unwrap();
        assert!(sub.get_flag("dry-run"));
    }
}
