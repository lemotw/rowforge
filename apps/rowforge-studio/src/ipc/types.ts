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
  attempts_count: number;
  last_attempt_state: string | null;
  last_attempt_counts: AttemptCountsStub | null;
}

export interface AttemptCountsStub {
  success: number;
  failed: number;
  crashed: number;
}

export interface Settings {
  schema_version: number;
  workspace_root: string | null;
  default_workers: number | null;
  max_concurrent_runs: number | null;
  telemetry_opt_in: boolean;
}

export type UiErrorKind =
  | "workspace_locked"
  | "not_found"
  | "invalid_arg"
  | "io"
  | "internal";

// Adjacently-tagged serde: #[serde(tag = "kind", content = "message")].
// JSON shape (confirmed by ipc_contract.rs test in src-tauri/tests/):
//   { "kind": "workspace_locked", "message": "no home" }
// NOTE: Plan 1 originally used #[serde(tag = "kind")] (internal tagging) which
// panics at runtime for newtype variants wrapping primitives. Fixed in Task 11
// to use adjacent tagging; the inner field is "message", not "0".
export interface UiError {
  kind: UiErrorKind;
  message: string;
}

export function uiErrorMessage(e: unknown): string {
  if (e && typeof e === "object" && "kind" in e) {
    const ue = e as UiError;
    return `[${ue.kind}] ${ue.message ?? ""}`;
  }
  return String(e);
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
