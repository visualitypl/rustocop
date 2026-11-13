//! The rustocop CLI.

use std::{io::Write as _, path::PathBuf};

use anyhow::Context as _;
use clap::Parser;
use rustocop::{baseline, config, cops::Registry, import, output, pipeline};

#[derive(Parser)]
#[command(
    name = "rustocop",
    version,
    about = "A blazing-fast, opinionated Ruby linter",
    after_help = "No config file is required; add rustocop.toml to customize."
)]
struct Cli {
    /// Files or directories to lint.
    #[arg(default_value = ".")]
    paths: Vec<PathBuf>,

    /// Run only the given cops or departments (comma-separated).
    #[arg(long, value_delimiter = ',')]
    only: Vec<String>,

    /// Skip the given cops or departments (comma-separated).
    #[arg(long, value_delimiter = ',')]
    except: Vec<String>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = output::Format::Text)]
    format: output::Format,

    /// Apply safe corrections and rewrite files in place.
    #[arg(long, conflicts_with = "stdin")]
    fix: bool,

    /// Stop after N correction passes over each file (default: until nothing changes).
    #[arg(long, value_name = "N", requires = "fix")]
    fix_passes: Option<usize>,

    /// Read source from stdin, using PATH as the display name.
    #[arg(long, value_name = "PATH")]
    stdin: Option<PathBuf>,

    /// Print the Ruby version that code is checked for, and where it comes from.
    #[arg(long, conflicts_with_all = ["fix", "stdin"])]
    show_ruby_version: bool,

    /// Write a rustocop.toml that silences the offenses the code has now.
    #[arg(long, conflicts_with_all = ["fix", "stdin", "only", "except", "show_ruby_version"])]
    write_baseline: bool,

    /// Write a rustocop.toml with the settings of .rubocop.yml that it can hold.
    #[arg(long, conflicts_with_all = ["fix", "stdin", "only", "except", "show_ruby_version", "write_baseline"])]
    import_rubocop_config: bool,
}

fn main() {
    std::process::exit(real_main());
}

fn real_main() -> i32 {
    let cli = Cli::parse();
    let registry = Registry::new();

    let run = || -> anyhow::Result<i32> {
        if cli.import_rubocop_config {
            return import_rubocop_config(&registry);
        }
        let config = config::ResolvedConfig::load(&registry)?;
        if cli.show_ruby_version {
            println!("{}", config.ruby_version_report());
            return Ok(pipeline::EXIT_CLEAN);
        }
        let baseline_path = config.root.join(config::CONFIG_FILE_NAME);
        if cli.write_baseline && baseline_path.exists() {
            anyhow::bail!(
                "{} already exists; --write-baseline does not change an existing config",
                baseline_path.display()
            );
        }
        let options = pipeline::RunOptions {
            only: cli.only.clone(),
            except: cli.except.clone(),
            stdin: cli.stdin.clone(),
            fix: cli.fix,
            fix_passes: cli.fix_passes,
        };
        let report = pipeline::run(&cli.paths, &options, &config, &registry)?;
        eprint!("{}", output::render_internal_errors(&report));
        if cli.write_baseline {
            return write_baseline(
                report,
                &cli.paths,
                &options,
                &config,
                &registry,
                &baseline_path,
            );
        }
        let rendered = match cli.format {
            output::Format::Text => output::render_text(&report),
            output::Format::Github => output::render_github(&report),
            output::Format::Offenses => output::render_offenses(&report),
        };
        print!("{rendered}");
        Ok(report.exit_code())
    };

    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            pipeline::EXIT_ERRORS
        }
    }
}

/// A cop that a baseline silences can uncover offenses of another cop, so
/// the baseline is checked by linting with it, this many times at most.
const BASELINE_PASSES: usize = 10;

/// Write to `path` the baseline config for `report`, the run of `paths` with
/// `config`, and say what it covers. File errors are listed: no config can
/// silence them.
fn write_baseline(
    mut report: pipeline::RunReport,
    paths: &[PathBuf],
    options: &pipeline::RunOptions,
    config: &config::ResolvedConfig,
    registry: &Registry,
    path: &std::path::Path,
) -> anyhow::Result<i32> {
    let mut baseline = baseline::Baseline::default();
    for _ in 0..BASELINE_PASSES {
        if report.offense_count() == 0 {
            break;
        }
        baseline.add(&report, &config.root);
        // Silencing a cop can make another one report, for example a
        // `rubocop:disable` comment that is now redundant: lint with the
        // baseline and add what is left, until a run is clean.
        let raw = toml::from_str(&baseline.toml()).context("invalid baseline config")?;
        let mut next = config::ResolvedConfig::resolve(raw, Some(path), registry)?;
        next.ruby_version = config.ruby_version;
        next.ruby_version_source = config.ruby_version_source.clone();
        report = pipeline::run(paths, options, &next, registry)?;
    }
    for error in &report.errors {
        println!("{}: error: {}", error.path.display(), error.message);
    }
    if baseline.offenses() == 0 {
        println!("No offenses; no baseline is needed.");
    } else {
        write_new(path, &baseline.toml())?;
        println!("Wrote {}: {}.", path.display(), baseline.summary());
        let left = report.offense_count();
        if left > 0 {
            println!("Still reported: {left}.");
        }
    }
    Ok(if report.has_errors() {
        pipeline::EXIT_ERRORS
    } else {
        pipeline::EXIT_CLEAN
    })
}

/// Write `text` to the new file `path`; never replace a config, also one
/// written meanwhile.
fn write_new(path: &std::path::Path, text: &str) -> anyhow::Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .with_context(|| format!("could not write {}", path.display()))
}

/// Write a `rustocop.toml` in the current directory from its `.rubocop.yml`,
/// and list the settings that are not carried over.
fn import_rubocop_config(registry: &Registry) -> anyhow::Result<i32> {
    let path = std::path::Path::new(config::CONFIG_FILE_NAME);
    if path.exists() {
        anyhow::bail!(
            "{} already exists; --import-rubocop-config does not change an existing config",
            path.display()
        );
    }
    let source = std::path::Path::new(import::RUBOCOP_FILE_NAME);
    let import = import::import(source, registry)?;
    if import.toml.is_empty() {
        println!(
            "No setting of {} has a rustocop counterpart; no {} is written.",
            import.files.join(" and "),
            path.display()
        );
    } else {
        // The config must be one that rustocop loads.
        let raw = toml::from_str(&import.toml).context("invalid imported config")?;
        config::ResolvedConfig::resolve(raw, Some(path), registry)?;
        write_new(path, &import.toml)?;
        println!(
            "Wrote {} from {}.",
            path.display(),
            import.files.join(" and ")
        );
    }
    if !import.dropped.is_empty() {
        println!("Not carried over; rustocop has no setting for these:");
        for line in &import.dropped {
            println!("  {line}");
        }
    }
    Ok(pipeline::EXIT_CLEAN)
}
