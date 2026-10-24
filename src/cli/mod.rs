//! CLI definition and dispatch.

pub mod output;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::error::Result;

#[derive(Debug, Parser)]
#[command(
    name = "cerberus",
    version,
    about = "Universal MCP management and gateway platform",
    long_about = "CerberusMCP manages a canonical registry of MCP servers and synchronizes \
configuration into multiple AI CLI clients. It optionally runs an MCP gateway with \
centralized policy enforcement.",
    propagate_version = true,
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Path to the registry file (default: OS config directory).
    #[arg(long, global = true, env = "CERBERUS_CONFIG_FILE", value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Emit machine-readable JSON on stdout.
    #[arg(long, global = true)]
    pub json: bool,

    /// Suppress non-essential output.
    #[arg(long, global = true)]
    pub quiet: bool,

    /// Increase log verbosity (info).
    #[arg(long, global = true)]
    pub verbose: bool,

    /// Enable debug logging (still redacts secrets).
    #[arg(long, global = true)]
    pub debug: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create an initial registry and data directories.
    Init {
        /// Overwrite an existing registry (a backup is created first).
        #[arg(long)]
        force: bool,
    },

    /// Inspect and validate configuration.
    #[command(subcommand)]
    Config(ConfigCommand),

    /// Manage registered MCP servers.
    #[command(subcommand)]
    Server(ServerCommand),

    /// Inspect and synchronize AI client integrations.
    #[command(subcommand)]
    Client(ClientCommand),

    /// Synchronize the registry into client configurations.
    Sync {
        /// Restrict to one client (repeatable).
        #[arg(long = "client", value_name = "NAME")]
        clients: Vec<String>,
        /// Write changes (default is a dry-run plan).
        #[arg(long, conflicts_with = "dry_run")]
        apply: bool,
        /// Show the plan without writing (default behavior; flag kept for clarity).
        #[arg(long)]
        dry_run: bool,
        /// Use a specific profile instead of the active one.
        #[arg(long)]
        profile: Option<String>,
        /// Overwrite conflicting entries owned by the user.
        #[arg(long)]
        force: bool,
    },

    /// Show differences between the registry and client configurations.
    Diff {
        #[arg(long = "client", value_name = "NAME")]
        clients: Vec<String>,
        #[arg(long)]
        profile: Option<String>,
    },

    /// Back up and restore managed configuration.
    #[command(subcommand)]
    Backup(BackupCommand),

    /// Manage named server profiles.
    #[command(subcommand)]
    Profile(ProfileCommand),

    /// Run the central MCP gateway.
    #[command(subcommand)]
    Gateway(GatewayCommand),

    /// Diagnose configuration, clients, and servers.
    Doctor {
        /// Only check one client.
        #[arg(long, value_name = "NAME")]
        client: Option<String>,
        /// Probe server processes (spawns processes).
        #[arg(long)]
        probe: bool,
    },

    /// Show registry and server health.
    Health,

    /// Show recent diagnostic logs.
    Logs {
        /// Number of lines to show.
        #[arg(long, default_value_t = 50)]
        lines: usize,
    },

    /// Print version and protocol information.
    Version,

    /// Generate shell completions.
    Completions {
        /// Target shell: bash, elvish, fish, PowerShell, zsh.
        shell: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print the active configuration and data paths.
    Path,
    /// Validate the registry.
    Validate,
    /// Show the registry (secrets redacted).
    Show,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)] // clap ergonomics over micro-optimization
pub enum ServerCommand {
    /// List registered servers.
    List {
        /// Filter by tag.
        #[arg(long)]
        tag: Option<String>,
    },
    /// Register a new server.
    Add {
        /// Server identifier ([a-z0-9-], no `__`).
        name: String,
        /// Transport: stdio (default) or http.
        #[arg(long, default_value = "stdio")]
        transport: String,
        /// Executable to launch (stdio).
        #[arg(long)]
        command: Option<String>,
        /// Arguments for the executable (stdio; repeatable).
        #[arg(long = "arg", value_name = "ARG")]
        args: Vec<String>,
        /// Remote URL (http transport).
        #[arg(long)]
        url: Option<String>,
        /// Environment binding: KEY=env:VAR, KEY=secret:NAME, or KEY=literal:value.
        #[arg(long = "env", value_name = "BINDING")]
        env: Vec<String>,
        /// Tags (comma-separated or repeatable).
        #[arg(long = "tag", value_name = "TAG")]
        tags: Vec<String>,
        /// Tool-call timeout in milliseconds.
        #[arg(long)]
        timeout_ms: Option<u64>,
        /// Startup timeout in milliseconds.
        #[arg(long)]
        startup_timeout_ms: Option<u64>,
        /// Execution mode: direct (default) or gateway.
        #[arg(long)]
        mode: Option<String>,
        /// Human-readable description.
        #[arg(long)]
        description: Option<String>,
    },
    /// Show details for one server.
    Inspect { name: String },
    /// Enable a server.
    Enable { name: String },
    /// Disable a server.
    Disable { name: String },
    /// Remove a server from the registry.
    Remove {
        name: String,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ClientCommand {
    /// List known clients and support status.
    List,
    /// Detect which clients are installed.
    Detect,
    /// Inspect a client's current configuration.
    Inspect {
        /// Client id: opencode, claude-code, codex, gemini-cli, cline.
        name: String,
    },
    /// Synchronize the registry into one client (dry-run unless --apply).
    Sync {
        name: String,
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        profile: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum BackupCommand {
    /// Back up a configuration file (defaults to the registry).
    Create {
        /// File to back up.
        #[arg(long)]
        file: Option<PathBuf>,
        /// Label stored with the backup.
        #[arg(long)]
        label: Option<String>,
    },
    /// List backups.
    List,
    /// Restore a backup by id (prefix ok).
    Restore { id: String },
}

#[derive(Debug, Subcommand)]
pub enum ProfileCommand {
    /// List profiles.
    List,
    /// Show a profile's servers.
    Show { name: String },
    /// Set the active profile.
    Use { name: String },
    /// Create a profile.
    Create {
        name: String,
        /// Servers to include (comma-separated or repeatable).
        #[arg(long = "server", value_name = "NAME")]
        servers: Vec<String>,
        #[arg(long)]
        description: Option<String>,
    },
    /// Validate a profile's server references.
    Validate { name: String },
    /// Delete a profile.
    Remove {
        name: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum GatewayCommand {
    /// Run the gateway in the foreground.
    Start {
        /// Transport for clients: stdio (default) or http.
        #[arg(long, default_value = "stdio")]
        transport: String,
        /// Bind address for the http transport (loopback only unless --allow-remote).
        #[arg(long, default_value = "127.0.0.1:8713")]
        bind: String,
        /// Explicitly allow binding beyond loopback (requires auth token).
        #[arg(long)]
        allow_remote: bool,
        /// Bearer token for http transport (or set CERBERUS_GATEWAY_TOKEN).
        #[arg(long, env = "CERBERUS_GATEWAY_TOKEN")]
        token: Option<String>,
        /// Restrict to one profile.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Show gateway status.
    Status,
    /// Diagnose gateway configuration and upstreams.
    Doctor,
    /// Validate gateway configuration without starting.
    Validate {
        #[arg(long)]
        profile: Option<String>,
    },
    /// Stop a running background gateway (if any).
    Stop,
}

pub async fn run(cli: Cli) -> Result<i32> {
    output::init_logging(cli.verbose, cli.debug);
    let ctx = output::Ctx {
        json: cli.json,
        quiet: cli.quiet,
        paths: crate::paths::CerberusPaths::resolve_with_override(cli.config)?,
    };
    crate::commands::dispatch(&ctx, cli.command).await
}
