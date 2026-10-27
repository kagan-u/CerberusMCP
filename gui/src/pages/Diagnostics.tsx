import { useState } from "react";
import { api } from "../api";
import type { DoctorReport, ValidateReport } from "../types";

interface Props {
  onError: (message: string) => void;
}

export default function Diagnostics({ onError }: Props) {
  const [report, setReport] = useState<ValidateReport | null>(null);
  const [doctor, setDoctor] = useState<DoctorReport | null>(null);
  const [probeServers, setProbeServers] = useState(false);
  const [busy, setBusy] = useState<"validate" | "doctor" | null>(null);

  const validate = async () => {
    setBusy("validate");
    try {
      setReport(await api.validate());
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const runDoctor = async () => {
    setBusy("doctor");
    try {
      setDoctor(await api.runDoctor(probeServers));
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="page">
      <header className="page-head">
        <h1>Diagnostics</h1>
        <div className="head-actions">
          <label className="check">
            <input
              type="checkbox"
              checked={probeServers}
              onChange={(e) => setProbeServers(e.target.checked)}
            />
            probe server processes (spawns)
          </label>
          <button
            className="btn"
            disabled={busy !== null}
            onClick={() => void validate()}
          >
            {busy === "validate" ? "Validating…" : "Validate registry"}
          </button>
          <button
            className="btn btn-primary"
            disabled={busy !== null}
            onClick={() => void runDoctor()}
          >
            {busy === "doctor" ? "Running…" : "Run doctor"}
          </button>
        </div>
      </header>

      {report && (
        <div className={report.ok ? "banner banner-ok" : "banner banner-error"}>
          {report.ok
            ? "Registry is valid (schema v2)."
            : report.error ?? "Registry is invalid."}
        </div>
      )}

      {doctor && (
        <>
          <section className="card">
            <div className="card-title-row">
              <h2>Doctor summary</h2>
              <span className={doctor.config_valid ? "pill pill-green" : "pill pill-warn"}>
                {doctor.config_valid ? "config ok" : "config problems"}
              </span>
            </div>
            <dl className="kv">
              <dt>Config</dt>
              <dd>
                <code className="truncate">{doctor.config_path}</code>
              </dd>
              <dt>Schema</dt>
              <dd>{doctor.schema_version ? `v${doctor.schema_version}` : "—"}</dd>
              <dt>Servers</dt>
              <dd>{doctor.server_count}</dd>
              <dt>Issues</dt>
              <dd>
                <span className="pill pill-warn">{doctor.warnings} warnings</span>{" "}
                <span className="pill pill-red">{doctor.critical} critical</span>
              </dd>
            </dl>
          </section>

          <section className="card">
            <div className="card-title-row">
              <h2>Diagnostics</h2>
            </div>
            {doctor.diagnostics.length === 0 ? (
              <p className="muted">No problems found.</p>
            ) : (
              <table className="table">
                <thead>
                  <tr>
                    <th>Severity</th>
                    <th>Area</th>
                    <th>Message</th>
                  </tr>
                </thead>
                <tbody>
                  {doctor.diagnostics.map((d, i) => (
                    <tr key={i}>
                      <td>
                        <span className={`sev sev-${d.severity}`}>{d.severity}</span>
                      </td>
                      <td>{d.area}</td>
                      <td className="wrap">{d.message}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </section>

          <section className="card">
            <div className="card-title-row">
              <h2>Clients</h2>
            </div>
            <table className="table">
              <thead>
                <tr>
                  <th>Client</th>
                  <th>Status</th>
                  <th>Detected</th>
                  <th>Servers synced</th>
                </tr>
              </thead>
              <tbody>
                {doctor.clients.map((c) => (
                  <tr key={c.client}>
                    <td>{c.display_name}</td>
                    <td className="muted">{c.status}</td>
                    <td>{c.detected ? "yes" : "no"}</td>
                    <td>{c.servers}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </section>
        </>
      )}

      {!report && !doctor && (
        <section className="card">
          <p className="muted">
            Run <strong>Validate</strong> for a fast schema check, or{" "}
            <strong>Run doctor</strong> for the full sweep (registry, server
            executables, client adapters).
          </p>
        </section>
      )}
    </div>
  );
}
