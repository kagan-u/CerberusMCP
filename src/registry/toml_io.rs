//! TOML registry I/O for `conf.cerberus` (schema v2).
//!
//! The TOML document is the human-edited source of truth. Parsing routes
//! through [`crate::registry::parse_registry`] so the migration chain and
//! validation behave identically to legacy YAML documents.

use std::path::Path;

use crate::error::{CerberusError, Result};
use crate::registry::schema::{Registry, SCHEMA_VERSION};

/// On-disk format of a registry document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryFormat {
    Toml,
    Yaml,
}

/// Format implied by a file's extension (used for writing).
pub fn format_for_path(path: &Path) -> RegistryFormat {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("yaml") | Some("yml") => RegistryFormat::Yaml,
        _ => RegistryFormat::Toml,
    }
}

/// Detect the format of `text`. Content markers win over the extension so
/// restored backups (whose names may no longer match their content) still
/// parse; ambiguous input falls back to the extension.
pub fn sniff_format(text: &str, path: &Path) -> RegistryFormat {
    let has_yaml_marker = text.contains("schema_version:");
    let has_toml_marker = text.contains("schema_version =");
    match (has_yaml_marker, has_toml_marker) {
        (true, false) => RegistryFormat::Yaml,
        (false, true) => RegistryFormat::Toml,
        _ => format_for_path(path),
    }
}

/// Parse a TOML registry document, rejecting version mismatches with an
/// actionable message instead of a serde field error.
pub fn parse_toml_registry(text: &str, path: &Path) -> Result<Registry> {
    let registry: Registry = toml::from_str(text).map_err(|e| CerberusError::ConfigParse {
        path: path.to_path_buf(),
        source: Box::new(e),
    })?;
    if registry.schema_version > SCHEMA_VERSION {
        return Err(CerberusError::UnsupportedSchemaVersion {
            found: registry.schema_version,
            supported: SCHEMA_VERSION,
        });
    }
    if registry.schema_version < SCHEMA_VERSION {
        return Err(CerberusError::Validation(format!(
            "schema_version = {}: TOML documents require schema_version {SCHEMA_VERSION}; \
             legacy YAML documents use `schema_version: 1` and migrate on load",
            registry.schema_version
        )));
    }
    Ok(registry)
}

/// Render a registry as the TOML document stored in `conf.cerberus`.
pub fn registry_to_toml(registry: &Registry) -> Result<String> {
    toml::to_string(registry)
        .map_err(|e| CerberusError::Validation(format!("failed to serialize registry: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::schema::{ServerDef, ServerPolicy, Transport};
    use std::collections::BTreeMap;

    fn sample() -> Registry {
        let mut reg = Registry::default();
        reg.servers.insert(
            "echo".into(),
            ServerDef {
                enabled: true,
                transport: Transport::Stdio {
                    command: "npx".into(),
                    args: vec!["-y".into(), "pkg".into()],
                    env: BTreeMap::new(),
                    cwd: Some("~/src".into()),
                },
                mode: None,
                tags: vec!["local".into()],
                timeout_ms: Some(5000),
                startup_timeout_ms: None,
                policy: ServerPolicy::default(),
                description: Some("Echo server".into()),
                pre_launch: vec![],
                source: None,
            },
        );
        reg
    }

    #[test]
    fn toml_roundtrip_preserves_registry() {
        let reg = sample();
        let text = registry_to_toml(&reg).unwrap();
        assert!(text.contains("schema_version = 2"));
        let back = parse_toml_registry(&text, Path::new("conf.cerberus")).unwrap();
        assert_eq!(reg, back);
    }

    #[test]
    fn parses_full_featured_document() {
        let text = r#"
schema_version = 2
active_profile = "dev"

[commands.bootstrap]
run = ["sh", "-c", "echo ready"]
description = "warm the cache"

[servers.a]
pre_launch = [{ use = "bootstrap" }]

[servers.a.transport]
type = "stdio"
command = "npx"

[profiles.dev]
servers = ["a"]
"#;
        let reg = parse_toml_registry(text, Path::new("conf.cerberus")).unwrap();
        assert_eq!(reg.schema_version, SCHEMA_VERSION);
        assert_eq!(reg.commands["bootstrap"].run[0], "sh");
        assert_eq!(
            reg.servers["a"].pre_launch[0].use_command.as_deref(),
            Some("bootstrap")
        );
    }

    #[test]
    fn rejects_future_schema_version() {
        let err =
            parse_toml_registry("schema_version = 99\n", Path::new("conf.cerberus")).unwrap_err();
        assert!(matches!(
            err,
            CerberusError::UnsupportedSchemaVersion { found: 99, .. }
        ));
    }

    #[test]
    fn rejects_legacy_version_in_toml() {
        let err =
            parse_toml_registry("schema_version = 1\n", Path::new("conf.cerberus")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("schema_version"), "{msg}");
    }

    #[test]
    fn sniff_prefers_content_over_extension() {
        // YAML content stored under a TOML-looking name (restored backup).
        assert_eq!(
            sniff_format("schema_version: 1\n", Path::new("conf.cerberus")),
            RegistryFormat::Yaml
        );
        assert_eq!(
            sniff_format("schema_version = 2\n", Path::new("conf.cerberus")),
            RegistryFormat::Toml
        );
        // No marker: extension decides.
        assert_eq!(
            sniff_format("nothing here\n", Path::new("x.yaml")),
            RegistryFormat::Yaml
        );
        assert_eq!(
            sniff_format("nothing here\n", Path::new("x.cerberus")),
            RegistryFormat::Toml
        );
    }

    #[test]
    fn format_follows_extension_for_writes() {
        assert_eq!(
            format_for_path(Path::new("a/b/cerberus.yaml")),
            RegistryFormat::Yaml
        );
        assert_eq!(
            format_for_path(Path::new("a/b/conf.cerberus")),
            RegistryFormat::Toml
        );
    }
}
