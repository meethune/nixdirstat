//! NixDirStat — disk usage analyzer and cleanup assistant for POSIX-compliant systems.
//!
//! Scan local filesystems, devices, and directories then explore disk usage
//! through sortable file lists, file-type statistics, and interactive treemaps.

mod cli;

/// Application entry point, called from `main.rs`.
///
/// Parses CLI arguments, dispatches to the appropriate subcommand,
/// and returns any errors for display by the caller.
///
/// # Errors
///
/// Returns an error if CLI parsing, scanning, or UI initialization fails.
pub fn run() -> anyhow::Result<()> {
    let _cli = cli::parse();
    Ok(())
}
