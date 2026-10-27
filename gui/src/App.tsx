import { useCallback, useEffect, useState } from "react";
import "./App.css";
import { api, onLogLine } from "./api";
import type { AppInfo, GatewayStatus, Registry } from "./types";
import Dashboard from "./pages/Dashboard";
import Servers from "./pages/Servers";
import RegistryPage from "./pages/RegistryPage";
import Diagnostics from "./pages/Diagnostics";
import Logs from "./pages/Logs";

const PAGES = ["dashboard", "servers", "registry", "diagnostics", "logs"] as const;
type Page = (typeof PAGES)[number];

const PAGE_LABELS: Record<Page, string> = {
  dashboard: "Dashboard",
  servers: "Servers",
  registry: "Registry",
  diagnostics: "Diagnostics",
  logs: "Logs",
};

const MAX_LOGS = 4000;

export default function App() {
  const [page, setPage] = useState<Page>("dashboard");
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [registry, setRegistry] = useState<Registry | null>(null);
  const [registryPath, setRegistryPath] = useState<string>("");
  const [exists, setExists] = useState(true);
  const [status, setStatus] = useState<GatewayStatus | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);

  const refreshRegistry = useCallback(async () => {
    try {
      const view = await api.getRegistry();
      setRegistry(view.registry);
      setRegistryPath(view.path);
      setExists(view.exists);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const refreshStatus = useCallback(async () => {
    try {
      setStatus(await api.gatewayStatus());
    } catch {
      /* gateway status is best-effort while the backend warms up */
    }
  }, []);

  useEffect(() => {
    void (async () => {
      try {
        setInfo(await api.appInfo());
      } catch (e) {
        setError(String(e));
      }
      await refreshRegistry();
      await refreshStatus();
      try {
        setLogs(await api.getLogs(MAX_LOGS));
      } catch {
        /* empty logs are fine */
      }
    })();
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void onLogLine((line) => {
      if (disposed) return;
      setLogs((prev) => {
        const next = [...prev, line];
        return next.length > MAX_LOGS ? next.slice(next.length - MAX_LOGS) : next;
      });
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [refreshRegistry, refreshStatus]);

  // Poll gateway status so start/stop from outside the app stays visible.
  useEffect(() => {
    const id = window.setInterval(() => void refreshStatus(), 2500);
    return () => window.clearInterval(id);
  }, [refreshStatus]);

  const initRegistry = async () => {
    try {
      const view = await api.initRegistry();
      setRegistry(view.registry);
      setRegistryPath(view.path);
      setExists(view.exists);
      setInfo(await api.appInfo());
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark">C</span>
          <div>
            <div className="brand-name">CerberusMCP</div>
            <div className="brand-sub">v{info?.cerberus_version ?? "…"}</div>
          </div>
        </div>
        <nav>
          {PAGES.map((id) => (
            <button
              key={id}
              className={page === id ? "nav-item active" : "nav-item"}
              onClick={() => setPage(id)}
            >
              {PAGE_LABELS[id]}
            </button>
          ))}
        </nav>
        <div className="sidebar-foot">
          <div className="path-label" title={registryPath}>
            {exists ? "conf.cerberus" : "no registry"}
          </div>
          {status?.running && <div className="pill pill-green">gateway live</div>}
        </div>
      </aside>

      <main className="content">
        {error && (
          <div className="banner banner-error" onClick={() => setError(null)}>
            {error}
          </div>
        )}
        {!exists && page !== "dashboard" && (
          <div className="banner banner-warn">
            No registry at <code>{registryPath}</code> —{" "}
            <button className="linklike" onClick={() => void initRegistry()}>
              create an empty one
            </button>
          </div>
        )}
        {page === "dashboard" && (
          <Dashboard
            info={info}
            registry={registry}
            status={status}
            onStatusChange={setStatus}
            onError={setError}
          />
        )}
        {page === "servers" && registry && (
          <Servers
            registry={registry}
            refresh={refreshRegistry}
            onError={setError}
          />
        )}
        {page === "registry" && registry && (
          <RegistryPage
            registry={registry}
            refresh={refreshRegistry}
            onError={setError}
          />
        )}
        {page === "diagnostics" && <Diagnostics onError={setError} />}
        {page === "logs" && (
          <Logs lines={logs} onClear={() => setLogs([])} />
        )}
      </main>
    </div>
  );
}
