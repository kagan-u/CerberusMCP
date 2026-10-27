import { useState } from "react";
import { api } from "../api";
import type { Registry, ServerDef } from "../types";
import ServerForm from "../components/ServerForm";

interface Props {
  registry: Registry;
  refresh: () => Promise<void>;
  onError: (message: string) => void;
}

export default function Servers({ registry, refresh, onError }: Props) {
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);

  const servers = Object.entries(registry.servers ?? {})
    .filter(([name]) => name.includes(query.trim()))
    .sort(([a], [b]) => a.localeCompare(b));

  const openName = creating || editing !== null;

  const toggle = async (name: string, enabled: boolean) => {
    try {
      await api.toggleServer(name, enabled);
      await refresh();
    } catch (e) {
      onError(String(e));
    }
  };

  const remove = async (name: string) => {
    if (!window.confirm(`Delete server “${name}” from the registry?`)) return;
    try {
      await api.deleteServer(name);
      await refresh();
    } catch (e) {
      onError(String(e));
    }
  };

  const submit = async (name: string, def: ServerDef): Promise<boolean> => {
    try {
      await api.upsertServer(name, def);
      await refresh();
      return true;
    } catch (e) {
      onError(String(e));
      return false;
    }
  };

  return (
    <div className="page">
      <header className="page-head">
        <h1>Servers</h1>
        <div className="head-actions">
          <input
            className="search"
            placeholder="filter by name"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          <button className="btn btn-primary" onClick={() => setCreating(true)}>
            + Add server
          </button>
        </div>
      </header>

      <section className="card">
        {servers.length === 0 ? (
          <p className="muted">No servers match.</p>
        ) : (
          <table className="table">
            <thead>
              <tr>
                <th>Enabled</th>
                <th>Name</th>
                <th>Transport</th>
                <th>Target</th>
                <th>Tags</th>
                <th>Description</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {servers.map(([name, s]) => (
                <tr key={name} className={s.enabled ? "" : "row-dim"}>
                  <td>
                    <label className="switch">
                      <input
                        type="checkbox"
                        checked={s.enabled}
                        onChange={(e) => void toggle(name, e.target.checked)}
                      />
                      <span className="slider" />
                    </label>
                  </td>
                  <td>
                    <code>{name}</code>
                  </td>
                  <td>
                    <span className="badge">{s.transport.type}</span>
                  </td>
                  <td className="muted small">
                    <code className="truncate">
                      {s.transport.type === "stdio"
                        ? s.transport.command
                        : s.transport.url}
                    </code>
                  </td>
                  <td className="muted small">{(s.tags ?? []).join(", ") || "—"}</td>
                  <td className="muted">{s.description ?? "—"}</td>
                  <td className="row-actions">
                    <button
                      className="btn btn-ghost"
                      onClick={() => {
                        setCreating(false);
                        setEditing(name);
                      }}
                    >
                      Edit
                    </button>
                    <button className="btn btn-ghost danger" onClick={() => void remove(name)}>
                      Delete
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>

      {openName && (
        <ServerForm
          initial={
            editing !== null ? (registry.servers?.[editing] ?? null) : null
          }
          initialName={editing ?? ""}
          existingNames={Object.keys(registry.servers ?? {})}
          onSubmit={submit}
          onClose={() => {
            setEditing(null);
            setCreating(false);
          }}
        />
      )}
    </div>
  );
}
