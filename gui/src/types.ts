/** Types mirroring the CerberusMCP registry schema (serde JSON shape). */

export type ExecutionMode = "direct" | "gateway";

export type Severity = "info" | "warning" | "critical";

export interface Settings {
  default_mode?: ExecutionMode;
  log_level?: string | null;
}

export type EnvValue =
  | { source: "env"; name: string }
  | { source: "secret_ref"; name: string }
  | { source: "literal"; value: string };

/** Untagged on the Rust side: a resolved EnvValue object or a plain string. */
export type HeaderValue = EnvValue | string;

export interface StdioTransport {
  type: "stdio";
  command: string;
  args?: string[];
  env?: Record<string, EnvValue>;
  cwd?: string | null;
}

export interface HttpTransport {
  type: "http";
  url: string;
  headers?: Record<string, HeaderValue>;
  bearer_token?: EnvValue | null;
}

export type Transport = StdioTransport | HttpTransport;

export interface ServerPolicy {
  allow_tools?: string[];
  deny_tools?: string[];
  require_confirmation?: boolean;
}

export interface HookStep {
  use?: string;
  run?: string[];
  cwd?: string | null;
  env?: Record<string, EnvValue>;
  timeout_ms?: number | null;
  continue_on_error?: boolean;
}

export interface SourceEnv {
  name: string;
  required?: boolean;
  secret?: boolean;
  description?: string | null;
}

export interface SourceDef {
  kind: "npm" | "pypi" | "git" | "binary";
  package?: string | null;
  version?: string | null;
  homepage?: string | null;
  runner?: string | null;
  args?: string[];
  url?: string | null;
  ref?: string | null;
  env?: SourceEnv[];
}

export interface ServerDef {
  enabled: boolean;
  transport: Transport;
  mode?: ExecutionMode | null;
  tags?: string[];
  timeout_ms?: number | null;
  startup_timeout_ms?: number | null;
  policy?: ServerPolicy;
  description?: string | null;
  pre_launch?: HookStep[];
  source?: SourceDef | null;
}

export interface CommandDef {
  run: string[];
  cwd?: string | null;
  env?: Record<string, EnvValue>;
  timeout_ms?: number | null;
  description?: string | null;
}

export interface Profile {
  servers?: string[];
  description?: string | null;
}

export interface Registry {
  schema_version: number;
  settings?: Settings;
  commands?: Record<string, CommandDef>;
  servers?: Record<string, ServerDef>;
  profiles?: Record<string, Profile>;
  active_profile?: string | null;
}

export interface RegistryView {
  registry: Registry;
  path: string;
  exists: boolean;
}

export interface AppInfo {
  cerberus_version: string;
  gui_version: string;
  config_dir: string;
  registry_path: string;
  data_dir: string;
  registry_exists: boolean;
}

export interface ValidateReport {
  ok: boolean;
  error: string | null;
}

export interface Diagnostic {
  severity: Severity;
  area: string;
  message: string;
}

export interface ClientDiagnostic {
  client: string;
  display_name: string;
  status: string;
  detected: boolean;
  supported: boolean;
  servers: number;
  notes?: string[];
}

export interface DoctorReport {
  config_path: string;
  config_valid: boolean;
  schema_version: number | null;
  server_count: number;
  clients: ClientDiagnostic[];
  diagnostics: Diagnostic[];
  warnings: number;
  critical: number;
}

export interface StartOptions {
  bind: string;
  allowRemote: boolean;
  token?: string | null;
  profile?: string | null;
  watch: boolean;
  watchInterval?: number | null;
}

export interface GatewayStatus {
  running: boolean;
  managed: boolean;
  pid: number | null;
  transport: string | null;
  bind: string | null;
  watch: boolean | null;
  startedUnix: number | null;
  lastEvent: string | null;
}
