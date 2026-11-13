//! The rustocop CLI.

use std::path::PathBuf;

use clap::Parser;
use rustocop::{config, cops::Registry, output, pipeline};

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
}

fn main() {
    std::process::exit(real_main());
}

fn real_main() -> i32 {
    let cli = Cli::parse();
    let registry = Registry::new();

    let run = || -> anyhow::Result<i32> {
        let config = config::ResolvedConfig::load(&registry)?;
        if cli.show_ruby_version {
            println!("{}", config.ruby_version_report());
            return Ok(pipeline::EXIT_CLEAN);
        }
        let options = pipeline::RunOptions {
            only: cli.only.clone(),
            except: cli.except.clone(),
            stdin: cli.stdin.clone(),
            fix: cli.fix,
            fix_passes: cli.fix_passes,
        };
        let report = pipeline::run(&cli.paths, &options, &config, &registry)?;
        let rendered = match cli.format {
            output::Format::Text => output::render_text(&report),
            output::Format::Github => output::render_github(&report),
        };
        print!("{rendered}");
        eprint!("{}", output::render_internal_errors(&report));
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
