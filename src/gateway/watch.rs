//! `gateway start --watch`: opt-in registry hot reload.
//!
//! A background task polls the registry file's stat (mtime + size) every
//! [`WATCH_INTERVAL`]. When it changes, the file is re-parsed and
//! re-validated; only a valid document whose content hash differs from the
//! applied one takes effect, and a broken file leaves the running
//! configuration untouched (with a warning).
//!
//! Reconciliation is per server: only `transport`/`pre_launch` changes
//! cancel and reconnect an upstream (which re-runs its pre-launch hooks);
//! every other change (policy, tags, mode, ...) is swapped in place while
//! the live connection stays up. The `[commands.*]` table and the
//! redaction list always follow the new document.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::{GatewayHandler, GatewayState, Upstream};
use crate::policy::ServerPolicyView;
use crate::registry::schema::ServerDef;
use crate::registry::{RegistryStore, validate};
use crate::secrets::collect_literal_secrets;

/// Stat-poll interval. File-stat only (no inotify/FSEvents), so behavior is
/// identical on every platform.
pub(crate) const WATCH_INTERVAL: Duration = Duration::from_millis(500);

/// What one reload has to do to the running state.
#[derive(Debug, Default)]
pub(crate) struct ReconcilePlan {
    /// Selection no longer contains these servers: cancel and drop them.
    pub removed: Vec<String>,
    /// New members of the selection: register and connect them.
    pub added: Vec<String>,
    /// `transport` or `pre_launch` changed: cancel, swap the definition,
    /// reconnect (the reconnect re-runs pre-launch hooks).
    pub reconnect: Vec<String>,
    /// Anything else changed (policy, tags, mode, ...): swap in place,
    /// connection untouched.
    pub update: Vec<String>,
    pub unchanged: usize,
}

fn plan_reconcile(
    old: &BTreeMap<String, ServerDef>,
    new: &BTreeMap<String, ServerDef>,
) -> ReconcilePlan {
    let mut plan = ReconcilePlan::default();
    for name in old.keys() {
        if !new.contains_key(name) {
            plan.removed.push(name.clone());
        }
    }
    for (name, new_def) in new {
        match old.get(name) {
            None => plan.added.push(name.clone()),
            Some(old_def) if old_def == new_def => plan.unchanged += 1,
            Some(old_def)
                if old_def.transport != new_def.transport
                    || old_def.pre_launch != new_def.pre_launch =>
            {
                plan.reconnect.push(name.clone())
            }
            Some(_) => plan.update.push(name.clone()),
        }
    }
    plan
}

fn file_stat(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    Some((mtime, meta.len()))
}

/// Spawn the watcher task; it stops when `ct` is cancelled (gateway shutdown).
pub(crate) fn spawn(
    state: Arc<GatewayState>,
    profile: Option<String>,
    baseline_hash: u64,
    ct: CancellationToken,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let source = state.paths.registry_file.clone();
        let mut seen: Option<(u64, u64)> = None;
        let mut applied_hash = baseline_hash;
        loop {
            tokio::select! {
                _ = ct.cancelled() => break,
                _ = tokio::time::sleep(interval) => {}
            }
            let stat = file_stat(&source);
            if stat == seen {
                continue;
            }
            // Record the stat before parsing: a half-written or broken file
            // must not retrigger every tick until someone fixes it; the next
            // edit changes the stat again.
            seen = stat;
            let store = match RegistryStore::load_with_migration(&state.paths) {
                Ok(store) => store,
                Err(e) => {
                    tracing::warn!(error = %e, "registry reload rejected; keeping the running configuration");
                    continue;
                }
            };
            if store.content_hash == applied_hash {
                continue; // mtime-only touch, or identical content
            }
            if let Err(e) = validate::validate(&store.registry) {
                tracing::warn!(error = %e, "registry reload rejected; keeping the running configuration");
                continue;
            }
            let selected = match super::selected_for_gateway(&store, profile.as_deref()) {
                Ok(selected) => selected,
                Err(e) => {
                    tracing::warn!(error = %e, "registry reload rejected; keeping the running configuration");
                    continue;
                }
            };
            let old: BTreeMap<String, ServerDef> = {
                let upstreams = state.upstreams.lock().await;
                upstreams
                    .iter()
                    .map(|(name, u)| (name.clone(), u.def.clone()))
                    .collect()
            };
            let plan = plan_reconcile(&old, &selected);
            apply(state.clone(), &store, &selected, &plan).await;
            applied_hash = store.content_hash;
            tracing::info!(
                added = plan.added.len(),
                reconnected = plan.reconnect.len(),
                updated = plan.update.len(),
                removed = plan.removed.len(),
                unchanged = plan.unchanged,
                "registry reloaded"
            );
        }
    })
}

async fn apply(
    state: Arc<GatewayState>,
    store: &RegistryStore,
    selected: &BTreeMap<String, ServerDef>,
    plan: &ReconcilePlan,
) {
    // Structural changes under the upstreams lock. Connections are made
    // after it is released — `ensure_connected` takes it itself.
    {
        let mut upstreams = state.upstreams.lock().await;
        for name in &plan.removed {
            if let Some(mut upstream) = upstreams.remove(name) {
                if let Some(service) = upstream.service.take() {
                    let _ = service.cancel().await;
                }
            }
        }
        for name in &plan.reconnect {
            if let Some(upstream) = upstreams.get_mut(name) {
                if let Some(service) = upstream.service.take() {
                    let _ = service.cancel().await;
                }
                upstream.tools.clear();
                upstream.last_error = None;
                upstream.def = selected[name].clone();
            }
        }
        for name in &plan.update {
            if let Some(upstream) = upstreams.get_mut(name) {
                upstream.def = selected[name].clone();
            }
        }
        for name in &plan.added {
            upstreams.insert(
                name.clone(),
                Upstream {
                    server: name.clone(),
                    def: selected[name].clone(),
                    service: None,
                    tools: Vec::new(),
                    last_error: None,
                },
            );
        }
    }

    // Policies and confirmations follow the new selection wholesale.
    {
        let mut policies = state.policies.write().await;
        *policies = selected
            .iter()
            .map(|(name, def)| (name.clone(), ServerPolicyView::from(&def.policy)))
            .collect();
        let mut confirmations = state.confirmations.write().await;
        *confirmations = selected
            .iter()
            .map(|(name, def)| (name.clone(), def.policy.require_confirmation))
            .collect();
    }

    // The hook command table and the redaction list always follow the document.
    *state.commands.write().await = store.registry.commands.clone();
    *state.redact.write().unwrap_or_else(|e| e.into_inner()) =
        collect_literal_secrets(&store.registry);

    // (Re)connect only what the plan marked; failures stay isolated per
    // server exactly like at startup.
    let handler = GatewayHandler::new(state.clone());
    for name in plan.added.iter().chain(&plan.reconnect) {
        if let Err(e) = handler.ensure_connected(name).await {
            tracing::warn!(
                server = %name,
                error = %e,
                "upstream connect failed after registry reload"
            );
        }
    }
    handler.refresh_routes().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn def(value: serde_json::Value) -> ServerDef {
        serde_json::from_value(value).expect("valid server def")
    }

    fn stdio(command: &str) -> ServerDef {
        def(json!({"transport": {"type": "stdio", "command": command}}))
    }

    fn map(defs: Vec<(&str, ServerDef)>) -> BTreeMap<String, ServerDef> {
        defs.into_iter()
            .map(|(name, def)| (name.to_string(), def))
            .collect()
    }

    #[test]
    fn membership_changes_split_into_added_and_removed() {
        let old = map(vec![("stay", stdio("sh")), ("gone", stdio("sh"))]);
        let new = map(vec![("stay", stdio("sh")), ("fresh", stdio("sh"))]);
        let plan = plan_reconcile(&old, &new);
        assert_eq!(plan.added, vec!["fresh".to_string()]);
        assert_eq!(plan.removed, vec!["gone".to_string()]);
        assert!(plan.reconnect.is_empty());
        assert!(plan.update.is_empty());
        assert_eq!(plan.unchanged, 1);
    }

    #[test]
    fn transport_changes_reconnect() {
        let old = map(vec![("a", stdio("sh"))]);
        let new = map(vec![("a", stdio("bash"))]);
        let plan = plan_reconcile(&old, &new);
        assert_eq!(plan.reconnect, vec!["a".to_string()]);
        assert!(plan.update.is_empty());
    }

    #[test]
    fn pre_launch_changes_reconnect() {
        let old = map(vec![("a", stdio("sh"))]);
        let mut hooked = stdio("sh");
        hooked.pre_launch = vec![
            serde_json::from_value(json!({"run": ["sh", "-c", "true"]})).expect("valid hook step"),
        ];
        let new = map(vec![("a", hooked)]);
        let plan = plan_reconcile(&old, &new);
        assert_eq!(plan.reconnect, vec!["a".to_string()]);
    }

    #[test]
    fn non_spawn_changes_update_in_place() {
        let old = map(vec![("a", stdio("sh"))]);

        let mut policy = stdio("sh");
        policy.policy = serde_json::from_value(json!({"deny_tools": ["x"]})).expect("policy");
        let plan = plan_reconcile(&old, &map(vec![("a", policy)]));
        assert_eq!(plan.update, vec!["a".to_string()]);
        assert!(plan.reconnect.is_empty());

        let mut description = stdio("sh");
        description.description = Some("changed".into());
        let plan = plan_reconcile(&old, &map(vec![("a", description)]));
        assert_eq!(plan.update, vec!["a".to_string()]);

        let mut tags = stdio("sh");
        tags.tags = vec!["prod".into()];
        let plan = plan_reconcile(&old, &map(vec![("a", tags)]));
        assert_eq!(plan.update, vec!["a".to_string()]);
    }

    #[test]
    fn identical_defs_count_as_unchanged() {
        let old = map(vec![("a", stdio("sh"))]);
        let new = map(vec![("a", stdio("sh"))]);
        let plan = plan_reconcile(&old, &new);
        assert_eq!(plan.unchanged, 1);
        assert!(plan.added.is_empty());
        assert!(plan.removed.is_empty());
        assert!(plan.reconnect.is_empty());
        assert!(plan.update.is_empty());
    }
}
