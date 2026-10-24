//! Tool routing for the gateway.
//!
//! Public tool names use `{server}__{tool}` (double underscore). Server names
//! may not contain `__` (enforced by registry validation), so splitting on
//! the first `__` is unambiguous. The mapping from public names to upstream
//! (server, tool) pairs is explicit and checked for collisions.

use std::collections::HashMap;

use serde::Serialize;

use crate::policy;

pub const SEPARATOR: &str = "__";

/// One routed tool.
#[derive(Debug, Clone, Serialize)]
pub struct Route {
    pub public_name: String,
    pub server: String,
    pub tool: String,
}

/// The routing table: public name → upstream target.
#[derive(Debug, Default, Clone)]
pub struct RoutingTable {
    routes: HashMap<String, Route>,
}

impl RoutingTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a tool. Returns an error when the public name collides with a
    /// different upstream target (same target is idempotent).
    pub fn insert(&mut self, server: &str, tool: &str) -> Result<(), String> {
        let public_name = public_tool_name(server, tool);
        match self.routes.get(&public_name) {
            Some(existing) if existing.server == server && existing.tool == tool => Ok(()),
            Some(existing) => Err(format!(
                "tool name collision: `{public_name}` maps to both \
                 `{}__{}` and `{server}__{tool}`",
                existing.server, existing.tool
            )),
            None => {
                self.routes.insert(
                    public_name.clone(),
                    Route {
                        public_name,
                        server: server.to_string(),
                        tool: tool.to_string(),
                    },
                );
                Ok(())
            }
        }
    }

    pub fn resolve(&self, public_name: &str) -> Option<&Route> {
        self.routes.get(public_name)
    }

    /// All routes whose upstream tool is permitted by the server's policy.
    pub fn permitted(&self, policies: &HashMap<String, policy::ServerPolicyView>) -> Vec<Route> {
        let mut out: Vec<Route> = self
            .routes
            .values()
            .filter(|r| {
                policies
                    .get(&r.server)
                    .map(|p| policy::evaluate(p, &r.tool).is_allow())
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| a.public_name.cmp(&b.public_name));
        out
    }

    pub fn len(&self) -> usize {
        self.routes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    pub fn all(&self) -> Vec<Route> {
        let mut out: Vec<Route> = self.routes.values().cloned().collect();
        out.sort_by(|a, b| a.public_name.cmp(&b.public_name));
        out
    }
}

/// Build the public name for a tool.
pub fn public_tool_name(server: &str, tool: &str) -> String {
    format!("{server}{SEPARATOR}{tool}")
}

/// Split a public name into (server, tool).
///
/// Splits on the FIRST `__`. Server names cannot contain `__` (validated at
/// registration), so the prefix is always the server name.
pub fn split_public_name(public: &str) -> Option<(&str, &str)> {
    let (server, rest) = public.split_once(SEPARATOR)?;
    if server.is_empty() || rest.is_empty() {
        return None;
    }
    Some((server, rest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::ServerPolicyView;

    #[test]
    fn round_trip_names() {
        assert_eq!(public_tool_name("github", "search"), "github__search");
        let (s, t) = split_public_name("github__search").unwrap();
        assert_eq!((s, t), ("github", "search"));
    }

    #[test]
    fn splits_on_first_separator_only() {
        // Upstream tool names may themselves contain `__`.
        let (s, t) = split_public_name("srv__weird__tool").unwrap();
        assert_eq!(s, "srv");
        assert_eq!(t, "weird__tool");
    }

    #[test]
    fn rejects_invalid_public_names() {
        assert!(split_public_name("noseparator").is_none());
        assert!(split_public_name("__tool").is_none());
        assert!(split_public_name("server__").is_none());
    }

    #[test]
    fn collision_detection() {
        let mut table = RoutingTable::new();
        table.insert("a", "t").unwrap();
        // Identical insert is fine.
        table.insert("a", "t").unwrap();
        // Server names containing `__` are rejected by validation, but the
        // table must still detect the ambiguity defensively: `a` + tool `b__c`
        // and server `a__b` + tool `c` would both publish `a__b__c`.
        table.insert("a", "b__c").unwrap();
        let err = table.insert("a__b", "c").unwrap_err();
        assert!(err.contains("collision"));
    }

    #[test]
    fn policy_filters_routes() {
        let mut table = RoutingTable::new();
        table.insert("srv", "safe").unwrap();
        table.insert("srv", "danger").unwrap();
        let policies = HashMap::from([(
            "srv".to_string(),
            ServerPolicyView {
                allow_tools: vec![],
                deny_tools: vec!["danger".to_string()],
            },
        )]);
        let permitted = table.permitted(&policies);
        assert_eq!(permitted.len(), 1);
        assert_eq!(permitted[0].tool, "safe");
    }
}
