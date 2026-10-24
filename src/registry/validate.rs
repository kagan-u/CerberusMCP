//! Strict registry validation with actionable error locations.

use std::collections::BTreeSet;

use crate::error::{CerberusError, Result};
use crate::registry::schema::{EnvValue, HeaderValue, Registry, ServerDef, Transport};

const MAX_NAME_LEN: usize = 64;

/// Validate an entire registry. Collects all problems and reports them
/// together so users fix batches instead of playing whack-a-mole.
pub fn validate(registry: &Registry) -> Result<()> {
    let mut errors = Vec::new();

    if registry.schema_version != super::schema::SCHEMA_VERSION {
        errors.push(format!(
            "schema_version: unsupported version {} (this build supports {})",
            registry.schema_version,
            super::schema::SCHEMA_VERSION
        ));
    }

    if let Some(level) = &registry.settings.log_level {
        let allowed = ["error", "warn", "info", "debug", "trace"];
        if !allowed.contains(&level.as_str()) {
            errors.push(format!(
                "settings.log_level: unknown level `{level}` (expected one of {})",
                allowed.join(", ")
            ));
        }
    }

    for (name, server) in &registry.servers {
        if let Err(e) = validate_server_name(name) {
            errors.extend(e.to_string().lines().map(str::to_string));
        }
        if let Err(e) = validate_server_def(name, server) {
            errors.push(e.to_string());
        }
    }

    for (pname, profile) in &registry.profiles {
        if let Err(e) = validate_profile_name(pname) {
            errors.extend(e.to_string().lines().map(str::to_string));
        }
        let mut seen = BTreeSet::new();
        for server in &profile.servers {
            if !seen.insert(server.as_str()) {
                errors.push(format!("profiles.{pname}: duplicate server `{server}`"));
            }
            if !registry.servers.contains_key(server) {
                errors.push(format!(
                    "profiles.{pname}: references unknown server `{server}`"
                ));
            }
        }
    }

    if let Some(active) = &registry.active_profile {
        if !registry.profiles.contains_key(active) {
            errors.push(format!(
                "active_profile: references unknown profile `{active}`"
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(CerberusError::Validation(errors.join("\n")))
    }
}

pub fn validate_server_name(name: &str) -> Result<()> {
    let mut errors = Vec::new();
    if name.is_empty() {
        errors.push("name must not be empty".to_string());
    }
    if name.len() > MAX_NAME_LEN {
        errors.push(format!("name must be at most {MAX_NAME_LEN} characters"));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        errors.push("name may only contain lowercase ASCII letters, digits, and `-`".to_string());
    }
    if name.starts_with('-') {
        errors.push("name must start with a letter or digit".to_string());
    }
    // Names containing the gateway's tool separator would be ambiguous.
    if name.contains("__") {
        errors.push(
            "name must not contain `__` (reserved as the gateway tool separator)".to_string(),
        );
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(CerberusError::Validation(format!(
            "server `{name}`: {}",
            errors.join("; ")
        )))
    }
}

pub fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(CerberusError::Validation(format!(
            "profile `{name}`: may only contain ASCII letters, digits, `-`, and `_`"
        )));
    }
    Ok(())
}

pub fn validate_server_def(name: &str, def: &ServerDef) -> Result<()> {
    match &def.transport {
        Transport::Stdio {
            command,
            args,
            env,
            cwd,
        } => {
            if command.trim().is_empty() {
                return Err(CerberusError::Validation(format!(
                    "servers.{name}.transport.command: must not be empty"
                )));
            }
            if command.contains('\0') {
                return Err(CerberusError::Validation(format!(
                    "servers.{name}.transport.command: must not contain NUL"
                )));
            }
            for (i, arg) in args.iter().enumerate() {
                if arg.contains('\0') {
                    return Err(CerberusError::Validation(format!(
                        "servers.{name}.transport.args[{i}]: must not contain NUL"
                    )));
                }
            }
            validate_env_map(name, env)?;
            if let Some(cwd) = cwd {
                if cwd.as_os_str().is_empty() {
                    return Err(CerberusError::Validation(format!(
                        "servers.{name}.transport.cwd: must not be empty"
                    )));
                }
            }
        }
        Transport::Http {
            url,
            headers,
            bearer_token,
        } => {
            let parsed = url::Url::parse(url).map_err(|e| {
                CerberusError::Validation(format!("servers.{name}.transport.url: invalid URL: {e}"))
            })?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err(CerberusError::Validation(format!(
                    "servers.{name}.transport.url: scheme must be http or https"
                )));
            }
            for (k, v) in headers {
                if k.trim().is_empty() {
                    return Err(CerberusError::Validation(format!(
                        "servers.{name}.transport.headers: empty header name"
                    )));
                }
                if let HeaderValue::Literal(s) = v {
                    if s.contains('\n') || s.contains('\r') {
                        return Err(CerberusError::Validation(format!(
                            "servers.{name}.transport.headers.{k}: header values must not contain newlines"
                        )));
                    }
                }
            }
            if let Some(EnvValue::Literal { value }) = bearer_token {
                if value.contains('\n') || value.contains('\r') {
                    return Err(CerberusError::Validation(format!(
                        "servers.{name}.transport.bearer_token: must not contain newlines"
                    )));
                }
            }
        }
    }

    if let Some(t) = def.timeout_ms {
        if t == 0 {
            return Err(CerberusError::Validation(format!(
                "servers.{name}.timeout_ms: must be greater than 0"
            )));
        }
    }
    if let Some(t) = def.startup_timeout_ms {
        if t == 0 {
            return Err(CerberusError::Validation(format!(
                "servers.{name}.startup_timeout_ms: must be greater than 0"
            )));
        }
    }

    for (i, pattern) in def.policy.allow_tools.iter().enumerate() {
        validate_tool_pattern(&format!("servers.{name}.policy.allow_tools[{i}]"), pattern)?;
    }
    for (i, pattern) in def.policy.deny_tools.iter().enumerate() {
        validate_tool_pattern(&format!("servers.{name}.policy.deny_tools[{i}]"), pattern)?;
    }

    Ok(())
}

fn validate_env_map(
    server: &str,
    env: &std::collections::BTreeMap<String, EnvValue>,
) -> Result<()> {
    for key in env.keys() {
        if key.is_empty() || key.contains('=') || key.contains('\0') {
            return Err(CerberusError::Validation(format!(
                "servers.{server}.transport.env: invalid variable name `{key}`"
            )));
        }
    }
    Ok(())
}

fn validate_tool_pattern(location: &str, pattern: &str) -> Result<()> {
    if pattern.is_empty() {
        return Err(CerberusError::Validation(format!(
            "{location}: empty pattern"
        )));
    }
    if pattern.contains('*') && !pattern.ends_with('*') {
        return Err(CerberusError::Validation(format!(
            "{location}: `{pattern}` — only a single trailing `*` wildcard is supported"
        )));
    }
    if pattern.matches('*').count() > 1 {
        return Err(CerberusError::Validation(format!(
            "{location}: `{pattern}` — only a single trailing `*` wildcard is supported"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::schema::*;
    use std::collections::BTreeMap;

    fn stdio_server(cmd: &str) -> ServerDef {
        ServerDef {
            enabled: true,
            transport: Transport::Stdio {
                command: cmd.into(),
                args: vec![],
                env: BTreeMap::new(),
                cwd: None,
            },
            mode: None,
            tags: vec![],
            timeout_ms: None,
            startup_timeout_ms: None,
            policy: ServerPolicy::default(),
            description: None,
        }
    }

    #[test]
    fn accepts_valid_registry() {
        let mut reg = Registry::default();
        reg.servers.insert("gh".into(), stdio_server("npx"));
        validate(&reg).unwrap();
    }

    #[test]
    fn rejects_reserved_separator_in_name() {
        assert!(validate_server_name("bad__name").is_err());
    }

    #[test]
    fn rejects_charset_outside_lowercase_hyphen_digit() {
        assert!(validate_server_name("Bad_Name").is_err());
        assert!(validate_server_name("has_underscore").is_err());
        assert!(validate_server_name("-leading").is_err());
        assert!(validate_server_name("ok-name-9").is_ok());
    }

    #[test]
    fn rejects_empty_command() {
        let mut reg = Registry::default();
        reg.servers.insert("s".into(), stdio_server("  "));
        assert!(validate(&reg).is_err());
    }

    #[test]
    fn rejects_bad_url_scheme() {
        let mut reg = Registry::default();
        reg.servers.insert(
            "s".into(),
            ServerDef {
                enabled: true,
                transport: Transport::Http {
                    url: "ftp://example.com".into(),
                    headers: BTreeMap::new(),
                    bearer_token: None,
                },
                mode: None,
                tags: vec![],
                timeout_ms: None,
                startup_timeout_ms: None,
                policy: ServerPolicy::default(),
                description: None,
            },
        );
        let err = validate(&reg).unwrap_err().to_string();
        assert!(err.contains("scheme must be http or https"));
    }

    #[test]
    fn rejects_midstring_wildcard() {
        let mut reg = Registry::default();
        let mut s = stdio_server("npx");
        s.policy.allow_tools = vec!["a*b".into()];
        reg.servers.insert("s".into(), s);
        assert!(validate(&reg).is_err());
    }

    #[test]
    fn rejects_unknown_profile_reference() {
        let mut reg = Registry::default();
        reg.servers.insert("a".into(), stdio_server("npx"));
        reg.profiles.insert(
            "p".into(),
            Profile {
                servers: vec!["missing".into()],
                description: None,
            },
        );
        assert!(validate(&reg).is_err());
    }

    #[test]
    fn rejects_nul_in_command() {
        let mut reg = Registry::default();
        reg.servers.insert("s".into(), stdio_server("ec\0ho"));
        assert!(validate(&reg).is_err());
    }

    #[test]
    fn rejects_header_injection() {
        let mut reg = Registry::default();
        reg.servers.insert(
            "s".into(),
            ServerDef {
                enabled: true,
                transport: Transport::Http {
                    url: "https://example.com".into(),
                    headers: [(
                        "X-Foo".to_string(),
                        HeaderValue::Literal("a\r\nInjected: 1".into()),
                    )]
                    .into_iter()
                    .collect(),
                    bearer_token: None,
                },
                mode: None,
                tags: vec![],
                timeout_ms: None,
                startup_timeout_ms: None,
                policy: ServerPolicy::default(),
                description: None,
            },
        );
        assert!(validate(&reg).is_err());
    }
}
