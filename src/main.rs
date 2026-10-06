//! NixDirStat — disk usage analyzer for Unix and Unix-like systems.

use std::process::ExitCode;

fn main() -> ExitCode {
    match nixdirstat::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err:#}");
            ExitCode::FAILURE
        },
    }
}
