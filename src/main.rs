use clap::Parser;

use cerberus_mcp::cli::{Cli, Command};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // `version` works even without a valid environment.
    if matches!(cli.command, Command::Version) {
        let ctx = cerberus_mcp::cli::output::Ctx {
            json: cli.json,
            quiet: cli.quiet,
            paths: match cerberus_mcp::paths::CerberusPaths::resolve_with_override(
                cli.config.clone(),
            ) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            },
        };
        let code = match cerberus_mcp::commands::dispatch(&ctx, Command::Version).await {
            Ok(code) => code,
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        };
        std::process::exit(code);
    }

    let result = cerberus_mcp::cli::run(cli).await;
    match result {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("error: {e}");
            let mut source = std::error::Error::source(&e);
            while let Some(inner) = source {
                eprintln!("  caused by: {inner}");
                source = inner.source();
            }
            std::process::exit(e.exit_code() as i32);
        }
    }
}
