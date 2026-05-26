// Hand-written mirrors of rowforge-studio-core public types.
// Keep in sync until Plan 3 introduces auto-gen via specta or
// tauri-specta. Cross-reference: `crates/rowforge-studio-core/src/*.rs`.

export interface Workspace {
  root: string;
  schema_version: number;
}

export interface ExecSummary {
  id: string;
  name: string;
  created_at: string; // ISO 8601 UTC
  input_rows: number | null;
  size_bytes: number | null; // Plan 10: total bytes of execution data on disk
  attempts_count: number;
  last_attempt_state: string | null;
  last_attempt_counts: AttemptCountsStub | null;
  last_handler_dir: string | null;
}

export interface AttemptCountsStub {
  success: number;
  failed: number;
  crashed: number;
}

export interface Settings {
  schema_version: number;
  workspace_root: string | null;
  /**
   * Workspace-scoped concurrency limit enforced at SessionRegistry. Reads at
   * workspace_open; surfaced via the "Will apply on next workspace open"
   * banner in the Settings form. Default 3 when null (spec §3.4).
   */
  max_concurrent_runs: number | null;
  telemetry_opt_in: boolean;
  /**
   * Plan 7 T15: preferred editor command for handler_open_editor.
   * When non-null, overrides $VISUAL / $EDITOR / auto-detect fallback chain.
   * The value is shell-split at call time ("code --wait" works).
   * null means fall through to the 4-tier resolver.
   */
  preferred_editor: string | null;
  /**
   * Plan 9 T5: when true, valid outcome JSON stdout lines are duplicated
   * into handler_log.log (in addition to outcomes.jsonl). Default false.
   * Turn on to debug protocol issues; leave off for normal operation.
   */
  handler_log_capture_raw_stdout: boolean;
}

// ===== Plan 10 exec delete =====

export interface ExecDeleteFailure {
  exec_id: string;
  reason: string;
}

export interface ExecDeleteBulkResult {
  deleted: string[];
  failed: ExecDeleteFailure[];
}

export type UiErrorKind =
  | "workspace_locked"
  | "not_found"
  | "invalid_arg"
  | "io"
  | "internal"
  | "run_aborted"
  | "run_busy"
  | "unknown_handle"
  | "invalid_input"
  | "duplicate_exec_name"
  | "export_incomplete"
  | "manifest_invalid"
  | "toolchain_missing"
  | "editor_not_found"
  | "handler_not_found"
  | "handler_exists"
  | "invalid_handler_name"
  | "build_failed"
  | "no_build_command"
  | "execution_in_use";

// Adjacently-tagged serde: #[serde(tag = "kind", content = "message")].
// JSON shapes (confirmed by ipc_contract tests):
//   tuple-String variant: { "kind": "workspace_locked", "message": "no home" }
//   struct variant:       { "kind": "duplicate_exec_name",
//                           "message": { "name": "foo" } }
//   #[serde(flatten)] on RunAborted.reason: the AbortReason JSON sits
//   directly under "message":
//                        { "kind": "run_aborted",
//                          "message": { "kind": "user_cancelled" } }
//
// NOTE: Plan 1 originally used #[serde(tag = "kind")] (internal tagging) which
// panics at runtime for newtype variants wrapping primitives. Fixed in Task 11
// to use adjacent tagging; the inner field is "message", not "0".
export type UiError =
  | { kind: "workspace_locked"; message: string }
  | { kind: "not_found"; message: string }
  | { kind: "invalid_arg"; message: string }
  | { kind: "io"; message: string }
  | { kind: "internal"; message: string }
  | { kind: "unknown_handle"; message: string }
  | { kind: "run_aborted"; message: AbortReason }
  | {
      kind: "run_busy";
      message: { execution_id: string; limit: number; scope: BusyScope };
    }
  | { kind: "invalid_input"; message: { reason: string } }
  | { kind: "duplicate_exec_name"; message: { name: string } }
  | { kind: "export_incomplete"; message: { missing_count: number } }
  | { kind: "manifest_invalid"; message: { errors: ManifestError[] } }
  // Plan 7 used { token: string }; Plan 8 reworked to { name: string; tool: string }.
  | { kind: "toolchain_missing"; message: { name: string; tool: string } | null }
  // EditorNotFound: unit variant. serde adjacent tagging emits message: null
  // (verified by editor_not_found_serializes test in error.rs).
  | { kind: "editor_not_found"; message: null }
  | { kind: "handler_not_found"; message: { name: string } }
  | { kind: "handler_exists"; message: { name: string } }
  | { kind: "invalid_handler_name"; message: { name: string } }
  // Plan 8 build variants.
  | { kind: "build_failed"; message: { name: string; exit_code: number } | null }
  | { kind: "no_build_command"; message: { name: string } | null }
  // Plan 10: returned when trying to delete an execution that has an active run.
  | { kind: "execution_in_use"; message: { exec_id: string } | null }
  // Plan 13: returned when trying to smoke-run a handler that already has an active run.
  | { kind: "handler_busy"; message: { name: string } | null };

function isUiError(e: unknown): e is UiError {
  return !!e && typeof e === "object" && "kind" in e && "message" in e;
}

export function uiErrorMessage(e: unknown): string {
  if (!isUiError(e)) return String(e);
  switch (e.kind) {
    // tuple-String variants — render the message verbatim.
    case "workspace_locked":
    case "not_found":
    case "invalid_arg":
    case "io":
    case "internal":
    case "unknown_handle":
      return `[${e.kind}] ${e.message}`;

    // struct variants — render the typed payload.
    case "run_aborted":
      return `[run_aborted] ${e.message.kind}`;
    case "run_busy":
      return `[run_busy] ${e.message.scope} limit ${e.message.limit} reached`;
    case "invalid_input":
      return `[invalid_input] ${e.message.reason}`;
    case "duplicate_exec_name":
      return `[duplicate_exec_name] '${e.message.name}' already exists in this workspace`;
    case "export_incomplete":
      return `[export_incomplete] ${e.message.missing_count} row(s) unresolved`;
    case "manifest_invalid":
      return `[manifest_invalid] ${e.message.errors.length} error(s)`;
    case "toolchain_missing":
      return `Build tool "${e.message?.tool ?? "?"}" not found in PATH. Install it or update entry.build in your manifest.`;
    case "editor_not_found":
      return `[editor_not_found] No editor found — set Settings.preferred_editor, or VISUAL/EDITOR env, or install code/cursor/nvim/vim/nano`;
    case "handler_not_found":
      return `[handler_not_found] '${e.message.name}' is not under <workspace>/handlers/`;
    case "handler_exists":
      return `[handler_exists] '${e.message.name}' already exists`;
    case "invalid_handler_name":
      return `[invalid_handler_name] '${e.message.name}' must match ^[a-z0-9][a-z0-9-]*$`;
    // Plan 8 build variants.
    case "build_failed":
      return `Build failed for "${e.message?.name ?? "handler"}" (exit ${e.message?.exit_code ?? "?"}). See the Last build section for details.`;
    case "no_build_command":
      return `Handler "${e.message?.name ?? "?"}" has no entry.build command in rowforge.yaml.`;
    case "execution_in_use":
      return `Execution "${e.message?.exec_id ?? "?"}" has an active run. Cancel the run first.`;
    case "handler_busy":
      return `Handler "${e.message?.name ?? "?"}" has an active run. Cancel the run first.`;
  }
}

export type ExecutionId = string;
export type AttemptId = string;

export interface ExecDetail {
  summary: ExecSummary;
  input_path_snapshot: string;
  input_format: "csv" | "jsonl" | "ndjson";
  handler_binding: { handler_id: string | null; handler_instance_id: string | null; version: string | null };
  attempts: AttemptSummary[];
  field_mapping: { fields: Record<string, string> } | null;
  config_overrides: Record<string, unknown>;
}

export interface AttemptSummary {
  id: AttemptId;
  state: string;
  started_at: string;
  finished_at: string | null;
  run_type: string;
  stats: AttemptCountsStub | null;
}

export interface AttemptDetail {
  id: AttemptId;
  execution_id: ExecutionId;
  state: string;
  run_type: string;
  started_at: string;
  finished_at: string | null;
  stats: AttemptCountsStub;
  by_error_code: Record<string, number>;
  handler_instance: { id: string | null; handler_id: string | null; version: string | null };
  paths: { meta_json: string; outcomes_jsonl: string; handler_stderr_log: string };
  is_terminal: boolean;
}

export interface ExecRollup {
  resolved: number;
  failed_last: number;
  crashed_last: number;
  cancelled_last: number;
  too_large: number;
  never_attempted: number;
  by_error_code: Record<string, number>;
}

export type RowOutcomeKind = "error" | "crash" | "too_large";

export interface FailedPageQuery {
  execution_id: ExecutionId;
  attempt_id: AttemptId;
  offset: number;
  limit: number;
  error_code_filter: string | null;
}

export interface FailedRowPage {
  rows: FailedRow[];
  next_offset: number | null;
  total_known: number | null;
}

export interface FailedRow {
  seq: number;
  // NOTE: no row_index — outcomes.jsonl doesn't carry it per T11 finding.
  kind: RowOutcomeKind;
  error_code: string | null;
  message: string | null;
  raw_record: unknown;
  dur_ms: number;
}

export interface RowHistory {
  seq: number;
  rows: Array<[AttemptId, RowOutcomeKind, string | null]>;
  resolved_at: AttemptId | null;
}

// ===== Plan 4: Run lifecycle + live progress =====

export type RunHandle = string;

/// Returned by `run_start`. Carries both the run handle and the attempt id
/// created synchronously, enabling direct navigation to the attempt's Live tab.
export interface RunStartedHandle {
  handle: RunHandle;
  attempt_id: string;
}

export type RunStatus =
  | "starting"
  | "running"
  | "cancelling"
  | "done"
  | "aborted"
  | "crashed";

export type CancelMode = "soft" | "hard";

export type Phase =
  | "initializing"
  | "snapshotting"
  | "starting"
  | "running"
  | "cancelling"
  | "persisting";

export interface RunReport {
  processed: number;
  success: number;
  failed: number;
  crashed: number;
  dur_ms: number;
}

export interface WorkerCrashRecord {
  worker_id: number;
  last_seq: number | null;
  exit_code: number | null;
  signal: number | null;
  stderr_tail: string;
}

// AbortReason is internally tagged on "kind".
export type AbortReason =
  | { kind: "user_cancelled" }
  | { kind: "handler_startup_timeout"; failed_workers: number; last_stderr: string }
  | { kind: "all_workers_crashed"; crashes: WorkerCrashRecord[] }
  | { kind: "stalled"; silent_secs: number; last_seq: number | null }
  | { kind: "missing_required_input"; columns: string[] }
  | { kind: "snapshot_hash_mismatch"; path: string; expected: string; actual: string }
  | { kind: "orphaned_on_restart" }
  | { kind: "crashed"; panic_message: string }
  | { kind: "internal"; message: string };

// ProgressEvent: adjacent-tag-like via "type" field. Tuple-variant
// payloads (WorkerCrashed, Done) are INLINED at top level (verified
// by Plan 4 T2 contract test for WorkerCrashed).
export type ProgressEvent =
  | { type: "phase_changed"; phase: Phase; at_ms: number }
  | { type: "worker_spawned"; worker_id: number }
  | { type: "handler_ready"; worker_id: number; handler_version: string; startup_ms: number }
  // WorkerCrashed payload is inlined:
  | ({ type: "worker_crashed" } & WorkerCrashRecord)
  | { type: "stall_warning"; silent_secs: number }
  | {
      type: "tick";
      seq: number;
      at_ms: number;
      processed: number;
      total: number | null;
      success: number;
      failed: number;
      crashed: number;
      in_flight: number;
      queue_depth: number;
      rate_1s: number;
      rate_10s: number;
      eta_ms: number | null;
    }
  | {
      type: "outcome_sample";
      row_index: number;
      kind: RowOutcomeKind;
      code: string | null;
      message: string | null;
      dur_ms: number;
    }
  | {
      type: "batch_summary";
      first_seq: number;
      n: number;
      success: number;
      failed: number;
      dur_ms: number;
    }
  | { type: "pipeline_warning"; code: string; message: string }
  | { type: "handler_stderr"; worker_id: number; line: string }
  // Done payload (RunReport) is inlined:
  | ({ type: "done" } & RunReport)
  | {
      type: "aborted";
      reason: AbortReason;
      at_phase: Phase;
      partial_report: RunReport;
    };

export interface RunRollupTick {
  active_runs: number;
  total_processed: number;
  total_failed: number;
  total_rate: number;
  slowest_run: RunHandle | null;
}

// ===== Plan 5 mirrors =====

export interface StartExecArgs {
  input_path: string;
  name: string;
  csv_id: string | null;
  pinned_handler_instance: string | null;
}

export type ExportFormat = "csv" | "jsonl" | "both";

export interface ExportOpts {
  output_dir: string | null;
  format: ExportFormat;
  require_complete: boolean;
}

export interface ExportWarning {
  code: string;
  message: string;
}

export interface ExportReport {
  output_dir: string;
  written_files: string[];
  success_count: number;
  failed_count: number;
  warnings: ExportWarning[];
}

export type ManifestSource = { type: "path"; path: string };

export interface Manifest {
  name: string;
  version: string;
  language: string;
  entry_cmd: string[];
  entry_build: string[] | null;
}

export type ManifestError =
  | { kind: "manifest_missing"; path: string }
  | { kind: "parse_failed"; message: string };

export type ManifestWarning =
  | { kind: "path_lookup_failed"; field: string; token: string };

export interface ManifestReport {
  manifest: Manifest | null;
  errors: ManifestError[];
  warnings: ManifestWarning[];
}

export type BusyScope = "per_exec" | "per_workspace";

/**
 * Counter snapshot mirrored from rowforge-studio-core::aggregator::ProgressSnapshot.
 * Used by useRun to bootstrap state on mount — Tauri events are
 * fire-and-forget, so a listener that attaches after the run started
 * misses earlier events. Calling `run_snapshot` fills them back in.
 */
export interface ProgressSnapshot {
  processed: number;
  total: number | null;
  success: number;
  failed: number;
  crashed: number;
  in_flight: number;
  queue_depth: number;
  phase: Phase | null;
  /** Plan 6 T5: sliding-window 10s rate. 0 while still warming up. */
  rate_10s: number;
}

// ===== Plan 8 build outcome =====

/** Mirror of rowforge-studio-core::build::BuildOutcome. */
export interface BuildOutcome {
  /** ISO 8601 UTC */
  started_at: string;
  /** ISO 8601 UTC */
  finished_at: string;
  exit_code: number;
  command: string[];
  stdout: string;
  stderr: string;
}

// ===== Plan 7 handler authoring =====

export type ManifestStatus = "valid" | "invalid" | "missing";

export type ScaffoldTemplate = "go_stdio" | "go_batch" | "empty";

export interface HandlerSummary {
  name: string;
  path: string;
  manifest_status: ManifestStatus;
  last_modified: string; // ISO 8601 UTC
  version: string | null;
  language: string | null;
}

export interface SourceFileSummary {
  name: string;
  size_bytes: number;
  is_directory: boolean;
}

export interface HandlerDetail {
  summary: HandlerSummary;
  /** Parsed manifest (raw rowforge-core shape); null when manifest_status != "valid". */
  manifest: Manifest | null;
  manifest_errors: ManifestError[];
  manifest_warnings: ManifestWarning[];
  source_files: SourceFileSummary[];
  has_fixtures_dir: boolean;
  /** Plan 8: outcome of the most recent handler_build invocation; null if never built. */
  last_build: BuildOutcome | null;
}

export interface ScaffoldArgs {
  name: string;
  template: ScaffoldTemplate;
  primary_field: string;
}

// ===== Plan 9 handler logs =====

export type HandlerStream = "stdout" | "stderr";

export interface HandlerLogLine {
  timestamp: string; // ISO 8601
  worker_id: number;
  stream: HandlerStream;
  line: string;
}

// ===== Plan 13 — handler smoke test =====

export type SmokeOutcomeStatus = "success" | "error" | "crash";

export interface SmokeOutcome {
  seq: number;
  status: SmokeOutcomeStatus;
  code: string | null;
  message: string | null;
  dur_ms: number;
  data: unknown | null;
}

export interface SmokeRunRequest {
  handler_name: string;
  rows: Record<string, unknown>[];
}

export interface SmokeRunResult {
  outcomes: SmokeOutcome[];
  stderr_tail: string;
  exit_code: number | null;
  elapsed_ms: number;
}
