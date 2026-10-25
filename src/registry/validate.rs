//! Strict registry validation with actionable error locations.

use std::collections::BTreeSet;

use crate::error::{CerberusError, Result};
use crate::registry::schema::{
    CommandDef, EnvValue, HeaderValue, HookStep, Registry, ServerDef, SourceDef, SourceKind,
    Transport,
};

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
        for (i, step) in server.pre_launch.iter().enumerate() {
            if let Some(r) = &step.use_command {
                if !registry.commands.contains_key(r) {
                    errors.push(format!(
                        "servers.{name}.pre_launch[{i}].use: unknown command `{r}` \
                         (define it under [commands.{r}])"
                    ));
                }
            }
        }
    }

    for (name, command) in &registry.commands {
        if let Err(e) = validate_command_name(name) {
            errors.push(e.to_string());
        }
        if let Err(e) = validate_command(name, command) {
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
            validate_env_map_at(&format!("servers.{name}.transport.env"), env)?;
            if let Some(cwd) = cwd {
                if cwd.is_empty() {
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

    if !def.pre_launch.is_empty() {
        if !matches!(def.transport, Transport::Stdio { .. }) {
            return Err(CerberusError::Validation(format!(
                "servers.{name}.pre_launch: only valid for stdio transports \
                 (http upstreams are not spawned by the gateway)"
            )));
        }
        for (i, step) in def.pre_launch.iter().enumerate() {
            validate_hook_step(&format!("servers.{name}.pre_launch[{i}]"), step)?;
        }
    }

    if let Some(source) = &def.source {
        validate_source(name, source)?;
    }

    Ok(())
}

/// Structural validation for one pre-launch step. Reference resolution
/// (`use` → `[commands.<name>]`) needs registry context and lives in
/// [`validate`].
fn validate_hook_step(location: &str, step: &HookStep) -> Result<()> {
    match (&step.use_command, &step.run) {
        (Some(_), Some(_)) => {
            return Err(CerberusError::Validation(format!(
                "{location}: set exactly one of `use` or `run`, not both"
            )));
        }
        (None, None) => {
            return Err(CerberusError::Validation(format!(
                "{location}: one of `use` or `run` is required"
            )));
        }
        (Some(name), None) => {
            if name.trim().is_empty() {
                return Err(CerberusError::Validation(format!(
                    "{location}.use: must not be empty"
                )));
            }
        }
        (None, Some(argv)) => {
            if argv.is_empty() {
                return Err(CerberusError::Validation(format!(
                    "{location}.run: must not be empty"
                )));
            }
            if argv[0].trim().is_empty() || argv[0].contains('\0') {
                return Err(CerberusError::Validation(format!(
                    "{location}.run[0]: executable must not be empty or contain NUL"
                )));
            }
            for (i, arg) in argv.iter().enumerate() {
                if arg.contains('\0') {
                    return Err(CerberusError::Validation(format!(
                        "{location}.run[{i}]: must not contain NUL"
                    )));
                }
            }
        }
    }
    if let Some(cwd) = &step.cwd {
        if cwd.is_empty() {
            return Err(CerberusError::Validation(format!(
                "{location}.cwd: must not be empty"
            )));
        }
    }
    if let Some(t) = step.timeout_ms {
        if t == 0 {
            return Err(CerberusError::Validation(format!(
                "{location}.timeout_ms: must be greater than 0"
            )));
        }
    }
    validate_env_map_at(&format!("{location}.env"), &step.env)
}

/// Validate a named `[commands.<name>]` entry.
pub fn validate_command(name: &str, command: &CommandDef) -> Result<()> {
    let base = format!("commands.{name}");
    if command.run.is_empty() {
        return Err(CerberusError::Validation(format!(
            "{base}.run: must not be empty"
        )));
    }
    if command.run[0].trim().is_empty() || command.run[0].contains('\0') {
        return Err(CerberusError::Validation(format!(
            "{base}.run[0]: executable must not be empty or contain NUL"
        )));
    }
    for (i, arg) in command.run.iter().enumerate() {
        if arg.contains('\0') {
            return Err(CerberusError::Validation(format!(
                "{base}.run[{i}]: must not contain NUL"
            )));
        }
    }
    if let Some(cwd) = &command.cwd {
        if cwd.is_empty() {
            return Err(CerberusError::Validation(format!(
                "{base}.cwd: must not be empty"
            )));
        }
    }
    if let Some(t) = command.timeout_ms {
        if t == 0 {
            return Err(CerberusError::Validation(format!(
                "{base}.timeout_ms: must be greater than 0"
            )));
        }
    }
    validate_env_map_at(&format!("{base}.env"), &command.env)
}

/// Command names share the profile charset (letters, digits, `-`, `_`).
pub fn validate_command_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(CerberusError::Validation(format!(
            "commands.{name}: may only contain ASCII letters, digits, `-`, and `_`"
        )));
    }
    Ok(())
}

/// Validate the install-provenance block. Requirements are per kind so a
/// half-filled source (e.g. a git URL without `url:`) is rejected at write
/// time rather than surfacing during a later `cerberus install --check`.
fn validate_source(server: &str, source: &SourceDef) -> Result<()> {
    let base = format!("servers.{server}.source");
    let package = source.package.as_deref().unwrap_or("");

    match source.kind {
        SourceKind::Npm => {
            require_field(&base, "package", package)?;
            if package.contains(char::is_whitespace) || package.contains('\0') {
                return Err(CerberusError::Validation(format!(
                    "{base}.package: `{package}` must not contain whitespace"
                )));
            }
            // npm allows scoped names of the shape `@scope/name`.
            let bare = match package.strip_prefix('@') {
                Some(rest) => {
                    let valid_scope = rest.split_once('/').is_some_and(|(scope, name)| {
                        !scope.is_empty()
                            && !name.is_empty()
                            && !name.contains('/')
                            && !name.starts_with('.')
                    });
                    if !valid_scope {
                        return Err(CerberusError::Validation(format!(
                            "{base}.package: `{package}` is not a valid npm package name"
                        )));
                    }
                    rest
                }
                None => {
                    if package.contains('/') {
                        return Err(CerberusError::Validation(format!(
                            "{base}.package: `{package}` is not a valid npm package name"
                        )));
                    }
                    package
                }
            };
            if bare.contains("//")
                || !bare
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-._/".contains(c))
            {
                return Err(CerberusError::Validation(format!(
                    "{base}.package: `{package}` is not a valid npm package name"
                )));
            }
        }
        SourceKind::Pypi => {
            require_field(&base, "package", package)?;
            let ok = !package.is_empty()
                && package
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
                && package
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric())
                && package
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_ascii_alphanumeric() && c != '-');
            if !ok {
                return Err(CerberusError::Validation(format!(
                    "{base}.package: `{package}` is not a valid PyPI package name"
                )));
            }
        }
        SourceKind::Git => {
            let url = source.url.as_deref().unwrap_or("");
            require_field(&base, "url", url)?;
            let scp_like = url.contains(':') && !url.contains("://");
            let parsed = url::Url::parse(url);
            let scheme_ok = matches!(
                parsed.as_ref().map(|u| u.scheme()),
                Ok("http") | Ok("https") | Ok("ssh") | Ok("git") | Ok("file")
            );
            if !scheme_ok && !scp_like {
                return Err(CerberusError::Validation(format!(
                    "{base}.url: `{url}` must be an http(s)/ssh/git/file URL \
                     or an scp-style `git@host:path`"
                )));
            }
            if let Some(r) = &source.git_ref {
                if r.trim().is_empty() {
                    return Err(CerberusError::Validation(format!(
                        "{base}.ref: must not be empty"
                    )));
                }
            }
        }
        SourceKind::Binary => {
            require_field(&base, "package", package)?;
            if package.contains('\0') {
                return Err(CerberusError::Validation(format!(
                    "{base}.package: must not contain NUL"
                )));
            }
        }
    }

    if let Some(version) = &source.version {
        if version.trim().is_empty() || version.chars().any(char::is_whitespace) {
            return Err(CerberusError::Validation(format!(
                "{base}.version: `{version}` must be a single token without whitespace"
            )));
        }
    }
    if let Some(runner) = &source.runner {
        if runner.trim().is_empty() {
            return Err(CerberusError::Validation(format!(
                "{base}.runner: must not be empty"
            )));
        }
        if runner.contains('\0') {
            return Err(CerberusError::Validation(format!(
                "{base}.runner: must not contain NUL"
            )));
        }
    }
    for (i, arg) in source.args.iter().enumerate() {
        if arg.contains('\0') {
            return Err(CerberusError::Validation(format!(
                "{base}.args[{i}]: must not contain NUL"
            )));
        }
    }
    if let Some(homepage) = &source.homepage {
        if url::Url::parse(homepage).is_err() {
            return Err(CerberusError::Validation(format!(
                "{base}.homepage: `{homepage}` is not a valid URL"
            )));
        }
    }

    let mut seen = BTreeSet::new();
    for (i, env) in source.env.iter().enumerate() {
        let loc = format!("{base}.env[{i}]");
        let valid = !env.name.is_empty()
            && env
                .name
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && env
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid {
            return Err(CerberusError::Validation(format!(
                "{loc}.name: `{}` is not a valid environment variable name",
                env.name
            )));
        }
        if !seen.insert(env.name.as_str()) {
            return Err(CerberusError::Validation(format!(
                "{base}.env: duplicate variable `{}`",
                env.name
            )));
        }
    }

    Ok(())
}

fn require_field(base: &str, field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(CerberusError::Validation(format!(
            "{base}.{field}: required for this source kind"
        )));
    }
    Ok(())
}

fn validate_env_map_at(
    location: &str,
    env: &std::collections::BTreeMap<String, EnvValue>,
) -> Result<()> {
    for key in env.keys() {
        if key.is_empty() || key.contains('=') || key.contains('\0') {
            return Err(CerberusError::Validation(format!(
                "{location}: invalid variable name `{key}`"
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
            source: None,
            pre_launch: vec![],
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
                source: None,
                pre_launch: vec![],
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
                source: None,
                pre_launch: vec![],
            },
        );
        assert!(validate(&reg).is_err());
    }

    fn server_with_source(source: SourceDef) -> ServerDef {
        let mut def = stdio_server("npx");
        def.source = Some(source);
        def
    }

    fn source(kind: SourceKind) -> SourceDef {
        SourceDef {
            kind,
            package: None,
            version: None,
            homepage: None,
            runner: None,
            args: vec![],
            url: None,
            git_ref: None,
            env: vec![],
        }
    }

    fn validate_source_of(source: SourceDef) -> Result<()> {
        validate_server_def("svc", &server_with_source(source))
    }

    #[test]
    fn accepts_well_formed_sources() {
        let mut npm = source(SourceKind::Npm);
        npm.package = Some("@modelcontextprotocol/server-filesystem".into());
        npm.version = Some("0.6.2".into());
        npm.runner = Some("npx".into());
        npm.args = vec!["-y".into()];
        npm.homepage = Some("https://github.com/modelcontextprotocol/servers".into());
        npm.env = vec![
            SourceEnv {
                name: "API_KEY".into(),
                required: true,
                secret: true,
                description: Some("key".into()),
            },
            SourceEnv {
                name: "OPTIONAL".into(),
                required: false,
                secret: false,
                description: None,
            },
        ];
        validate_source_of(npm).unwrap();

        let mut pypi = source(SourceKind::Pypi);
        pypi.package = Some("my_package-2".into());
        validate_source_of(pypi).unwrap();

        let mut git = source(SourceKind::Git);
        git.url = Some("git@github.com:org/repo.git".into());
        git.git_ref = Some("v1.2.3".into());
        validate_source_of(git).unwrap();

        let mut bin = source(SourceKind::Binary);
        bin.package = Some("/usr/local/bin/mcp-server".into());
        validate_source_of(bin).unwrap();
    }

    #[test]
    fn rejects_missing_package_for_npm() {
        let err = validate_source_of(source(SourceKind::Npm))
            .unwrap_err()
            .to_string();
        assert!(err.contains("package"), "{err}");
        assert!(err.contains("required"), "{err}");
    }

    #[test]
    fn rejects_malformed_package_names() {
        for bad in ["Has Space", "a//b", "@scope", "UPPER"] {
            let mut s = source(SourceKind::Npm);
            s.package = Some(bad.into());
            let err = validate_source_of(s).unwrap_err().to_string();
            // Whitespace and charset are rejected by different checks; both
            // must name the offending field.
            assert!(err.contains("source.package"), "{bad}: {err}");
        }

        for bad in ["my pkg", "trailing-", "-leading", ""] {
            let mut s = source(SourceKind::Pypi);
            s.package = Some(bad.into());
            assert!(validate_source_of(s).is_err(), "pypi should reject {bad:?}");
        }
    }

    #[test]
    fn rejects_bad_or_missing_git_url() {
        let err = validate_source_of(source(SourceKind::Git))
            .unwrap_err()
            .to_string();
        assert!(err.contains("url"), "{err}");

        let mut s = source(SourceKind::Git);
        s.url = Some("ftp://example.com/repo".into());
        let err = validate_source_of(s).unwrap_err().to_string();
        assert!(err.contains("http(s)/ssh/git/file"), "{err}");

        let mut ok = source(SourceKind::Git);
        ok.url = Some("https://github.com/org/repo".into());
        validate_source_of(ok).unwrap();
    }

    #[test]
    fn rejects_ambiguous_version_and_bad_env_names() {
        let mut s = source(SourceKind::Npm);
        s.package = Some("pkg".into());
        s.version = Some(">= 1, < 2".into());
        let err = validate_source_of(s).unwrap_err().to_string();
        assert!(err.contains("version"), "{err}");

        let mut s = source(SourceKind::Npm);
        s.package = Some("pkg".into());
        s.env = vec![SourceEnv {
            name: "1BAD-NAME".into(),
            required: true,
            secret: false,
            description: None,
        }];
        let err = validate_source_of(s).unwrap_err().to_string();
        assert!(err.contains("environment variable name"), "{err}");

        let mut s = source(SourceKind::Npm);
        s.package = Some("pkg".into());
        s.env = vec![
            SourceEnv {
                name: "SAME".into(),
                required: true,
                secret: false,
                description: None,
            },
            SourceEnv {
                name: "SAME".into(),
                required: false,
                secret: false,
                description: None,
            },
        ];
        let err = validate_source_of(s).unwrap_err().to_string();
        assert!(err.contains("duplicate variable"), "{err}");
    }

    #[test]
    fn rejects_invalid_homepage() {
        let mut s = source(SourceKind::Npm);
        s.package = Some("pkg".into());
        s.homepage = Some("not a url".into());
        let err = validate_source_of(s).unwrap_err().to_string();
        assert!(err.contains("homepage"), "{err}");
    }
}
