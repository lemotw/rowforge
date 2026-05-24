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

export type UiErrorKind = "workspace_unavailable" | "io" | "internal";

// Adjacently-tagged serde: #[serde(tag = "kind", content = "message")].
// JSON shape (confirmed by ipc_contract.rs test in src-tauri/tests/):
//   { "kind": "workspace_unavailable", "message": "no home" }
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
