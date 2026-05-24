import { invoke } from "@tauri-apps/api/core";
import type {
  AttemptDetail,
  AttemptId,
  CancelMode,
  ExecDetail,
  ExecRollup,
  ExecSummary,
  ExecutionId,
  FailedPageQuery,
  FailedRowPage,
  RowHistory,
  RunHandle,
  RunStatus,
  Settings,
  Workspace,
} from "./types";

export const ipc = {
  workspace_open: (args: { path: string | null }) =>
    invoke<Workspace>("workspace_open", args),
  workspace_current: () => invoke<Workspace | null>("workspace_current"),
  exec_list: () => invoke<ExecSummary[]>("exec_list"),
  workspace_settings_load: () => invoke<Settings>("workspace_settings_load"),
  workspace_settings_save: (args: { settings: Settings }) =>
    invoke<void>("workspace_settings_save", args),
  exec_show: (args: { id: ExecutionId }) => invoke<ExecDetail>("exec_show", args),
  attempt_show: (args: { executionId: ExecutionId; attemptId: AttemptId }) =>
    invoke<AttemptDetail>("attempt_show", args),
  exec_rollup: (args: { id: ExecutionId }) => invoke<ExecRollup>("exec_rollup", args),
  attempt_failed_page: (args: { query: FailedPageQuery }) =>
    invoke<FailedRowPage>("attempt_failed_page", args),
  attempt_row_history: (args: { executionId: ExecutionId; seq: number }) =>
    invoke<RowHistory>("attempt_row_history", args),
  run_start: (args: { executionId: ExecutionId; handlerDir: string }) =>
    invoke<RunHandle>("run_start", { executionId: args.executionId, handlerDir: args.handlerDir }),
  run_cancel: (args: { handle: RunHandle; mode: CancelMode }) =>
    invoke<void>("run_cancel", args),
  run_status: (args: { handle: RunHandle }) =>
    invoke<RunStatus>("run_status", args),
  run_active: () =>
    invoke<RunHandle[]>("run_active"),
};
