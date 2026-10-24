//! Access policy evaluation.
//!
//! Policies are enforced by the gateway before any upstream request is
//! forwarded. Evaluation is deterministic and independent of what any client
//! claims about a tool.

use serde::{Deserialize, Serialize};

use crate::error::{CerberusError, Result};
use crate::registry::schema::ServerPolicy;

/// Lightweight view of a policy used by the routing table (avoids coupling
/// routing to the full registry types).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerPolicyView {
    pub allow_tools: Vec<String>,
    pub deny_tools: Vec<String>,
}

impl From<&ServerPolicy> for ServerPolicyView {
    fn from(p: &ServerPolicy) -> Self {
        Self {
            allow_tools: p.allow_tools.clone(),
            deny_tools: p.deny_tools.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Deny { reason: String },
}

impl Decision {
    pub fn is_allow(&self) -> bool {
        matches!(self, Decision::Allow)
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Decision::Allow => None,
            Decision::Deny { reason } => Some(reason),
        }
    }
}

/// Anything that exposes allow/deny lists can be evaluated.
pub trait PolicySpec {
    fn allow_list(&self) -> &[String];
    fn deny_list(&self) -> &[String];
}

impl PolicySpec for ServerPolicy {
    fn allow_list(&self) -> &[String] {
        &self.allow_tools
    }
    fn deny_list(&self) -> &[String] {
        &self.deny_tools
    }
}

impl PolicySpec for ServerPolicyView {
    fn allow_list(&self) -> &[String] {
        &self.allow_tools
    }
    fn deny_list(&self) -> &[String] {
        &self.deny_tools
    }
}

/// Evaluate whether `tool` may be invoked under `policy`.
///
/// Rules (in order):
/// 1. `deny_tools` wins unconditionally.
/// 2. If `allow_tools` is non-empty, the tool must match at least one entry
///    (default-deny).
/// 3. Otherwise the tool is allowed.
///
/// Pattern syntax: exact names, or a single trailing `*` wildcard
/// (e.g. `read_*`). Wildcards are validated at configuration time.
pub fn evaluate<P: PolicySpec + ?Sized>(policy: &P, tool: &str) -> Decision {
    for pattern in policy.deny_list() {
        if pattern_matches(pattern, tool) {
            return Decision::Deny {
                reason: format!("tool `{tool}` matches deny pattern `{pattern}`"),
            };
        }
    }
    if !policy.allow_list().is_empty() {
        let allowed = policy.allow_list().iter().any(|p| pattern_matches(p, tool));
        if !allowed {
            return Decision::Deny {
                reason: format!(
                    "tool `{tool}` is not in the allow list for this server (default-deny)"
                ),
            };
        }
    }
    Decision::Allow
}

/// Filter a tool list down to what policy permits. Used for `tools/list`.
pub fn filter_tools<'a, I>(policy: &ServerPolicy, tools: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a str>,
{
    tools
        .into_iter()
        .filter(|t| evaluate(policy, t).is_allow())
        .map(str::to_string)
        .collect()
}

/// Enforce a decision, converting denials into typed errors.
pub fn enforce(server: &str, tool: &str, policy: &ServerPolicy) -> Result<()> {
    match evaluate(policy, tool) {
        Decision::Allow => Ok(()),
        Decision::Deny { reason } => Err(CerberusError::PolicyDenied {
            server: server.to_string(),
            tool: tool.to_string(),
            reason,
        }),
    }
}

fn pattern_matches(pattern: &str, name: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        name.starts_with(prefix)
    } else {
        pattern == name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(allow: &[&str], deny: &[&str]) -> ServerPolicy {
        ServerPolicy {
            allow_tools: allow.iter().map(|s| s.to_string()).collect(),
            deny_tools: deny.iter().map(|s| s.to_string()).collect(),
            require_confirmation: false,
        }
    }

    #[test]
    fn empty_policy_allows_all() {
        assert!(evaluate(&policy(&[], &[]), "anything").is_allow());
    }

    #[test]
    fn deny_wins_over_allow() {
        let p = policy(&["read_*"], &["read_secret"]);
        let d = evaluate(&p, "read_secret");
        assert!(!d.is_allow());
        assert!(d.reason().unwrap().contains("deny pattern"));
        assert!(evaluate(&p, "read_file").is_allow());
    }

    #[test]
    fn allow_list_is_default_deny() {
        let p = policy(&["search_*"], &[]);
        assert!(evaluate(&p, "search_code").is_allow());
        assert!(!evaluate(&p, "delete_everything").is_allow());
    }

    #[test]
    fn wildcard_prefix_match() {
        assert!(pattern_matches("read_*", "read_file"));
        assert!(!pattern_matches("read_*", "write_file"));
        assert!(pattern_matches("exact", "exact"));
    }

    #[test]
    fn filter_tools_respects_policy() {
        let p = policy(&[], &["dangerous"]);
        let kept = filter_tools(&p, ["a", "dangerous", "b"]);
        assert_eq!(kept, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn enforce_returns_typed_error() {
        let p = policy(&[], &["x"]);
        let err = enforce("srv", "x", &p).unwrap_err();
        assert!(matches!(err, CerberusError::PolicyDenied { .. }));
    }
}
