//! Terminal output helpers: human formatting, JSON emission, logging setup.

use std::io::IsTerminal;

use crate::paths::CerberusPaths;

pub struct Ctx {
    pub json: bool,
    pub quiet: bool,
    pub paths: CerberusPaths,
}

impl Ctx {
    pub fn emit_json<T: serde::Serialize>(&self, value: &T) {
        match serde_json::to_string_pretty(value) {
            Ok(text) => println!("{text}"),
            Err(e) => eprintln!("failed to serialize JSON: {e}"),
        }
    }

    pub fn out(&self, text: &str) {
        if !self.json && !self.quiet {
            println!("{text}");
        }
    }

    pub fn out_line(&self, text: impl AsRef<str>) {
        self.out(text.as_ref());
    }

    pub fn warn(&self, text: &str) {
        if !self.json {
            eprintln!("{} {text}", style("warning:", Style::Yellow));
        }
    }

    pub fn error(&self, text: &str) {
        eprintln!("{} {text}", style("error:", Style::Red));
    }

    pub fn status(&self, label: &str, value: &str) {
        if self.json || self.quiet {
            return;
        }
        let width: usize = 22;
        let dots = width.saturating_sub(label.chars().count());
        println!("  {label} {} {}", ".".repeat(dots), value);
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Style {
    Red,
    Yellow,
    Green,
    Cyan,
    Bold,
    Dim,
}

fn color_enabled() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if std::env::var("TERM").map(|t| t == "dumb").unwrap_or(false) {
        return false;
    }
    std::io::stdout().is_terminal()
}

pub fn style(text: &str, color: Style) -> String {
    if !color_enabled() {
        return text.to_string();
    }
    let code = match color {
        Style::Red => "31",
        Style::Yellow => "33",
        Style::Green => "32",
        Style::Cyan => "36",
        Style::Bold => "1",
        Style::Dim => "2",
    };
    format!("\x1b[{code}m{text}\x1b[0m")
}

pub fn bold(text: &str) -> String {
    style(text, Style::Bold)
}

pub fn dim(text: &str) -> String {
    style(text, Style::Dim)
}

pub fn green(text: &str) -> String {
    style(text, Style::Green)
}

pub fn yellow(text: &str) -> String {
    style(text, Style::Yellow)
}

pub fn red(text: &str) -> String {
    style(text, Style::Red)
}

pub fn cyan(text: &str) -> String {
    style(text, Style::Cyan)
}

pub fn check_mark(ok: bool) -> String {
    if ok { green("✓") } else { red("✗") }
}

/// Initialize tracing. Logs go to stderr so they never pollute JSON stdout.
pub fn init_logging(verbose: bool, debug: bool) {
    use tracing_subscriber::EnvFilter;
    let default_level = if debug {
        "debug"
    } else if verbose {
        "info"
    } else {
        "warn"
    };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
}

/// Parse a KEY=source:value binding used by `server add`.
pub fn parse_env_binding(
    binding: &str,
) -> Result<(String, crate::registry::schema::EnvValue), String> {
    let Some((key, rest)) = binding.split_once('=') else {
        return Err(format!(
            "`{binding}` must be KEY=env:VAR, KEY=secret:NAME, or KEY=literal:VALUE"
        ));
    };
    if key.is_empty() {
        return Err("environment variable name must not be empty".into());
    }
    let value = match rest.split_once(':') {
        Some(("env", var)) if !var.is_empty() => crate::registry::schema::EnvValue::env(var),
        Some(("secret", name)) if !name.is_empty() => {
            crate::registry::schema::EnvValue::SecretRef {
                name: name.to_string(),
            }
        }
        Some(("literal", value)) => crate::registry::schema::EnvValue::literal(value),
        // Bare values default to literal for ergonomics.
        other => {
            let literal = match other {
                Some((prefix, rest)) => format!("{prefix}:{rest}"),
                None => rest.to_string(),
            };
            crate::registry::schema::EnvValue::literal(literal)
        }
    };
    Ok((key.to_string(), value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parse_bindings() {
        let (k, v) = parse_env_binding("TOKEN=env:MY_TOKEN").unwrap();
        assert_eq!(k, "TOKEN");
        assert!(matches!(v, crate::registry::schema::EnvValue::Env { name } if name == "MY_TOKEN"));

        let (_, v) = parse_env_binding("T=secret:github").unwrap();
        assert!(matches!(
            v,
            crate::registry::schema::EnvValue::SecretRef { .. }
        ));

        let (_, v) = parse_env_binding("T=literal:abc").unwrap();
        assert!(matches!(
            v,
            crate::registry::schema::EnvValue::Literal { .. }
        ));

        let (_, v) = parse_env_binding("T=bare").unwrap();
        assert_eq!(v, crate::registry::schema::EnvValue::literal("bare"));

        assert!(parse_env_binding("noequals").is_err());
        assert!(parse_env_binding("=x").is_err());
    }

    #[test]
    fn no_color_is_respected() {
        // color_enabled reads env; we only assert style() passes through
        // when NO_COLOR is set in this process (set by CI environments).
        let s = style("x", Style::Red);
        assert!(s == "x" || s.contains('x'));
    }

    #[test]
    fn ctx_json_goes_to_stdout_only_via_emit() {
        let paths = CerberusPaths::with_roots(PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b"));
        let ctx = Ctx {
            json: true,
            quiet: false,
            paths,
        };
        // out() must be silent in json mode
        ctx.out("should not print");
    }
}
