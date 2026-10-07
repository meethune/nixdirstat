//! NixDirStat — disk usage analyzer for Unix and Unix-like systems.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match nixdirstat::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            if let Some(e) = err.downcast_ref::<nixdirstat::ScanError>() {
                eprintln!("Error: {}", e.localized_message());
            } else if let Some(e) = err.downcast_ref::<nixdirstat::StorageError>() {
                eprintln!("Error: {}", e.localized_message());
            } else {
                eprintln!("Error: {err:#}");
            }
            ExitCode::FAILURE
        },
    }
}
