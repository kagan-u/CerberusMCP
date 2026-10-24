//! Secret references and redaction.
//!
//! Secrets are never printed by CerberusMCP. Values flow from a pluggable
//! provider (environment-backed by default) to process environments at
//! launch time. Debug output for secret-bearing types is always redacted.

use std::collections::BTreeMap;
use std::fmt;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::error::{CerberusError, Result};
use crate::registry::schema::EnvValue;

/// A resolved secret. Debug/Display are redacted.
pub struct Secret(SecretString);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(SecretString::from(value.into()))
    }

    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl Clone for Secret {
    fn clone(&self) -> Self {
        Self(SecretString::from(self.expose().to_string()))
    }
}

/// Provider interface for resolving named secrets.
pub trait SecretProvider: Send + Sync {
    /// Resolve a secret by logical name, or `None` if unknown.
    fn resolve(&self, name: &str) -> Result<Option<Secret>>;
    fn describe(&self) -> String;
}

/// Default provider: resolves secrets from process environment variables.
///
/// Lookup order for logical name `github_token`:
/// 1. `CERBERUS_SECRET_GITHUB_TOKEN` (uppercased, `-` → `_`)
/// 2. the literal name uppercased (`GITHUB_TOKEN`)
pub struct EnvSecretProvider;

impl SecretProvider for EnvSecretProvider {
    fn resolve(&self, name: &str) -> Result<Option<Secret>> {
        let upper = name.to_uppercase().replace('-', "_");
        let prefixed = format!("CERBERUS_SECRET_{upper}");
        if let Ok(v) = std::env::var(&prefixed) {
            if !v.is_empty() {
                return Ok(Some(Secret::new(v)));
            }
        }
        if let Ok(v) = std::env::var(&upper) {
            if !v.is_empty() {
                return Ok(Some(Secret::new(v)));
            }
        }
        Ok(None)
    }

    fn describe(&self) -> String {
        "environment".to_string()
    }
}

/// A provider backed by an in-memory map (used in tests and by future
/// OS-keychain providers).
#[derive(Default)]
pub struct MapSecretProvider {
    map: BTreeMap<String, String>,
}

impl MapSecretProvider {
    pub fn new(map: BTreeMap<String, String>) -> Self {
        Self { map }
    }
}

impl SecretProvider for MapSecretProvider {
    fn resolve(&self, name: &str) -> Result<Option<Secret>> {
        Ok(self.map.get(name).cloned().map(Secret::new))
    }

    fn describe(&self) -> String {
        "map".to_string()
    }
}

/// Bundle of providers consulted in order.
pub struct SecretStore {
    providers: Vec<Box<dyn SecretProvider>>,
}

impl Default for SecretStore {
    fn default() -> Self {
        Self {
            providers: vec![Box::new(EnvSecretProvider)],
        }
    }
}

impl SecretStore {
    pub fn with_providers(providers: Vec<Box<dyn SecretProvider>>) -> Self {
        Self { providers }
    }

    pub fn resolve(&self, name: &str) -> Result<Secret> {
        for provider in &self.providers {
            if let Some(secret) = provider.resolve(name)? {
                return Ok(secret);
            }
        }
        Err(CerberusError::SecretResolution {
            name: name.to_string(),
            detail: format!(
                "not provided by any secret provider ({})",
                self.providers
                    .iter()
                    .map(|p| p.describe())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        })
    }

    /// Resolve an [`EnvValue`] into a concrete string for process launch.
    pub fn resolve_env_value(&self, value: &EnvValue) -> Result<String> {
        match value {
            EnvValue::Literal { value } => Ok(value.clone()),
            EnvValue::Env { name } => {
                std::env::var(name).map_err(|_| CerberusError::SecretResolution {
                    name: name.clone(),
                    detail: "environment variable is not set".into(),
                })
            }
            EnvValue::SecretRef { name } => Ok(self.resolve(name)?.expose().to_string()),
        }
    }

    /// Resolve every entry in an env map, returning concrete values.
    pub fn resolve_env_map(
        &self,
        map: &BTreeMap<String, EnvValue>,
    ) -> Result<BTreeMap<String, String>> {
        let mut out = BTreeMap::new();
        for (k, v) in map {
            out.insert(k.clone(), self.resolve_env_value(v)?);
        }
        Ok(out)
    }
}

/// Redact `value` if it appears inside `text`. Used before logging strings
/// that may embed resolved secrets (e.g. upstream error messages).
pub fn redact_secrets(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for secret in secrets {
        if secret.len() >= 4 && out.contains(secret.as_str()) {
            out = out.replace(secret.as_str(), "<redacted>");
        }
    }
    out
}

/// Collect the literal secret strings referenced by a registry, for redaction
/// of logs and diagnostics.
pub fn collect_literal_secrets(registry: &crate::registry::schema::Registry) -> Vec<String> {
    // We intentionally do NOT resolve secrets here; only literal values that
    // could otherwise leak through error messages are collected.
    let mut out = Vec::new();
    for server in registry.servers.values() {
        if let crate::registry::schema::Transport::Stdio { env, .. } = &server.transport {
            for v in env.values() {
                if let EnvValue::Literal { value } = v {
                    if !value.is_empty() {
                        out.push(value.clone());
                    }
                }
            }
        }
        if let crate::registry::schema::Transport::Http {
            headers,
            bearer_token,
            ..
        } = &server.transport
        {
            for h in headers.values() {
                if let crate::registry::schema::HeaderValue::Literal(s) = h {
                    if !s.is_empty() {
                        out.push(s.clone());
                    }
                }
            }
            if let Some(EnvValue::Literal { value }) = bearer_token {
                if !value.is_empty() {
                    out.push(value.clone());
                }
            }
        }
    }
    out
}

/// Serializable, redacted summary safe for JSON output and diagnostics.
#[derive(Debug, Serialize, Deserialize)]
pub struct RedactedEnvSummary {
    pub keys: Vec<String>,
    pub sources: Vec<String>,
}

pub fn summarize_env(map: &BTreeMap<String, EnvValue>) -> RedactedEnvSummary {
    let mut keys = Vec::new();
    let mut sources = Vec::new();
    for (k, v) in map {
        keys.push(k.clone());
        sources.push(v.redacted_display());
    }
    RedactedEnvSummary { keys, sources }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_is_redacted() {
        let s = Secret::new("hunter2");
        assert_eq!(format!("{s:?}"), "Secret(<redacted>)");
        assert_eq!(format!("{s}"), "<redacted>");
        assert_eq!(s.expose(), "hunter2");
    }

    #[test]
    fn map_provider_resolves() {
        let mut map = BTreeMap::new();
        map.insert("token".to_string(), "abc".to_string());
        let store = SecretStore::with_providers(vec![Box::new(MapSecretProvider::new(map))]);
        assert_eq!(store.resolve("token").unwrap().expose(), "abc");
        assert!(store.resolve("missing").is_err());
    }

    #[test]
    fn env_provider_prefers_prefixed_var() {
        unsafe { std::env::set_var("CERBERUS_SECRET_TEST_REDACT_X", "prefixed") };
        unsafe { std::env::set_var("TEST_REDACT_X", "plain") };
        let store = SecretStore::default();
        assert_eq!(store.resolve("test_redact_x").unwrap().expose(), "prefixed");
        unsafe { std::env::remove_var("CERBERUS_SECRET_TEST_REDACT_X") };
        unsafe { std::env::remove_var("TEST_REDACT_X") };
    }

    #[test]
    fn resolve_env_map_merges_literals_and_refs() {
        let map = BTreeMap::from([
            ("A".to_string(), EnvValue::literal("1")),
            ("B".to_string(), EnvValue::env("PATH")),
        ]);
        let store = SecretStore::default();
        let resolved = store.resolve_env_map(&map).unwrap();
        assert_eq!(resolved["A"], "1");
        assert!(resolved.contains_key("B"));
    }

    #[test]
    fn redact_secrets_replaces_known_values() {
        let out = redact_secrets(
            "token=super-secret-value here",
            &["super-secret-value".into()],
        );
        assert_eq!(out, "token=<redacted> here");
    }

    #[test]
    fn short_secrets_are_not_redacted() {
        // Avoid destroying ordinary text with tiny replacements.
        let out = redact_secrets("a bc", &["bc".into()]);
        assert_eq!(out, "a bc");
    }
}
