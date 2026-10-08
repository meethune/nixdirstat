//! Command-line argument parsing via `clap`.

use std::io::Write;
use std::path::PathBuf;

use clap::{CommandFactory as _, Parser, Subcommand};

/// Disk usage analyzer and cleanup assistant for Unix and Unix-like systems.
#[derive(Debug, Parser)]
#[command(
    name = "nixdirstat",
    version,
    about,
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Path to scan (shorthand for `nixdirstat scan <path>`).
    path: Option<PathBuf>,

    /// Override the detected locale (e.g., "fr", "de", "ja").
    #[arg(long, global = true)]
    lang: Option<String>,
}

/// Available subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Scan a directory tree and collect filesystem metadata.
    Scan {
        /// Root path to scan.
        path: PathBuf,

        /// Save scan results to a file instead of opening the TUI.
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Follow cross-device mount points.
        #[arg(long)]
        cross_device: bool,
    },

    /// Open a previously saved scan file in the interactive TUI.
    Explore {
        /// Path to a saved scan database file.
        scan_file: PathBuf,
    },

    /// Generate shell completions for the given shell.
    #[command(hide = true)]
    Completions {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Export scan results to CSV or JSON.
    Export {
        /// Path to a saved scan database file.
        scan_file: PathBuf,

        /// Output format.
        #[arg(short, long, value_enum)]
        format: ExportFormat,
    },
}

/// Supported export formats.
#[derive(Debug, Clone, clap::ValueEnum)]
pub enum ExportFormat {
    /// Comma-separated values.
    Csv,
    /// JSON format.
    Json,
}

/// Resolve a parsed [`Cli`] into a [`Command`], treating a bare path as
/// `scan <path>`. Returns `None` when neither a subcommand nor a path is given.
fn resolve(cli: Cli) -> Option<Command> {
    cli.command.or_else(|| {
        cli.path.map(|path| Command::Scan {
            path,
            output: None,
            cross_device: false,
        })
    })
}

/// Write shell completions to the given writer.
pub fn write_completions(shell: clap_complete::Shell, out: &mut impl Write) {
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, "nixdirstat", out);
}

/// Parse command-line arguments, resolving bare path shorthand.
///
/// `nixdirstat <path>` is treated as `nixdirstat scan <path>`.
/// Prints help and exits if neither a subcommand nor a path is given.
///
/// Returns the resolved command and the optional `--lang` override.
pub fn parse() -> (Command, Option<String>) {
    let cli = Cli::parse();
    let lang = cli.lang.clone();
    let command = resolve(cli).unwrap_or_else(|| {
        Cli::command().print_help().ok();
        std::process::exit(2);
    });
    (command, lang)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Option<Command> {
        Cli::try_parse_from(args).ok().and_then(resolve)
    }

    #[test]
    fn bare_path_resolves_to_scan() {
        let cmd = parse_args(&["nixdirstat", "/tmp"]);
        assert!(
            matches!(
                cmd,
                Some(Command::Scan {
                    ref path,
                    output: None,
                    cross_device: false,
                }) if path.as_os_str() == "/tmp"
            ),
            "expected Scan with /tmp, got {cmd:?}"
        );
    }

    #[test]
    fn explicit_scan_subcommand_works() {
        let cmd = parse_args(&["nixdirstat", "scan", "/tmp"]);
        assert!(
            matches!(cmd, Some(Command::Scan { ref path, .. }) if path.as_os_str() == "/tmp"),
            "expected Scan with /tmp, got {cmd:?}"
        );
    }

    #[test]
    fn scan_with_cross_device_flag() {
        let cmd = parse_args(&["nixdirstat", "scan", "/tmp", "--cross-device"]);
        assert!(
            matches!(
                cmd,
                Some(Command::Scan {
                    cross_device: true,
                    ..
                })
            ),
            "expected Scan with cross_device=true, got {cmd:?}"
        );
    }

    #[test]
    fn no_args_resolves_to_none() {
        let cmd = parse_args(&["nixdirstat"]);
        assert!(cmd.is_none(), "expected None, got {cmd:?}");
    }

    #[test]
    fn explore_subcommand_works() {
        let cmd = parse_args(&["nixdirstat", "explore", "/tmp/scan.db"]);
        assert!(
            matches!(cmd, Some(Command::Explore { .. })),
            "expected Explore, got {cmd:?}"
        );
    }

    #[test]
    fn completions_subcommand_parses() {
        let cmd = parse_args(&["nixdirstat", "completions", "bash"]);
        assert!(
            matches!(cmd, Some(Command::Completions { .. })),
            "expected Completions, got {cmd:?}"
        );
    }

    #[test]
    fn completions_generates_nonempty_output() {
        let mut buf = Vec::new();
        write_completions(clap_complete::Shell::Bash, &mut buf);
        let output = String::from_utf8(buf).expect("completions should be valid UTF-8");
        assert!(
            output.contains("nixdirstat"),
            "completions should contain the binary name"
        );
        assert!(output.len() > 100, "completions should be substantial");
    }

    #[test]
    fn lang_flag_parses_with_scan() {
        let cli = Cli::try_parse_from(["nixdirstat", "scan", "/tmp", "--lang", "fr"]).unwrap();
        assert_eq!(cli.lang.as_deref(), Some("fr"));
    }

    #[test]
    fn lang_flag_absent_by_default() {
        let cli = Cli::try_parse_from(["nixdirstat", "scan", "/tmp"]).unwrap();
        assert!(cli.lang.is_none());
    }
}
