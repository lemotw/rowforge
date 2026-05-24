import { invoke } from "@tauri-apps/api/core";
import type { ExecSummary, Settings, Workspace } from "./types";

export const ipc = {
  workspace_open: (args: { path: string | null }) =>
    invoke<Workspace>("workspace_open", args),
  exec_list: () => invoke<ExecSummary[]>("exec_list"),
  workspace_settings_load: () => invoke<Settings>("workspace_settings_load"),
  workspace_settings_save: (args: { settings: Settings }) =>
    invoke<void>("workspace_settings_save", args),
};
