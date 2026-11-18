//! The AWS Lambda entry point (`provided.al2023`, arm64): Lambda runs the binary named
//! `bootstrap`, which serves the Function URL's requests through the same router.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    marcoferra_server::logs();
    let served = match marcoferra_server::lambda_router().await {
        Ok(router) => lambda_http::run(router).await,
        Err(err) => Err(err),
    };
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(error = %err, "the function stopped on an error");
            ExitCode::FAILURE
        }
    }
}
