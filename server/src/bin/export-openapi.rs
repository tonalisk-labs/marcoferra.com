//! Prints the `OpenAPI` contract the site's client is generated from. `mise run contract` checks
//! that `server/openapi.yaml` holds it.

use std::process::ExitCode;

fn main() -> ExitCode {
    kit_server::openapi::print("export-openapi", marcoferra_server::openapi_yaml())
}
