//! The service on a local port, for development and the E2E suite: the same router Lambda serves.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    marcoferra_server::logs();
    match marcoferra_server::serve().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(error = %err, "the service stopped on an error");
            ExitCode::FAILURE
        }
    }
}
