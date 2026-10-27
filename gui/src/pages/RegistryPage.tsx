import { useEffect, useState } from "react";
import { api } from "../api";
import type { CommandDef, ExecutionMode, Profile, Registry } from "../types";

interface Props {
  registry: Registry;
  refresh: () => Promise<void>;
  onError: (message: string) => void;
}

interface CommandDraft {
  open: boolean;
  name: string;
  runText: string;
  description: string;
  timeoutMs: string;
  cwd: string;
  editing: string | null;
}

const EMPTY_COMMAND: CommandDraft = {
  open: false,
  name: "",
  runText: "",
  description: "",
  timeoutMs: "",
  cwd: "",
  editing: null,
};

function splitLines(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

export default function RegistryPage({ registry, refresh, onError }: Props) {
  const [defaultMode, setDefaultMode] = useState<ExecutionMode>(
    registry.settings?.default_mode ?? "direct",
  );
  const [logLevel, setLogLevel] = useState(registry.settings?.log_level ?? "");
  const [activeProfile, setActiveProfile] = useState(registry.active_profile ?? "");
  const [commandDraft, setCommandDraft] = useState<CommandDraft>(EMPTY_COMMAND);

  useEffect(() => {
    setDefaultMode(registry.settings?.default_mode ?? "direct");
    setLogLevel(registry.settings?.log_level ?? "");
    setActiveProfile(registry.active_profile ?? "");
  }, [registry]);

  const run = async (op: () => Promise<unknown>) => {
    try {
      await op();
      await refresh();
    } catch (e) {
      onError(String(e));
    }
  };

  const saveSettings = () =>
    run(() =>
      api.setSettings({
        default_mode: defaultMode,
        ...(logLevel ? { log_level: logLevel } : {}),
      }),
    );

  const saveActiveProfile = (name: string) =>
    run(() => api.setActiveProfile(name || null));

  const saveProfile = (name: string, profile: Profile) =>
    run(() => api.upsertProfile(name, profile));

  const removeProfile = (name: string) => {
    if (!window.confirm(`Delete profile “${name}”?`)) return;
    void run(() => api.deleteProfile(name));
  };

  const saveCommands = (commands: Record<string, CommandDef>) =>
    run(() => api.setCommands(commands));

  const commands = registry.commands ?? {};
  const profiles = Object.entries(registry.profiles ?? {}).sort(([a], [b]) =>
    a.localeCompare(b),
  );
  const serverNames = Object.keys(registry.servers ?? {});

  const commandBusy = false;

  return (
    <div className="page">
      <header className="page-head">
        <h1>Registry</h1>
        <span className="muted">
          schema v{registry.schema_version} · active profile:{" "}
          {registry.active_profile ?? "none (all servers)"}
        </span>
      </header>

      <section className="card">
        <div className="card-title-row">
          <h2>Settings</h2>
        </div>
        <div className="form-grid cols-3">
          <label>
            Default mode
            <select
              value={defaultMode}
              onChange={(e) => setDefaultMode(e.target.value as ExecutionMode)}
            >
              <option value="direct">direct</option>
              <option value="gateway">gateway</option>
            </select>
          </label>
          <label>
            Log level
            <select value={logLevel} onChange={(e) => setLogLevel(e.target.value)}>
              <option value="">(unset)</option>
              <option value="error">error</option>
              <option value="warn">warn</option>
              <option value="info">info</option>
              <option value="debug">debug</option>
            </select>
          </label>
          <div className="actions-end">
            <button className="btn btn-primary" onClick={() => void saveSettings()}>
              Save settings
            </button>
          </div>
        </div>
      </section>

      <section className="card">
        <div className="card-title-row">
          <h2>Profiles</h2>
        </div>
        <label className="field-inline">
          Active profile
          <select
            value={activeProfile}
            onChange={(e) => void saveActiveProfile(e.target.value)}
          >
            <option value="">none — all enabled servers</option>
            {profiles.map(([name]) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>
        </label>

        <table className="table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Servers</th>
              <th>Description</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {profiles.map(([name, profile]) => (
              <tr key={name}>
                <td>
                  <code>{name}</code>
                  {registry.active_profile === name && (
                    <span className="pill pill-green">active</span>
                  )}
                </td>
                <td>
                  <div className="checkbox-row">
                    {serverNames.map((server) => {
                      const checked = (profile.servers ?? []).includes(server);
                      return (
                        <label className="check" key={server}>
                          <input
                            type="checkbox"
                            checked={checked}
                            onChange={(e) => {
                              const list = new Set(profile.servers ?? []);
                              if (e.target.checked) list.add(server);
                              else list.delete(server);
                              void saveProfile(name, {
                                ...profile,
                                servers: [...list].sort(),
                              });
                            }}
                          />
                          {server}
                        </label>
                      );
                    })}
                    {serverNames.length === 0 && (
                      <span className="muted small">no servers yet</span>
                    )}
                  </div>
                </td>
                <td className="muted">{profile.description ?? "—"}</td>
                <td className="row-actions">
                  <button
                    className="btn btn-ghost danger"
                    onClick={() => removeProfile(name)}
                  >
                    Delete
                  </button>
                </td>
              </tr>
            ))}
            {profiles.length === 0 && (
              <tr>
                <td colSpan={4} className="muted">
                  No profiles. Create one with <code>cerberus profile add</code>{" "}
                  or below.
                </td>
              </tr>
            )}
          </tbody>
        </table>

        <div className="row-line profile-add">
          <input
            id="new-profile-name"
            placeholder="new-profile-name"
            onChange={() => undefined}
          />
          <button
            className="btn"
            onClick={() => {
              const input = document.getElementById(
                "new-profile-name",
              ) as HTMLInputElement | null;
              const name = input?.value.trim();
              if (!name) return;
              input!.value = "";
              void saveProfile(name, { servers: [] });
            }}
          >
            + Create profile
          </button>
        </div>
      </section>

      <section className="card">
        <div className="card-title-row">
          <h2>Reusable commands</h2>
          <button
            className="btn btn-primary"
            onClick={() => setCommandDraft({ ...EMPTY_COMMAND, open: true })}
          >
            + Add command
          </button>
        </div>

        {commandDraft.open ? (
          <div className="inline-form">
            <div className="form-grid cols-2">
              <label>
                Name
                <input
                  value={commandDraft.editing ?? commandDraft.name}
                  disabled={commandDraft.editing !== null}
                  placeholder="bootstrap"
                  onChange={(e) =>
                    setCommandDraft({ ...commandDraft, name: e.target.value })
                  }
                />
              </label>
              <label>
                Timeout (ms)
                <input
                  value={commandDraft.timeoutMs}
                  placeholder="10000"
                  onChange={(e) =>
                    setCommandDraft({ ...commandDraft, timeoutMs: e.target.value })
                  }
                />
              </label>
              <label className="span-2">
                Arguments (one per line, exec vector — no implicit shell)
                <textarea
                  rows={3}
                  value={commandDraft.runText}
                  onChange={(e) =>
                    setCommandDraft({ ...commandDraft, runText: e.target.value })
                  }
                />
              </label>
              <label>
                Working directory
                <input
                  value={commandDraft.cwd}
                  onChange={(e) =>
                    setCommandDraft({ ...commandDraft, cwd: e.target.value })
                  }
                />
              </label>
              <label>
                Description
                <input
                  value={commandDraft.description}
                  onChange={(e) =>
                    setCommandDraft({ ...commandDraft, description: e.target.value })
                  }
                />
              </label>
            </div>
            <div className="actions">
              <button
                className="btn"
                onClick={() => setCommandDraft(EMPTY_COMMAND)}
              >
                Cancel
              </button>
              <button
                className="btn btn-primary"
                disabled={commandBusy}
                onClick={() => {
                  const name = (commandDraft.editing ?? commandDraft.name).trim();
                  const run = splitLines(commandDraft.runText);
                  if (!name || run.length === 0) {
                    onError("Command needs a name and at least one argument.");
                    return;
                  }
                  const next: Record<string, CommandDef> = { ...commands };
                  if (commandDraft.editing && commandDraft.editing !== name) {
                    delete next[commandDraft.editing];
                  }
                  next[name] = {
                    run,
                    ...(commandDraft.cwd.trim()
                      ? { cwd: commandDraft.cwd.trim() }
                      : {}),
                    ...(commandDraft.timeoutMs.trim()
                      ? { timeout_ms: Number(commandDraft.timeoutMs) || 0 }
                      : {}),
                    ...(commandDraft.description.trim()
                      ? { description: commandDraft.description.trim() }
                      : {}),
                  };
                  void saveCommands(next).then(() =>
                    setCommandDraft(EMPTY_COMMAND),
                  );
                }}
              >
                Save command
              </button>
            </div>
          </div>
        ) : Object.keys(commands).length === 0 ? (
          <p className="muted">No reusable commands.</p>
        ) : (
          <table className="table">
            <thead>
              <tr>
                <th>Name</th>
                <th>Run</th>
                <th>Description</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {Object.entries(commands).map(([name, cmd]) => (
                <tr key={name}>
                  <td>
                    <code>{name}</code>
                  </td>
                  <td>
                    <code className="truncate">{cmd.run.join(" ")}</code>
                  </td>
                  <td className="muted">{cmd.description ?? "—"}</td>
                  <td className="row-actions">
                    <button
                      className="btn btn-ghost"
                      onClick={() =>
                        setCommandDraft({
                          open: true,
                          name,
                          editing: name,
                          runText: cmd.run.join("\n"),
                          description: cmd.description ?? "",
                          timeoutMs:
                            cmd.timeout_ms != null ? String(cmd.timeout_ms) : "",
                          cwd: cmd.cwd ?? "",
                        })
                      }
                    >
                      Edit
                    </button>
                    <button
                      className="btn btn-ghost danger"
                      onClick={() => {
                        const next = { ...commands };
                        delete next[name];
                        void saveCommands(next);
                      }}
                    >
                      Delete
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </div>
  );
}
