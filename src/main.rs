//! NixDirStat — disk usage analyzer for Unix and Unix-like systems.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match nixdirstat::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err:#}");
            ExitCode::FAILURE
        },
    }
}
