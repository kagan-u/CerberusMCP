import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppInfo,
  CommandDef,
  DoctorReport,
  GatewayStatus,
  Profile,
  Registry,
  RegistryView,
  ServerDef,
  Settings,
  StartOptions,
  ValidateReport,
} from "./types";

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  getRegistry: () => invoke<RegistryView>("get_registry"),
  initRegistry: () => invoke<RegistryView>("init_registry"),
  validate: () => invoke<ValidateReport>("validate_registry"),
  runDoctor: (probeServers: boolean) =>
    invoke<DoctorReport>("run_doctor", { probeServers }),

  upsertServer: (name: string, server: ServerDef) =>
    invoke<RegistryView>("upsert_server", { name, server }),
  deleteServer: (name: string) =>
    invoke<RegistryView>("delete_server", { name }),
  toggleServer: (name: string, enabled: boolean) =>
    invoke<RegistryView>("toggle_server", { name, enabled }),

  upsertProfile: (name: string, profile: Profile) =>
    invoke<RegistryView>("upsert_profile", { name, profile }),
  deleteProfile: (name: string) =>
    invoke<RegistryView>("delete_profile", { name }),
  setActiveProfile: (name: string | null) =>
    invoke<RegistryView>("set_active_profile", { name }),
  setSettings: (settings: Settings) =>
    invoke<RegistryView>("set_settings", { settings }),
  setCommands: (commands: Record<string, CommandDef>) =>
    invoke<RegistryView>("set_commands", { commands }),

  getLogs: (limit: number) => invoke<string[]>("get_logs", { limit }),
  clearLogs: () => invoke<void>("clear_logs"),

  gatewayStart: (opts: StartOptions) =>
    invoke<GatewayStatus>("gateway_start", { opts }),
  gatewayStop: () => invoke<GatewayStatus>("gateway_stop"),
  gatewayStatus: () => invoke<GatewayStatus>("gateway_status"),
};

/** Subscribe to the live log stream; resolves to the unlisten function. */
export function onLogLine(cb: (line: string) => void): Promise<UnlistenFn> {
  return listen<string>("log-line", (event) => cb(event.payload));
}

export function serversOf(registry: Registry): [string, ServerDef][] {
  return Object.entries(registry.servers ?? {});
}

export function profilesOf(registry: Registry): [string, Profile][] {
  return Object.entries(registry.profiles ?? {});
}
