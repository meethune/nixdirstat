//! Command-line argument parsing via `clap`.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Disk usage analyzer and cleanup assistant for Unix and Unix-like systems.
#[derive(Debug, Parser)]
#[command(name = "nixdirstat", version, about)]
pub struct Cli {
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Command,
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

/// Parse command-line arguments.
pub fn parse() -> Cli {
    Cli::parse()
}
