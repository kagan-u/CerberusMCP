use std::path::PathBuf;

use thiserror::Error;

pub type Result<T, E = CerberusError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum CerberusError {
    #[error("configuration not found at {path}; run `cerberus init` to create one")]
    ConfigNotFound { path: PathBuf },

    #[error("failed to parse configuration at {path}: {source}")]
    ConfigParse {
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("schema version {found} is not supported (supported: {supported})")]
    UnsupportedSchemaVersion { found: u32, supported: u32 },

    #[error("configuration validation failed:\n{0}")]
    Validation(String),

    #[error("server `{name}` already exists")]
    ServerExists { name: String },

    #[error("server `{name}` not found")]
    ServerNotFound { name: String },

    #[error("profile `{name}` already exists")]
    ProfileExists { name: String },

    #[error("profile `{name}` not found")]
    ProfileNotFound { name: String },

    #[error("client `{name}` is not supported: {reason}")]
    UnsupportedClient { name: String, reason: String },

    #[error("client `{name}` not found")]
    ClientNotFound { name: String },

    #[error("client `{name}` was not detected: {reason}")]
    ClientNotDetected { name: String, reason: String },

    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("synchronization conflict for client `{client}` server `{server}`: {detail}")]
    SyncConflict {
        client: String,
        server: String,
        detail: String,
    },

    #[error("synchronization failed for client `{client}`: {detail}")]
    SyncFailed { client: String, detail: String },

    #[error("backup `{id}` not found")]
    BackupNotFound { id: String },

    #[error("policy denied `{tool}` on server `{server}`: {reason}")]
    PolicyDenied {
        server: String,
        tool: String,
        reason: String,
    },

    #[error("secret `{name}` could not be resolved: {detail}")]
    SecretResolution { name: String, detail: String },

    #[error("failed to launch `{command}`: {source}")]
    ProcessSpawn {
        command: String,
        #[source]
        source: std::io::Error,
    },

    #[error("operation timed out after {timeout_ms} ms")]
    Timeout { timeout_ms: u64 },

    #[error("transport error: {0}")]
    Transport(String),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("upstream server `{server}` failed: {detail}")]
    Upstream { server: String, detail: String },

    #[error("unsupported capability: {0}")]
    Unsupported(String),

    #[error("gateway error: {0}")]
    Gateway(String),

    /// Generic "not found" for lookups that are not servers, profiles,
    /// clients, or backups (e.g. no registry record matches a name).
    #[error("{0}")]
    NotFound(String),

    /// The official MCP Registry could not be queried or returned garbage.
    #[error("registry request to {url} failed: {detail}")]
    RegistryRequest { url: String, detail: String },

    #[error("{0}")]
    Other(String),
}

impl From<serde_yaml::Error> for CerberusError {
    fn from(e: serde_yaml::Error) -> Self {
        CerberusError::Validation(format!("YAML error: {e}"))
    }
}

impl From<serde_json::Error> for CerberusError {
    fn from(e: serde_json::Error) -> Self {
        CerberusError::Validation(format!("JSON error: {e}"))
    }
}

/// Process exit codes. Kept stable for scripting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ExitCode {
    Success = 0,
    Failure = 1,
    Usage = 2,
    ConfigInvalid = 3,
    Conflict = 4,
    NotFound = 5,
    PolicyDenied = 6,
    Timeout = 7,
    Unsupported = 8,
}

impl CerberusError {
    pub fn exit_code(&self) -> ExitCode {
        use CerberusError::*;
        match self {
            ConfigNotFound { .. }
            | ServerNotFound { .. }
            | ProfileNotFound { .. }
            | ClientNotFound { .. }
            | BackupNotFound { .. }
            | NotFound(_) => ExitCode::NotFound,
            ConfigParse { .. } | Validation(_) | UnsupportedSchemaVersion { .. } => {
                ExitCode::ConfigInvalid
            }
            ServerExists { .. } | ProfileExists { .. } | SyncConflict { .. } => ExitCode::Conflict,
            PolicyDenied { .. } => ExitCode::PolicyDenied,
            Timeout { .. } => ExitCode::Timeout,
            UnsupportedClient { .. } | Unsupported(_) => ExitCode::Unsupported,
            _ => ExitCode::Failure,
        }
    }
}
