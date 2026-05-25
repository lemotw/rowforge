# Plan 8 — Handler Build + Validate (active surface, minimum scope)

**Date:** 2026-05-25
**Branch:** `studio-plan-08-build`
**Supersedes/extends:** Plan 7 (handler authoring static surface)

## 1. Purpose

Plan 7 shipped the read-only handler surface (list, show, scaffold, rename, delete, open editor). Plan 8 makes handlers **buildable**: rowforge-core gains a real build executor, the CLI auto-builds before `exec run` (closing today's "binary doesn't exist → ENOENT" pain), and Studio's handler detail page surfaces a Build button + Last build log.

Smoke-test, build cancel, stderr streaming, build / exec-run interlock, and persisted build records are **explicitly deferred** — see §10.

## 2. Scope deltas vs spec part 8

The original `docs/spec/studio/part-8-handler-authoring.md` envisioned a richer surface (top-level `build`/`run` manifest fields, smoke-test pipeline with handshake + outcome routing, stderr streaming, mutual-exclusion interlocks). Plan 8 deliberately diverges:

| Spec part 8 design | Plan 8 reality |
|---|---|
| Top-level `Manifest.build` + `Manifest.run` (breaking change) | Keep existing `entry.cmd: Vec<String>` + `entry.build: Option<Vec<String>>` — **spec updated to match code** |
| Smoke-test lifecycle (handshake → row → outcome) | Deferred to a later plan |
| Build stderr streamed via `handler:build:<name>` event | Sync wait, full output returned once at completion |
| Build cancel (3 s soft → hard kill) | No cancel; build is short-lived, user waits |
| Build / exec-run mutex interlock | No interlock — concurrent build + run on same handler is the user's problem |
| `BuildRecord` kept until restart | Same: in-memory `HashMap<HandlerName, BuildOutcome>` on `StudioCore`, lost on quit |

The §2 row "spec updated to match code" is part of this plan's deliverables.

## 3. Manifest shape (unchanged from current code)

```rust
struct Entry {
    pub cmd: Vec<String>,             // ["./handler"] or ["python3", "handler.py"]
    pub build: Option<Vec<String>>,   // e.g. ["go", "build", "-o", "handler", "."]
    pub startup_timeout_ms: Option<u64>,
}

struct Manifest {
    pub name: String,
    pub version: Option<String>,
    pub language: Option<String>,
    pub kind: HandlerKind,            // Row | Batch
    pub primary_field: String,
    pub entry: Entry,
    pub runtime: Option<Runtime>,
    pub output: Option<Output>,
    // ... unchanged
}
```

No new fields. No serde changes. CLI and Studio share the type via `rowforge_core::Manifest`.

## 4. Build module (`rowforge-core::build`)

New module `crates/rowforge-core/src/build.rs`. Two public functions + two value types.

### 4.1 Types

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct BuildOutcome {
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub exit_code: i32,           // 0 = success
    pub command: Vec<String>,     // copy of entry.build at run time
    pub stdout: String,           // captured (utf-8, lossy)
    pub stderr: String,           // captured (utf-8, lossy)
}

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("no build command in manifest")]
    NoBuildCommand,
    #[error("build tool {tool:?} not found in PATH")]
    ToolchainMissing { tool: String },
    #[error("build failed (exit {exit_code}): {stderr_tail}")]
    BuildFailed { exit_code: i32, stderr_tail: String, outcome: BuildOutcome },
    #[error("io: {0}")]
    Io(String),
}
```

`BuildOutcome` is the success-path-and-also-non-zero-exit envelope — see `run_build` return.

### 4.2 Staleness check

```rust
pub fn needs_build(handler_dir: &Path, manifest: &Manifest) -> bool;
```

Decision matrix on `entry.cmd[0]`:

| First token | Decision |
|---|---|
| `entry.build` is `None` | `false` (no build command, never builds) |
| Token is absolute (`/usr/bin/python3`) | `false` (no relative binary to stale-check) |
| Token resolves via `which::which` (`python3`, `node`) | `false` (interpreter on PATH; nothing to build) |
| Token is relative (`./handler`, `handler`) — treat as binary at `handler_dir.join(token)` | Compare mtime per §4.2.1 |

#### 4.2.1 mtime comparison

If treating as binary:
- Binary path `bin = handler_dir.join(token)` (strip leading `./` if present)
- Binary missing → `true`
- Otherwise: scan handler_dir top-level entries for source files by extension (`.go .rs .py .js .ts .mjs .java .c .cpp .h`); take the max mtime across all source files
- If max_source_mtime > binary_mtime → `true`
- Else → `false`

Top-level only — no recursion into subdirs. Rationale: 99% of single-file handlers; recursion adds latency without clear value for the minimum scope.

### 4.3 Build runner

```rust
pub fn run_build(handler_dir: &Path, manifest: &Manifest) -> Result<BuildOutcome, BuildError>;
```

- `manifest.entry.build` is `None` → `BuildError::NoBuildCommand`
- Resolve `cmd[0]` via `which::which`; failure → `BuildError::ToolchainMissing`
- `std::process::Command::new(cmd[0]).args(&cmd[1..]).current_dir(handler_dir)`
- Capture stdout + stderr with `Stdio::piped()`; sync `.output()` (blocks current thread)
- Build the `BuildOutcome` with timestamps
- If exit_code != 0 → `BuildError::BuildFailed { exit_code, stderr_tail: last 500 chars, outcome }`
- Else → `Ok(outcome)`

Sync block is deliberate. Callers (CLI exec run, studio handler_build) decide threading:
- CLI runs on the main thread — fine, build is short
- Studio runs inside `async_runtime::spawn_blocking` to keep tokio reactor free

## 5. Manifest validation extension

Extend `validate_manifest` (`crates/rowforge-core/src/manifest.rs` or wherever it lives — verify location) with two new warning emitters:

```rust
pub struct ManifestWarning {
    pub code: ManifestWarningCode,
    pub message: String,
}

pub enum ManifestWarningCode {
    BuildToolNotInPath,   // NEW
    CmdTargetMissing,     // NEW
    // ...existing variants...
}
```

Logic:
- If `entry.build` is `Some(v)` and `which::which(v[0])` fails → push `BuildToolNotInPath`
- For `entry.cmd[0]` (call it `t`):
  - If `t` starts with `/` or contains `/` (relative path) → check `handler_dir.join(strip_leading_dot_slash(t)).exists()`. If missing AND `entry.build` is None → push `CmdTargetMissing`. If missing AND `entry.build` is Some → no warning (build is expected to produce it).
  - Else (bare name): `which::which(t)` failure → push `CmdTargetMissing`.

Both are warnings (not errors). Consumers of `HandlerDetail.manifest_warnings` already render warnings yellow per Plan 7 UI.

## 6. CLI integration

### 6.1 `exec run` auto-build gate

Find the spawn path in `crates/rowforge-cli/src/exec_cmd.rs` (likely `pool_streaming` invocation site or the function feeding it). Before the first worker spawns:

```rust
if rowforge_core::build::needs_build(handler_dir, manifest) {
    eprintln!("[rowforge] building {} ...", manifest.name);
    match rowforge_core::build::run_build(handler_dir, manifest) {
        Ok(outcome) => {
            eprintln!("[rowforge] build ok ({} ms)", outcome_ms(&outcome));
        }
        Err(BuildError::BuildFailed { exit_code, outcome, .. }) => {
            eprintln!("[rowforge] build failed (exit {}):", exit_code);
            eprintln!("{}", outcome.stderr);
            std::process::exit(2);
        }
        Err(BuildError::ToolchainMissing { tool }) => {
            eprintln!("[rowforge] build tool not found: {}", tool);
            std::process::exit(2);
        }
        Err(BuildError::NoBuildCommand) => unreachable!(), // needs_build returned false in that case
        Err(BuildError::Io(e)) => {
            eprintln!("[rowforge] build io error: {}", e);
            std::process::exit(2);
        }
    }
}
```

Same gate applies to `exec start` if it spawns directly. Confirm during T3 by reading the call sites.

### 6.2 New `rowforge handler build [name]` subcommand

```
USAGE:
  rowforge handler build               # builds every <workspace>/handlers/* with entry.build
  rowforge handler build <name>        # builds one
  rowforge handler build --force <name> # rebuild even if not stale
```

Default: respects `needs_build` gate. `--force` bypasses staleness. Useful for CI / pre-flight.

Output: one line per handler `[name] ok (NNN ms)` or `[name] failed (exit N)` + stderr dumped to terminal on failure. Exit code = number of failed handlers (cap at 125).

## 7. studio-core integration

### 7.1 New error variants

In `crates/rowforge-studio-core/src/error.rs`:

```rust
pub enum UiError {
    // ...
    BuildFailed { name: String, exit_code: i32 },
    ToolchainMissing { name: String, tool: String },
    NoBuildCommand { name: String },     // attempted Build on handler without entry.build
}
```

### 7.2 Build cache

`StudioCore` gains:

```rust
struct StudioCore {
    // ...
    build_cache: Mutex<HashMap<String /* handler name */, BuildOutcome>>,
}
```

In-memory only. Cleared on Drop. Restart loses all entries.

### 7.3 Public methods

In `crates/rowforge-studio-core/src/handler.rs` (or new module file):

```rust
pub fn build(workspace_root: &Path, name: &str) -> Result<BuildOutcome, UiError>;
```

- Resolve handler dir + manifest via existing `show()` machinery
- If manifest invalid / missing → propagate UiError
- If `entry.build` is `None` → `UiError::NoBuildCommand { name }`
- Call `rowforge_core::build::run_build` (always — Studio always forces, unlike CLI's needs_build gate)
- Map `BuildError::BuildFailed` → `UiError::BuildFailed { name, exit_code }` (but ALSO populate cache with the failure outcome — the UI wants to show the failed log)
- Map `BuildError::ToolchainMissing` → `UiError::ToolchainMissing { name, tool }`
- Update cache regardless of pass/fail

On `StudioCore`:

```rust
pub fn handler_build(&self, name: &str) -> Result<BuildOutcome, UiError> {
    let outcome_result = handler::build(self.workspace.root.as_path(), name);
    if let Ok(ref outcome) = outcome_result {
        self.build_cache.lock().unwrap().insert(name.to_string(), outcome.clone());
    }
    // For BuildFailed we ALSO want to cache the outcome (so detail page shows the log).
    // Adjust handler::build to return the outcome via the error variant or via a side
    // channel; concretely: pass &self.build_cache to handler::build so it can write
    // before returning the error. Implementation detail — pick the cleanest of:
    //   (a) handler::build takes &mut HashMap and writes itself
    //   (b) UiError::BuildFailed carries the outcome
    //   (c) StudioCore::handler_build catches BuildError directly
    // Recommend (c): expose a lower-level handler::build_raw -> Result<BuildOutcome, BuildError>
    // and let handler_build do cache + error mapping.
    outcome_result
}
```

`handler_show` reads the cache and writes into `HandlerDetail.last_build`:

```rust
pub struct HandlerDetail {
    // ...existing fields...
    pub last_build: Option<BuildOutcome>,    // NEW — None if no build ever attempted this session
}
```

## 8. Tauri shell

`apps/rowforge-studio/src-tauri/src/commands.rs`:

```rust
#[tauri::command]
pub async fn handler_build(
    state: State<'_, AppState>,
    name: String,
) -> Result<BuildOutcome, UiError> {
    let core_arc = state.core.clone();
    tokio::task::spawn_blocking(move || {
        let guard = core_arc.lock().unwrap_or_else(|p| p.into_inner());
        let core = guard.as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
        core.handler_build(&name)
    })
    .await
    .map_err(|e| UiError::Io(format!("spawn_blocking join: {}", e)))?
}
```

> Tauri arg shape, AppState wiring, error envelope — match existing async commands. Pattern from Plan 5 export commands is a good template.

Emit `handlers:list` event after successful build (so HandlerList query — which carries last_modified per Plan 7 round-2 — picks up the new binary's mtime). Optional; nice-to-have.

Register in `invoke_handler![...]`.

## 9. React UI

### 9.1 TS mirrors

`apps/rowforge-studio/src/ipc/types.ts`:

```ts
export interface BuildOutcome {
  started_at: string;       // ISO 8601
  finished_at: string;
  exit_code: number;
  command: string[];
  stdout: string;
  stderr: string;
}

export interface HandlerDetail {
  // ...existing...
  last_build: BuildOutcome | null;
}

// UiError union gains:
export type UiError =
  | ...existing...
  | { kind: "build_failed"; message: string; data: { name: string; exit_code: number } }
  | { kind: "toolchain_missing"; message: string; data: { name: string; tool: string } }
  | { kind: "no_build_command"; message: string; data: { name: string } };
```

`uiErrorMessage` gains arms for the new variants — friendly copy:
- `build_failed` → "Build failed (exit N). See the Last build section for details."
- `toolchain_missing` → "Build tool 'go' not found in PATH. Install it or update entry.build in your manifest."
- `no_build_command` → "This handler has no entry.build command. Add one to rowforge.yaml first."

### 9.2 Hook

`apps/rowforge-studio/src/ipc/use-handlers.ts`:

```ts
export const useHandlerBuild = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { name: string }) => ipc.handler_build(args),
    onSuccess: (_data, vars) => {
      qc.invalidateQueries({ queryKey: ["handler_show", vars.name] });
      qc.invalidateQueries({ queryKey: ["handler_list"] });
    },
  });
};
```

`ipc.handler_build({ name })` returns `BuildOutcome` even on success path; mutation `error` carries the UiError on failure.

### 9.3 `LastBuildSection` component

`apps/rowforge-studio/src/components/LastBuildSection.tsx`:

```tsx
interface Props {
  last_build: BuildOutcome | null;
  pending: boolean;
}

export function LastBuildSection({ last_build, pending }: Props) {
  if (pending) {
    return (
      <Section title="Last build">
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <Spinner /> Building…
        </div>
      </Section>
    );
  }
  if (!last_build) return null;

  const success = last_build.exit_code === 0;
  const [open, setOpen] = useState(false);
  const durationMs = new Date(last_build.finished_at).getTime() - new Date(last_build.started_at).getTime();

  return (
    <Section title="Last build">
      <div className="flex items-center gap-3">
        <StatusBadge success={success} />
        <span className="text-sm text-muted-foreground">
          exit {last_build.exit_code} · {durationMs} ms · {new Date(last_build.finished_at).toLocaleTimeString()}
        </span>
      </div>
      <button onClick={() => setOpen(v => !v)} className="text-xs text-blue-400 hover:underline">
        {open ? "Hide output ▴" : "Show output ▾"}
      </button>
      {open && (
        <pre className="max-h-64 overflow-auto rounded border border-zinc-700 bg-zinc-900 p-2 text-xs font-mono whitespace-pre-wrap">
          {last_build.stdout}
          {last_build.stderr && "\n--- stderr ---\n"}
          {last_build.stderr}
        </pre>
      )}
    </Section>
  );
}
```

### 9.4 `HandlerDetailPage` integration

Add Build button to the header action row (between Open in editor and Rename…). Hide when `manifest?.entry?.build` is null/undefined.

```tsx
const build = useHandlerBuild();
// ...
{manifest?.entry?.build && (
  <Button
    onClick={() => build.mutate({ name })}
    disabled={build.isPending}
  >
    {build.isPending ? "Building…" : "Build"}
  </Button>
)}
```

Render `<LastBuildSection last_build={data.last_build} pending={build.isPending} />` between Manifest section and Files section.

## 10. Out of scope (explicit)

- Smoke test (the entire §8.4.3 spec lifecycle)
- Build cancel
- stderr streaming during build (we wait + dump)
- Build / exec-run mutex interlock
- Persisting BuildOutcome across Studio restart
- Multi-handler parallel build in CLI (`rowforge handler build` builds sequentially)

## 11. Testing

| Suite | Adds | Notes |
|---|---|---|
| rowforge-core | ~10 | `needs_build` matrix (5), `run_build` paths (3), `validate_manifest` warnings (2) |
| rowforge-cli | ~3 | `exec run` auto-build path, `handler build` subcommand |
| rowforge-studio-core | ~4 | `handler::build` happy + failure + cache invariants + UiError mapping |
| studio-shell (ipc_contract) | ~1 | new command registered + JSON shape |
| vitest | ~7 | LastBuildSection 4 states + HandlerDetailPage Build button visibility + useHandlerBuild invalidation |

Targets:
- cargo: 309 → ~325 (+16)
- vitest: 110 → ~117 (+7)

## 12. Spec doc updates (en + zh-Hant)

In Plan 8 deliverables (T10):

- `docs/spec/studio/part-8-handler-authoring.md`:
  - §8.2 rewritten: manifest uses `entry.cmd` + `entry.build` (not top-level `build`/`run`)
  - §8.3 `HandlerDetail.last_build: Option<BuildOutcome>` documented; `BuildRecord` renamed `BuildOutcome` for consistency with code
  - §8.4.2 simplified: sync build, no cancel, no stderr stream
  - §8.4.3 marked deferred (smoke test)
  - §8.4.5 marked deferred (interlock)
  - §8.5.3 lists new `handler_build` command
  - §8.5.4 lists new UiError variants
  - §8.6.5 W-H1 doesn't change; new W-H2 wireframe replaces smoke-test variant
- `docs/spec/studio/part-2-model.md`: no changes
- `docs/spec/studio/part-5-api.md`: `BuildOutcome` + `BuildError` listed; `handler_build` command in §5.5; new UiError variants in §5.3
- All en updates mirrored in `docs/spec/studio/zh-Hant/`

## 13. Acceptance criteria

1. `cargo build && cargo test` clean
2. `cd apps/rowforge-studio && pnpm tsc -b && pnpm test && pnpm build` clean
3. `rowforge exec run --handler examples/handlers/golang-stats-refund-records ...` on a fresh checkout (no pre-built binary) automatically builds and runs without ENOENT
4. `rowforge handler build` builds every handler in the workspace; non-zero exit if any failed
5. Studio Handler detail page shows Build button for handlers with `entry.build`, hides it otherwise
6. Build success: Last build section shows green badge, exit 0, expandable log
7. Build failure: Last build section shows red badge, non-zero exit, expandable log; toast surfaces UiError copy
8. Cache persists during session; restart loses it
9. validate_manifest emits new BuildToolNotInPath / CmdTargetMissing warnings appropriately
10. HUMAN_SMOKE Plan 08 walkthrough added

## 14. Open questions

None at design time. Implementation may surface clarifications worth resolving inline.
