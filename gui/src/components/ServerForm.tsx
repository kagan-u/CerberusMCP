import { useState } from "react";
import type {
  EnvValue,
  ExecutionMode,
  HeaderValue,
  HookStep,
  ServerDef,
  SourceDef,
  Transport,
} from "../types";

interface Props {
  initial: ServerDef | null;
  initialName: string;
  existingNames: string[];
  onSubmit: (name: string, def: ServerDef) => Promise<boolean>;
  onClose: () => void;
}

interface EnvRow {
  key: string;
  source: "env" | "secret_ref" | "literal";
  value: string;
}

interface HookRow {
  kind: "use" | "run";
  useName: string;
  runText: string;
  cwd: string;
  timeoutMs: string;
  continueOnError: boolean;
  envRows: EnvRow[];
}

const NAME_RE = /^[a-z0-9-]{1,64}$/;

function splitLines(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

function numOrNull(text: string): number | null {
  const trimmed = text.trim();
  if (!trimmed) return null;
  const value = Number(trimmed);
  return Number.isFinite(value) && value >= 0 ? value : null;
}

function envFromRows(rows: EnvRow[]): Record<string, EnvValue> | undefined {
  const out: Record<string, EnvValue> = {};
  for (const row of rows) {
    const key = row.key.trim();
    if (!key) continue;
    out[key] =
      row.source === "literal"
        ? { source: "literal", value: row.value }
        : { source: row.source, name: row.value };
  }
  return Object.keys(out).length > 0 ? out : undefined;
}

function envRowsOf(env: Record<string, EnvValue> | undefined): EnvRow[] {
  if (!env) return [];
  return Object.entries(env).map(([key, value]) => ({
    key,
    source: value.source,
    value:
      value.source === "literal" ? value.value : value.name,
  }));
}

function transportRows(transport: Transport): {
  kind: "stdio" | "http";
  command: string;
  argsText: string;
  cwd: string;
  envRows: EnvRow[];
  url: string;
  headerRows: EnvRow[];
  bearerSource: string;
  bearerValue: string;
} {
  if (transport.type === "stdio") {
    return {
      kind: "stdio",
      command: transport.command,
      argsText: (transport.args ?? []).join("\n"),
      cwd: transport.cwd ?? "",
      envRows: envRowsOf(transport.env),
      url: "",
      headerRows: [],
      bearerSource: "",
      bearerValue: "",
    };
  }
  const headers: EnvRow[] = Object.entries(transport.headers ?? {}).map(
    ([key, value]) => {
      if (typeof value === "string") {
        return { key, source: "literal" as const, value };
      }
      return {
        key,
        source: value.source,
        value: value.source === "literal" ? value.value : value.name,
      };
    },
  );
  const bearer = transport.bearer_token;
  return {
    kind: "http",
    command: "",
    argsText: "",
    cwd: "",
    envRows: [],
    url: transport.url,
    headerRows: headers,
    bearerSource: bearer ? bearer.source : "",
    bearerValue: bearer
      ? bearer.source === "literal"
        ? bearer.value
        : bearer.name
      : "",
  };
}

function hookRowsOf(hooks: HookStep[] | undefined): HookRow[] {
  return (hooks ?? []).map((hook) => ({
    kind: hook.use ? "use" : "run",
    useName: hook.use ?? "",
    runText: (hook.run ?? []).join("\n"),
    cwd: hook.cwd ?? "",
    timeoutMs: hook.timeout_ms != null ? String(hook.timeout_ms) : "",
    continueOnError: hook.continue_on_error ?? false,
    envRows: envRowsOf(hook.env),
  }));
}

function headerValueOf(row: EnvRow): HeaderValue {
  if (row.source === "literal") return row.value;
  return { source: row.source, name: row.value };
}

export default function ServerForm({
  initial,
  initialName,
  existingNames,
  onSubmit,
  onClose,
}: Props) {
  const initTransport = initial ? transportRows(initial.transport) : null;
  const [name, setName] = useState(initialName);
  const [enabled, setEnabled] = useState(initial?.enabled ?? true);
  const [description, setDescription] = useState(initial?.description ?? "");
  const [tags, setTags] = useState((initial?.tags ?? []).join(", "));
  const [mode, setMode] = useState<"" | ExecutionMode>(
    initial?.mode === "gateway" ? "gateway" : initial?.mode === "direct" ? "direct" : "",
  );
  const [timeoutMs, setTimeoutMs] = useState(
    initial?.timeout_ms != null ? String(initial.timeout_ms) : "",
  );
  const [startupTimeoutMs, setStartupTimeoutMs] = useState(
    initial?.startup_timeout_ms != null ? String(initial.startup_timeout_ms) : "",
  );
  const [kind, setKind] = useState<"stdio" | "http">(initTransport?.kind ?? "stdio");
  const [command, setCommand] = useState(initTransport?.command ?? "");
  const [argsText, setArgsText] = useState(initTransport?.argsText ?? "");
  const [cwd, setCwd] = useState(initTransport?.cwd ?? "");
  const [envRows, setEnvRows] = useState<EnvRow[]>(initTransport?.envRows ?? []);
  const [url, setUrl] = useState(initTransport?.url ?? "");
  const [headerRows, setHeaderRows] = useState<EnvRow[]>(initTransport?.headerRows ?? []);
  const [bearerSource, setBearerSource] = useState(initTransport?.bearerSource ?? "");
  const [bearerValue, setBearerValue] = useState(initTransport?.bearerValue ?? "");
  const [allowTools, setAllowTools] = useState(
    (initial?.policy?.allow_tools ?? []).join("\n"),
  );
  const [denyTools, setDenyTools] = useState(
    (initial?.policy?.deny_tools ?? []).join("\n"),
  );
  const [requireConfirmation, setRequireConfirmation] = useState(
    initial?.policy?.require_confirmation ?? false,
  );
  const [hooks, setHooks] = useState<HookRow[]>(hookRowsOf(initial?.pre_launch));
  const [saving, setSaving] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);

  const source: SourceDef | null = initial?.source ?? null;

  const patchRows = (
    rows: EnvRow[],
    index: number,
    patch: Partial<EnvRow>,
  ): EnvRow[] => rows.map((row, i) => (i === index ? { ...row, ...patch } : row));

  const buildDef = (): ServerDef | null => {
    const transport: Transport =
      kind === "stdio"
        ? {
            type: "stdio",
            command: command.trim(),
            args: splitLines(argsText),
            ...(envFromRows(envRows) ? { env: envFromRows(envRows) } : {}),
            cwd: cwd.trim() || null,
          }
        : {
            type: "http",
            url: url.trim(),
            ...(headerRows.length > 0
              ? {
                  headers: Object.fromEntries(
                    headerRows
                      .filter((row) => row.key.trim())
                      .map((row) => [row.key.trim(), headerValueOf(row)]),
                  ),
                }
              : {}),
            ...(bearerSource && bearerValue
              ? {
                  bearer_token:
                    bearerSource === "literal"
                      ? ({ source: "literal", value: bearerValue } as EnvValue)
                      : ({ source: bearerSource as "env" | "secret_ref", name: bearerValue } as EnvValue),
                }
              : {}),
          };

    const policy = {
      allow_tools: splitLines(allowTools),
      deny_tools: splitLines(denyTools),
      require_confirmation: requireConfirmation,
    };

    const preLaunch: HookStep[] = [];
    for (const [i, hook] of hooks.entries()) {
      if (hook.kind === "use" && !hook.useName.trim()) {
        setFormError(`Pre-launch step ${i + 1}: “use” needs a command name.`);
        return null;
      }
      if (hook.kind === "run" && splitLines(hook.runText).length === 0) {
        setFormError(`Pre-launch step ${i + 1}: “run” needs at least one argument.`);
        return null;
      }
      preLaunch.push({
        ...(hook.kind === "use"
          ? { use: hook.useName.trim() }
          : { run: splitLines(hook.runText) }),
        cwd: hook.cwd.trim() || null,
        ...(envFromRows(hook.envRows) ? { env: envFromRows(hook.envRows) } : {}),
        timeout_ms: numOrNull(hook.timeoutMs),
        continue_on_error: hook.continueOnError,
      });
    }

    return {
      enabled,
      transport,
      mode: mode || null,
      tags: tags
        .split(",")
        .map((tag) => tag.trim())
        .filter((tag) => tag.length > 0),
      timeout_ms: numOrNull(timeoutMs),
      startup_timeout_ms: numOrNull(startupTimeoutMs),
      policy,
      description: description.trim() || null,
      pre_launch: preLaunch,
      source,
    };
  };

  const save = async () => {
    setFormError(null);
    const trimmed = name.trim();
    const isNew = !initial;
    if (!NAME_RE.test(trimmed) || trimmed.includes("__")) {
      setFormError(
        "Name must match [a-z0-9-] (max 64) and must not contain “__”.",
      );
      return;
    }
    if (isNew && existingNames.includes(trimmed)) {
      setFormError(`Server “${trimmed}” already exists.`);
      return;
    }
    if (kind === "stdio" && !command.trim()) {
      setFormError("Stdio transport needs a command.");
      return;
    }
    if (kind === "http" && !url.trim()) {
      setFormError("HTTP transport needs a URL.");
      return;
    }
    const def = buildDef();
    if (!def) return;
    setSaving(true);
    const ok = await onSubmit(trimmed, def);
    setSaving(false);
    if (ok) onClose();
  };

  const envEditor = (
    rows: EnvRow[],
    setRows: (rows: EnvRow[]) => void,
    keyLabel: string,
    keyHint: string,
  ) => (
    <div className="rows">
      {rows.map((row, index) => (
        <div className="row-line" key={index}>
          <input
            placeholder={keyLabel}
            value={row.key}
            onChange={(e) => setRows(patchRows(rows, index, { key: e.target.value }))}
          />
          <select
            value={row.source}
            onChange={(e) =>
              setRows(
                patchRows(rows, index, {
                  source: e.target.value as EnvRow["source"],
                }),
              )
            }
          >
            <option value="env">from env</option>
            <option value="secret_ref">secret ref</option>
            <option value="literal">literal</option>
          </select>
          <input
            placeholder={keyHint}
            value={row.value}
            onChange={(e) => setRows(patchRows(rows, index, { value: e.target.value }))}
          />
          <button
            className="btn btn-ghost danger"
            onClick={() => setRows(rows.filter((_, i) => i !== index))}
          >
            ✕
          </button>
        </div>
      ))}
      <button
        className="btn btn-ghost"
        onClick={() =>
          setRows([...rows, { key: "", source: "env", value: "" }])
        }
      >
        + row
      </button>
    </div>
  );

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <header className="modal-head">
          <h2>{initial ? `Edit ${initialName}` : "Add server"}</h2>
          <button className="btn btn-ghost" onClick={onClose}>
            ✕
          </button>
        </header>

        {formError && <div className="banner banner-error">{formError}</div>}

        <div className="modal-body">
          <fieldset>
            <legend>General</legend>
            <div className="form-grid cols-2">
              <label>
                Name
                <input
                  value={name}
                  disabled={!!initial}
                  placeholder="my-server"
                  onChange={(e) => setName(e.target.value)}
                />
              </label>
              <label>
                Mode
                <select
                  value={mode}
                  onChange={(e) => setMode(e.target.value as "" | ExecutionMode)}
                >
                  <option value="">default (settings)</option>
                  <option value="direct">direct</option>
                  <option value="gateway">gateway</option>
                </select>
              </label>
              <label className="span-2">
                Description
                <input
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                />
              </label>
              <label>
                Tags (comma separated)
                <input value={tags} onChange={(e) => setTags(e.target.value)} />
              </label>
              <label className="check inline">
                <input
                  type="checkbox"
                  checked={enabled}
                  onChange={(e) => setEnabled(e.target.checked)}
                />
                enabled
              </label>
              <label>
                Call timeout (ms)
                <input
                  value={timeoutMs}
                  placeholder="30000"
                  onChange={(e) => setTimeoutMs(e.target.value)}
                />
              </label>
              <label>
                Startup timeout (ms)
                <input
                  value={startupTimeoutMs}
                  placeholder="15000"
                  onChange={(e) => setStartupTimeoutMs(e.target.value)}
                />
              </label>
            </div>
          </fieldset>

          <fieldset>
            <legend>Transport</legend>
            <div className="check-row">
              <label className="check">
                <input
                  type="radio"
                  name="transport"
                  checked={kind === "stdio"}
                  onChange={() => setKind("stdio")}
                />
                stdio (local process)
              </label>
              <label className="check">
                <input
                  type="radio"
                  name="transport"
                  checked={kind === "http"}
                  onChange={() => setKind("http")}
                />
                http (remote)
              </label>
            </div>

            {kind === "stdio" ? (
              <div className="form-grid cols-2">
                <label>
                  Command
                  <input
                    value={command}
                    placeholder="npx"
                    onChange={(e) => setCommand(e.target.value)}
                  />
                </label>
                <label>
                  Working directory
                  <input
                    value={cwd}
                    placeholder="~ or absolute path"
                    onChange={(e) => setCwd(e.target.value)}
                  />
                </label>
                <label className="span-2">
                  Arguments (one per line)
                  <textarea
                    rows={3}
                    value={argsText}
                    onChange={(e) => setArgsText(e.target.value)}
                  />
                </label>
                <div className="span-2">
                  <div className="sub-label">Environment</div>
                  {envEditor(envRows, setEnvRows, "KEY", "env name / value")}
                </div>
              </div>
            ) : (
              <div className="form-grid cols-2">
                <label className="span-2">
                  URL
                  <input
                    value={url}
                    placeholder="https://example.com/mcp"
                    onChange={(e) => setUrl(e.target.value)}
                  />
                </label>
                <div className="span-2">
                  <div className="sub-label">Headers</div>
                  {envEditor(headerRows, setHeaderRows, "Header", "value")}
                </div>
                <label>
                  Bearer token source
                  <select
                    value={bearerSource}
                    onChange={(e) => setBearerSource(e.target.value)}
                  >
                    <option value="">none</option>
                    <option value="env">from env</option>
                    <option value="secret_ref">secret ref</option>
                    <option value="literal">literal</option>
                  </select>
                </label>
                <label>
                  Bearer value
                  <input
                    value={bearerValue}
                    disabled={!bearerSource}
                    onChange={(e) => setBearerValue(e.target.value)}
                  />
                </label>
              </div>
            )}
          </fieldset>

          <fieldset>
            <legend>Policy</legend>
            <div className="form-grid cols-2">
              <label>
                Allow tools (one per line, trailing * wildcards)
                <textarea
                  rows={3}
                  value={allowTools}
                  onChange={(e) => setAllowTools(e.target.value)}
                />
              </label>
              <label>
                Deny tools (one per line)
                <textarea
                  rows={3}
                  value={denyTools}
                  onChange={(e) => setDenyTools(e.target.value)}
                />
              </label>
              <label className="check inline span-2">
                <input
                  type="checkbox"
                  checked={requireConfirmation}
                  onChange={(e) => setRequireConfirmation(e.target.checked)}
                />
                require confirmation for every call
              </label>
            </div>
          </fieldset>

          <fieldset>
            <legend>Pre-launch hooks (stdio, gateway mode)</legend>
            {hooks.map((hook, index) => (
              <div className="hook" key={index}>
                <div className="row-line">
                  <select
                    value={hook.kind}
                    onChange={(e) =>
                      setHooks(
                        hooks.map((h, i) =>
                          i === index
                            ? { ...h, kind: e.target.value as "use" | "run" }
                            : h,
                        ),
                      )
                    }
                  >
                    <option value="use">use command</option>
                    <option value="run">inline run</option>
                  </select>
                  {hook.kind === "use" ? (
                    <input
                      placeholder="command name from [commands.*]"
                      value={hook.useName}
                      onChange={(e) =>
                        setHooks(
                          hooks.map((h, i) =>
                            i === index ? { ...h, useName: e.target.value } : h,
                          ),
                        )
                      }
                    />
                  ) : (
                    <input
                      placeholder="argv, one per line (sh -c … for shells)"
                      value={hook.runText}
                      onChange={(e) =>
                        setHooks(
                          hooks.map((h, i) =>
                            i === index ? { ...h, runText: e.target.value } : h,
                          ),
                        )
                      }
                    />
                  )}
                  <input
                    placeholder="cwd"
                    value={hook.cwd}
                    onChange={(e) =>
                      setHooks(
                        hooks.map((h, i) =>
                          i === index ? { ...h, cwd: e.target.value } : h,
                        ),
                      )
                    }
                  />
                  <input
                    className="narrow"
                    placeholder="ms"
                    value={hook.timeoutMs}
                    onChange={(e) =>
                      setHooks(
                        hooks.map((h, i) =>
                          i === index ? { ...h, timeoutMs: e.target.value } : h,
                        ),
                      )
                    }
                  />
                  <label className="check">
                    <input
                      type="checkbox"
                      checked={hook.continueOnError}
                      onChange={(e) =>
                        setHooks(
                          hooks.map((h, i) =>
                            i === index
                              ? { ...h, continueOnError: e.target.checked }
                              : h,
                          ),
                        )
                      }
                    />
                    continue on error
                  </label>
                  <button
                    className="btn btn-ghost danger"
                    onClick={() =>
                      setHooks(hooks.filter((_, i) => i !== index))
                    }
                  >
                    ✕
                  </button>
                </div>
                {envEditor(
                  hook.envRows,
                  (rows) =>
                    setHooks(
                      hooks.map((h, i) => (i === index ? { ...h, envRows: rows } : h)),
                    ),
                  "KEY",
                  "env name / value",
                )}
              </div>
            ))}
            <button
              className="btn btn-ghost"
              onClick={() =>
                setHooks([
                  ...hooks,
                  {
                    kind: "use",
                    useName: "",
                    runText: "",
                    cwd: "",
                    timeoutMs: "",
                    continueOnError: false,
                    envRows: [],
                  },
                ])
              }
            >
              + step
            </button>
          </fieldset>

          {source && (
            <fieldset>
              <legend>Install provenance (read-only)</legend>
              <p className="muted small">
                <span className="badge">{source.kind}</span>{" "}
                <code>{source.package ?? source.url ?? "—"}</code>
                {source.version && ` @ ${source.version}`}
                {source.runner && ` via ${source.runner}`}
              </p>
            </fieldset>
          )}
        </div>

        <footer className="modal-foot">
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            className="btn btn-primary"
            disabled={saving}
            onClick={() => void save()}
          >
            {saving ? "Saving…" : "Save"}
          </button>
        </footer>
      </div>
    </div>
  );
}
