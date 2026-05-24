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

// Plan 1 used tuple variants under #[serde(tag = "kind")]. The exact JSON
// shape for tuple variants with internal tagging may be { "kind": "...", "0": "..." }
// or { "kind": "..." } alone with the inner string lost. Task 11's IPC
// contract test confirms the actual shape; this type may need adjustment
// after that test runs.
export interface UiError {
  kind: UiErrorKind;
  0?: string;
}

export function uiErrorMessage(e: unknown): string {
  if (e && typeof e === "object" && "kind" in e) {
    const ue = e as UiError;
    return `[${ue.kind}] ${ue[0] ?? ""}`;
  }
  return String(e);
}
