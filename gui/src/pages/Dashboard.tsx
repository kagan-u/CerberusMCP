import { useState } from "react";
import { api } from "../api";
import type {
  AppInfo,
  GatewayStatus,
  Registry,
  StartOptions,
} from "../types";

interface Props {
  info: AppInfo | null;
  registry: Registry | null;
  status: GatewayStatus | null;
  onStatusChange: (status: GatewayStatus) => void;
  onError: (message: string) => void;
}

export default function Dashboard({
  info,
  registry,
  status,
  onStatusChange,
  onError,
}: Props) {
  const [bind, setBind] = useState("127.0.0.1:8713");
  const [token, setToken] = useState("");
  const [allowRemote, setAllowRemote] = useState(false);
  const [profile, setProfile] = useState("");
  const [watch, setWatch] = useState(true);
  const [watchInterval, setWatchInterval] = useState(500);
  const [busy, setBusy] = useState(false);

  const servers = Object.entries(registry?.servers ?? {});
  const enabledCount = servers.filter(([, s]) => s.enabled).length;
  const profiles = Object.keys(registry?.profiles ?? {});
  const running = status?.running ?? false;

  const start = async () => {
    setBusy(true);
    try {
      const opts: StartOptions = {
        bind: bind.trim(),
        allowRemote,
        token: token || null,
        profile: profile || null,
        watch,
        watchInterval: watch ? watchInterval : null,
      };
      onStatusChange(await api.gatewayStart(opts));
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const stop = async () => {
    setBusy(true);
    try {
      onStatusChange(await api.gatewayStop());
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="page">
      <header className="page-head">
        <h1>Dashboard</h1>
        <span className="muted">
          config: <code>{info?.config_dir}</code>
        </span>
      </header>

      <div className="grid-2">
        <section className="card">
          <div className="card-title-row">
            <h2>Gateway</h2>
            <span className={running ? "pill pill-green" : "pill"}>
              {running ? (status?.managed ? "running (this app)" : "running (external)") : "stopped"}
            </span>
          </div>

          {running ? (
            <dl className="kv">
              <dt>Transport</dt>
              <dd>{status?.transport ?? "http"}</dd>
              <dt>Bind</dt>
              <dd>
                <code>{status?.bind ?? "—"}</code>
              </dd>
              <dt>PID</dt>
              <dd>{status?.pid ?? "—"}</dd>
              <dt>Watch</dt>
              <dd>{status?.watch ? "on" : "off"}</dd>
              <dt>Started</dt>
              <dd>
                {status?.startedUnix
                  ? new Date(status.startedUnix * 1000).toLocaleTimeString()
                  : "—"}
              </dd>
            </dl>
          ) : (
            <>
              {status?.lastEvent && (
                <p className="muted small">Last run: {status.lastEvent}</p>
              )}
              <div className="form-grid">
                <label>
                  Bind
                  <input value={bind} onChange={(e) => setBind(e.target.value)} />
                </label>
                <label>
                  Auth token
                  <input
                    type="password"
                    placeholder="required for non-loopback"
                    value={token}
                    onChange={(e) => setToken(e.target.value)}
                  />
                </label>
                <label>
                  Profile
                  <select value={profile} onChange={(e) => setProfile(e.target.value)}>
                    <option value="">active profile / all</option>
                    {profiles.map((name) => (
                      <option key={name} value={name}>
                        {name}
                      </option>
                    ))}
                  </select>
                </label>
                <label>
                  Watch interval (ms)
                  <input
                    type="number"
                    min={50}
                    value={watchInterval}
                    onChange={(e) => setWatchInterval(Number(e.target.value) || 500)}
                  />
                </label>
              </div>
              <div className="check-row">
                <label className="check">
                  <input
                    type="checkbox"
                    checked={allowRemote}
                    onChange={(e) => setAllowRemote(e.target.checked)}
                  />
                  allow remote bind
                </label>
                <label className="check">
                  <input
                    type="checkbox"
                    checked={watch}
                    onChange={(e) => setWatch(e.target.checked)}
                  />
                  watch conf.cerberus (hot reload)
                </label>
              </div>
            </>
          )}

          <div className="actions">
            {running ? (
              <button className="btn btn-danger" disabled={busy} onClick={() => void stop()}>
                Stop gateway
              </button>
            ) : (
              <button className="btn btn-primary" disabled={busy} onClick={() => void start()}>
                {busy ? "Starting…" : "Start gateway"}
              </button>
            )}
          </div>
        </section>

        <section className="card">
          <div className="card-title-row">
            <h2>Registry</h2>
            <span className="pill">{registry?.schema_version === 2 ? "schema v2" : "unknown"}</span>
          </div>
          <dl className="kv">
            <dt>Servers</dt>
            <dd>
              {enabledCount} enabled / {servers.length} total
            </dd>
            <dt>Profiles</dt>
            <dd>{profiles.length}</dd>
            <dt>Active profile</dt>
            <dd>{registry?.active_profile ?? "all servers"}</dd>
            <dt>Commands</dt>
            <dd>{Object.keys(registry?.commands ?? {}).length}</dd>
            <dt>Path</dt>
            <dd>
              <code className="truncate">{info?.registry_path}</code>
            </dd>
          </dl>
        </section>
      </div>

      <section className="card">
        <div className="card-title-row">
          <h2>Enabled servers</h2>
        </div>
        {servers.length === 0 ? (
          <p className="muted">No servers in the registry yet.</p>
        ) : (
          <table className="table">
            <thead>
              <tr>
                <th>Name</th>
                <th>Transport</th>
                <th>Mode</th>
                <th>Description</th>
              </tr>
            </thead>
            <tbody>
              {servers.map(([name, s]) => (
                <tr key={name} className={s.enabled ? "" : "row-dim"}>
                  <td>
                    <code>{name}</code>
                    {!s.enabled && <span className="pill pill-warn">disabled</span>}
                  </td>
                  <td>
                    <span className="badge">{s.transport.type}</span>
                  </td>
                  <td>{s.mode ?? registry?.settings?.default_mode ?? "direct"}</td>
                  <td className="muted">{s.description ?? "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </div>
  );
}
