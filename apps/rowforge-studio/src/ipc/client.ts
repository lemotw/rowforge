import { invoke } from "@tauri-apps/api/core";
import type {
  AttemptDetail,
  AttemptId,
  BuildOutcome,
  CancelMode,
  ExecDeleteBulkResult,
  ExecDetail,
  ExecRollup,
  ExecSummary,
  ExecutionId,
  ExportOpts,
  ExportReport,
  FailedPageQuery,
  FailedRowPage,
  HandlerDetail,
  HandlerLogLine,
  HandlerSummary,
  ManifestReport,
  ManifestSource,
  ProgressSnapshot,
  RowHistory,
  RunHandle,
  RunStartedHandle,
  RunStatus,
  ScaffoldArgs,
  Settings,
  SmokeRunRequest,
  SmokeRunResult,
  StartExecArgs,
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
  run_start: (args: {
    executionId: ExecutionId;
    handlerDir: string;
    rowLimit?: number | null;
    workers?: number | null;
    dryRun?: boolean | null;
    skipAttempted?: boolean | null;
    onlyRowIds?: number[] | null;
  }) =>
    invoke<RunStartedHandle>("run_start", {
      executionId: args.executionId,
      handlerDir: args.handlerDir,
      rowLimit: args.rowLimit ?? null,
      workers: args.workers ?? null,
      dryRun: args.dryRun ?? null,
      skipAttempted: args.skipAttempted ?? null,
      onlyRowIds: args.onlyRowIds ?? null,
    }),
  run_cancel: (args: { handle: RunHandle; mode: CancelMode }) =>
    invoke<void>("run_cancel", args),
  run_status: (args: { handle: RunHandle }) =>
    invoke<RunStatus>("run_status", args),
  run_active: () =>
    invoke<RunHandle[]>("run_active"),
  run_snapshot: (args: { handle: RunHandle }) =>
    invoke<ProgressSnapshot>("run_snapshot", args),
  attempt_active_handle: (args: { attemptId: AttemptId }) =>
    invoke<RunHandle | null>("attempt_active_handle", args),

  exec_start: (args: StartExecArgs) =>
    invoke<ExecutionId>("exec_start", { args }),

  exec_export: (id: ExecutionId, opts: ExportOpts) =>
    invoke<ExportReport>("exec_export", { id, opts }),

  manifest_validate: (source: ManifestSource) =>
    invoke<ManifestReport>("manifest_validate", { source }),

  // ===== Plan 7 handler authoring =====

  handler_list: () => invoke<HandlerSummary[]>("handler_list"),
  handler_show: (args: { name: string }) =>
    invoke<HandlerDetail>("handler_show", args),
  handler_open_editor: (args: { name: string }) =>
    invoke<void>("handler_open_editor", args),
  handler_reveal: (args: { name: string }) =>
    invoke<void>("handler_reveal", args),
  handler_scaffold: (args: ScaffoldArgs) =>
    invoke<string>("handler_scaffold", { args }),
  handler_delete: (args: { name: string }) =>
    invoke<void>("handler_delete", args),
  handler_rename: (args: { old: string; new: string }) =>
    invoke<void>("handler_rename", args),
  handler_build: (args: { name: string }) =>
    invoke<BuildOutcome>("handler_build", args),

  // ===== Plan 9 handler logs =====

  handler_log_tail: (args: { execId: string; attemptId: string; maxLines?: number }) =>
    invoke<HandlerLogLine[]>("handler_log_tail", args),
  handler_log_subscribe: (args: { execId: string; attemptId: string }) =>
    invoke<void>("handler_log_subscribe", args),
  handler_log_unsubscribe: (args: { attemptId: string }) =>
    invoke<void>("handler_log_unsubscribe", args),

  // ===== Plan 11 rerun failed =====

  attempt_failed_row_ids: (args: { execId: string; attemptId: string }) =>
    invoke<number[]>("attempt_failed_row_ids", args),

  // ===== Plan 10 exec delete =====

  execution_delete: (args: { execId: string }) =>
    invoke<void>("execution_delete", args),
  execution_delete_bulk: (args: { execIds: string[] }) =>
    invoke<ExecDeleteBulkResult>("execution_delete_bulk", args),

  // ===== Plan 12 handler import + fork =====

  handler_import_from_folder: (args: { sourcePath: string; name: string }) =>
    invoke<void>("handler_import_from_folder", args),
  handler_fork: (args: { sourceName: string; newName: string }) =>
    invoke<void>("handler_fork", args),

  // ===== Plan 13 handler smoke test =====

  handler_smoke_run: (args: { request: SmokeRunRequest }) =>
    invoke<SmokeRunResult>("handler_smoke_run", args),
  handler_smoke_load_fixtures: (args: { path: string; limit: number }) =>
    invoke<Record<string, unknown>[]>("handler_smoke_load_fixtures", args),
};
