# 第 5 部分 — API

定義 `rowforge-studio-core` 的公開介面、其上的 Tauri command 層、
錯誤模型、設定、與版本管理。

## 5.1 Crate 邊界契約

三個 crate，三種職責：

### `rowforge-core`（engine）
擁有：串流流水線、worker pool、handler 協定、SQLite 遷移、所有磁碟
artifact 解析與寫入、`RowResolution` 計算、manifest 驗證、workspace 探索。

以下若今天位於 `rowforge-cli`，作為 v1 一部分上抬至 `rowforge-core`：
- `default_workspace_root()`
- SQLite `open_with_migrations()`
- 僅計數版 `compute_resolution` 入口
- `validate_manifest(source)`（回傳結構化報告）
- `outcomes.jsonl` 逐行迭代作為公開工具

理由：CLI 與 `studio-core` 都是合法消費者。

### `rowforge-studio-core`（僅 GUI 的延伸）
擁有：`UiError`、`SessionRegistry`、`ProgressAggregator`（事件取樣 /
合併）、`ExecRollup` 編排、`Settings` 型別與檔案格式無關的 load/save、
重播 adapter（v2）。

**不**擁有：Tauri 型別、IPC 議題、app-data-dir 解析、視窗事件處理、
manifest schema（從 core 重新匯出）。

### `apps/rowforge-studio`（Tauri 層）
擁有：command 轉譯、`tauri::State<StudioCore>` 生命週期、事件 emit
轉發、設定檔路徑解析（透過 Tauri 的 `app_data_dir`）、啟動接線、
telemetry hook（若日後加入）。

不得繞過 `studio-core` 直接呼叫 `rowforge-core`。

## 5.2 `StudioCore` 公開 API（v1）

```rust
impl StudioCore {
    pub fn open(opts: OpenOpts) -> Result<Self, UiError>;
    pub fn workspace(&self) -> &Workspace;

    // Handler 記錄（Plan 9；見第 3 部分 §3.9）
    pub fn handler_log_tail(&self, exec: &ExecutionId, attempt: &AttemptId, max_lines: Option<usize>)
        -> Result<Vec<HandlerLogLine>, UiError>;
    pub fn handler_log_subscribe(&self, attempt: &AttemptId)
        -> Result<broadcast::Receiver<HandlerLogLine>, UiError>;  // attempt 非 active 時 Err
    pub fn set_handler_log_capture_raw_stdout(&self, v: bool);

    // 讀投影（第 2 部分）
    pub fn list(&self, filter: ListFilter) -> Result<Vec<ExecSummary>, UiError>;
    pub fn show(&self, id: &ExecutionId) -> Result<ExecDetail, UiError>;
    pub fn attempt(&self, e: &ExecutionId, r: &AttemptId)
        -> Result<AttemptDetail, UiError>;
    pub fn rollup(&self, e: &ExecutionId) -> Result<ExecRollup, UiError>;
    pub fn failed_page(&self, q: FailedPageQuery) -> Result<FailedRowPage, UiError>;
    pub fn row_history(&self, e: &ExecutionId, seq: u64)
        -> Result<RowHistory, UiError>;

    // Run 生命週期（第 3 部分 §3.3）
    pub fn start_run(&self, e: &ExecutionId, opts: RunOpts)
        -> Result<RunStartedHandle, UiError>;
    // RunStartedHandle = { handle: RunHandle, attempt_id: String }
    // — 回傳 attempt_id 讓 UI 一次往返就組出
    //   /exec/:id/attempt/:aid?run=<handle> URL。
    pub fn subscribe(&self, h: &RunHandle) -> Result<RunStream, UiError>;
    pub fn cancel(&self, h: &RunHandle, mode: CancelMode) -> Result<(), UiError>;
    pub fn status(&self, h: &RunHandle) -> Result<RunStatus, UiError>;
    pub fn active_runs(&self) -> Vec<RunHandle>;
    pub fn active_runs_stream(&self) -> ActiveRunsStream;  // 第 6 部分 §6.6

    // Execution 生命週期
    pub fn start_exec(&self, args: StartExecArgs) -> Result<ExecutionId, UiError>;
    pub fn export(&self, e: &ExecutionId, opts: ExportOpts)
        -> Result<ExportReport, UiError>;

    // Plan 11 — 重跑失敗的 row
    pub fn attempt_failed_row_ids(&self, exec_id: &ExecutionId, attempt_id: &AttemptId)
        -> Result<Vec<u64>, UiError>;
    // 讀取指定 attempt 的 outcomes.jsonl；從 BatchOutcome 信封中收集
    // 巢狀 outcome type 為 "error" 或 "crash" 的 seq 值。
    // 回傳去重後升序排列的 Vec<u64>。seq 欄位是整個 pipeline 使用的
    // row 識別符（u64）（磁碟欄位名稱：seq）。

    // Execution 刪除（Plan 10；見第 3 部分 §3.10）
    pub fn execution_delete(&self, exec_id: &str) -> Result<(), UiError>;
    pub fn execution_delete_bulk(&self, exec_ids: Vec<String>)
        -> Result<ExecDeleteBulkResult, UiError>;

    // Handler 撰寫的錨點（第 5 部分 §5.4）
    pub fn validate_manifest(&self, source: ManifestSource)
        -> Result<ManifestReport, UiError>;

    // Plan 13 — handler smoke test（見第 8 部分 §8.4.3）
    pub async fn handler_smoke_run(&self, req: SmokeRunRequest)
        -> Result<SmokeRunResult, UiError>;
    pub fn handler_smoke_load_fixtures(&self, path: &Path, limit: usize)
        -> Result<Vec<Map<String, Value>>, UiError>;
    // smoke_run:           短暫派發 N≤100 列；不建立 execution
    // smoke_load_fixtures: 讀取 jsonl/json/csv fixture（1..=100 列）；limit 夾緊 1..=100
}
```

支援型別：

```rust
struct OpenOpts { workspace: Option<PathBuf> }
struct ListFilter { /* v1: 無；保留給未來 */ }
struct RunOpts {
    handler: HandlerSource,
    limit: Option<u64>,
    dry_run: bool,
    workers: Option<u32>,
    force: bool,
    retry_failed: bool,
    config_overrides: BTreeMap<String, JsonValue>,
    mapping: Option<FieldMapping>,
    sync_data: bool,
    only_row_ids: Option<Vec<u64>>,  // Plan 11：若 Some，僅派發這些 seq
}
enum HandlerSource {
    Dir(PathBuf),
    // v2: Sandbox { manifest: ManifestDraft, source_dir: PathBuf },
}
enum CancelMode { Soft, Hard }
struct RunStream {
    handle: RunHandle,
    rx: broadcast::Receiver<ProgressEvent>,
    snapshot: ProgressSnapshot,         // 訂閱時的計數器快照
}
struct StartExecArgs {
    input_path: PathBuf,
    name: String,
    csv_id: Option<String>,
    pinned_handler_instance: Option<HandlerInstanceId>,
}
struct ExportOpts {
    output_dir: Option<PathBuf>,
    format: ExportFormat,               // Csv | Jsonl | Both
    require_complete: bool,
}
enum ManifestSource {
    Path(PathBuf),
    // v2: Draft(ManifestDraft),
}
struct ManifestReport {
    manifest: Manifest,                 // 解析成功時的內容
    errors: Vec<ManifestError>,
    warnings: Vec<ManifestWarning>,
}
```

刻意**不**在 API 內的：
- `raw_outcomes_path(&self, ...)` — 不開繞過投影的逃生口。
- `sql_query(&self, ...)` — 不開直接 SQL 存取。
- `subscribe_all_runs()` 把多 run stream 多工到單一 channel — 會破壞
  每 handle 的事件名隔離（第 6 部分 §6.6）。改用 `active_runs_stream()`,
  其僅為計數聚合。

## 5.3 錯誤模型

```rust
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum UiError {
    #[error("{kind} not found: {id}")]
    NotFound { kind: String, id: String },

    #[error("invalid argument: {0}")]
    InvalidArg(String),

    #[error("handler build failed")]
    HandlerBuildFailed { stderr: String },

    #[error("run aborted: {reason:?}")]
    RunAborted { reason: AbortReason },     // 結構化；見第 6 部分 §6.5

    #[error("handle expired or unknown: {0}")]
    UnknownHandle(String),

    #[error("workspace locked or incompatible: {by}")]
    WorkspaceLocked { by: String },

    #[error("manifest invalid")]
    ManifestInvalid { errors: Vec<ManifestError> },

    #[error("run cannot start: another run is active for {execution_id}")]
    RunBusy { execution_id: String, scope: BusyScope }, // PerExec | Workspace

    #[error("io error: {0}")]
    Io(String),

    #[error("internal: {0}")]
    Internal(String),

    // Plan 7 — handler 管理變體（完整 handler 錯誤集見第 8 部分 §8.5.4）
    #[error("editor not found")]
    EditorNotFound,

    #[error("handler not found: {name}")]
    HandlerNotFound { name: String },

    #[error("handler already exists: {name}")]
    HandlerExists { name: String },

    #[error("invalid handler name: {name}")]
    InvalidHandlerName { name: String },

    // Plan 8 — 建置變體（詳見第 8 部分 §8.5.4）
    #[error("build failed for handler '{name}' (exit {exit_code})")]
    BuildFailed { name: String, exit_code: i32 },

    #[error("build tool '{tool}' for handler '{name}' not found in PATH")]
    ToolchainMissing { name: String, tool: String },

    #[error("handler '{name}' has no entry.build in its manifest")]
    NoBuildCommand { name: String },

    // Plan 10 — 執行刪除
    #[error("execution is in use: {exec_id}")]
    ExecutionInUse { exec_id: String },

    // Plan 13 — handler smoke test
    #[error("handler '{name}' has an active run; cancel it first")]
    HandlerBusy { name: String },
}
```

Plan 7 變體說明：

| 變體 | 序列化 `kind` | payload | 由何 emit | UI 呈現 |
|---|---|---|---|---|
| `EditorNotFound` | `editor_not_found` | 無（`message: null`） | `handler_open_editor`，preferred / `$VISUAL` / `$EDITOR` / 探測全部失敗時 | Toast 或 inline error；文案引導使用者前往 Settings → Editor 或設定 `$VISUAL`/`$EDITOR` |
| `HandlerNotFound { name }` | `handler_not_found` | `{ name }` | `handler_show`、`handler_open_editor`、`handler_reveal`、`handler_delete`、`handler_rename`，目標目錄不存在時 | 詳情頁：「Handler '<name>' not found. It may have been deleted or renamed.」加返回 `/handlers` 連結 |
| `HandlerExists { name }` | `handler_exists` | `{ name }` | `handler_scaffold`（目標目錄已存在）、`handler_rename`（新名稱已被使用） | 對應 dialog 的 inline banner；名稱未修改前 submit 停用 |
| `InvalidHandlerName { name }` | `invalid_handler_name` | `{ name }` | `handler_scaffold`、`handler_rename`，名稱未通過正則 `/^[a-z0-9][a-z0-9-]*$/` 時 | Inline 欄位錯誤；打字時即在前端驗證，後端為最終依據 |
| `InvalidArg(String)` | `invalid_arg` | `{ message }` | `handler_scaffold`，`primary_field` 未通過識別碼正則 `^[a-zA-Z_][a-zA-Z0-9_]*$` 時 | primary_field 欄位 inline 錯誤；防止腳手架檔案中的 YAML/Go 注入 |

Plan 8 變體說明：

| 變體 | 序列化 `kind` | Payload | 由何 emit | UI 呈現 |
|---|---|---|---|---|
| `BuildFailed { name, exit_code }` | `build_failed` | `{ name, exit_code }` | `handler_build` 建置以非零值結束時 | Sonner toast："Build failed for 'NAME' (exit N). See the Last build section for details." |
| `ToolchainMissing { name, tool }` | `toolchain_missing` | `{ name, tool }` | `handler_build` 當 `entry.build[0]` 不在 `PATH` 時 | Toast："Build tool 'TOOL' not found in PATH. Install it or update entry.build in your manifest." |
| `NoBuildCommand { name }` | `no_build_command` | `{ name }` | `handler_build` 當 manifest 無 `entry.build` 時 | Toast："Handler 'NAME' has no entry.build command in rowforge.yaml." |

Plan 10 變體說明：

| 變體 | 序列化 `kind` | Payload | 由何 emit | UI 呈現 |
|---|---|---|---|---|
| `ExecutionInUse { exec_id }` | `execution_in_use` | `{ exec_id }` | `execution_delete` / `execution_delete_bulk`（每項）當 `SessionRegistry::has_active_run_for_exec` 回傳 `true` 時 | ExecList 選取模式中核取方塊停用，提示文字「Cancel active run first」；批量部分失敗時，清單上方顯示黃色警告，列出無法刪除的 exec_id |

Plan 13 變體說明：

| 變體 | 序列化 `kind` | Payload | 由何 emit | UI 呈現 |
|---|---|---|---|---|
| `HandlerBusy { name }` | `handler_busy` | `{ name }` | `handler_smoke_run`，`ExecutionStore::has_active_attempt_for_handler_dir` 回傳 `true` 時 | SmokeSection inline error：「Handler 'NAME' has an active run. Cancel the run first.」 |

組合規則：
- 不提供 blanket `From<anyhow::Error> for UiError`。
- 每個呼叫端自行分類根因並挑選正確變體。
- `From<std::io::Error>` 與 `From<serde_json::Error>` 映射至 `Io`。
- `Internal` 保留給「無法分類」;UI 顯示泛型 toast 並附 copy-details
  按鈕。

## 5.4 Handler 撰寫的延伸介面（錨點）

> **由第 8 部分實現。** Handler 撰寫現已在 v1 範圍。下列錨點仍有效，
> 但其 v2-only 標籤(`Sandbox`、`Draft`)指向仍延後的功能。完整的
> handler API 加在這些錨點之上，見第 8 部分 §8.5。

v1 保留三個錨點，使 handler 撰寫功能落地時不破壞相容：

1. **`HandlerSource` enum** — v1 只有 `Dir(PathBuf)`。v2 將新增
   `Sandbox { manifest: ManifestDraft, source_dir: PathBuf }`,讓煙霧
   測試能對未儲存的草稿執行。

2. **`ManifestSource` enum** — 同形態：v1 `Path(PathBuf)`,v2 加上
   `Draft(ManifestDraft)`。

3. **`validate_manifest`** — v1 為薄包裝,呼叫 `rowforge-core` 既有的
   manifest 驗證器,以結構化 `ManifestReport` 取代 CLI 的文字輸出。
   v2 的編輯器在每次儲存 / 即時呼叫此 API,不需再改 API。

`Manifest`、`ManifestDraft`、`ManifestError`、`ManifestWarning`、
`ManifestSource` 全部住在 `rowforge-core`,由 `studio-core` 重新匯出。

## 5.5 Tauri command 介面

命名為 `noun_verb`、snake_case（Tauri 的 JS binding 自動 camelCase;
我們不設覆寫）。每個 command 直接回傳 `Result<T, UiError>`;v1 不包
`{ data, meta }` 信封。

```
workspace_open(opts)                  -> Workspace
workspace_settings_load()             -> Settings
workspace_settings_save(s)            -> ()

exec_list(filter)                     -> Vec<ExecSummary>
exec_show(id)                         -> ExecDetail
exec_rollup(id)                       -> ExecRollup
exec_start(args)                      -> ExecutionId
exec_export(id, opts)                 -> ExportReport

attempt_show(execution_id, attempt_id)            -> AttemptDetail
attempt_failed_page(query)                        -> FailedRowPage
attempt_row_history(execution_id, seq)            -> RowHistory
attempt_failed_row_ids(execution_id, attempt_id)  -> Vec<u64>
    // Plan 11。讀取 outcomes.jsonl；回傳 BatchOutcome outcome type 為
    // "error" 或 "crash" 的去重升序 seq 值。type 欄位名稱為 "type"
    // （非 "status"）。attempt 無失敗時回傳 []。attempt 不存在時
    // 回傳 NotFound。

run_start(execution_id, handler_dir,
          row_limit?, workers?,
          dry_run?, skip_attempted?,
          only_row_ids?)               -> RunStartedHandle
    // only_row_ids（Option<Vec<u64>>，Plan 11）：若提供，pipeline 僅
    // 派發列出的 seq 值，並對這些 row 略過 skip_seqs。
run_cancel(handle, mode)              -> ()
run_status(handle)                    -> RunStatus
run_active()                          -> Vec<RunHandle>
run_snapshot(handle)                  -> ProgressSnapshot
attempt_active_handle(attempt_id)     -> Option<RunHandle>

manifest_validate(source)             -> ManifestReport

// Plan 7 — handler 管理 commands（完整清單見第 8 部分 §8.5.3）
handler_list()                        -> Vec<HandlerSummary>
handler_show(name)                    -> HandlerDetail
handler_open_editor(name)             -> ()
handler_reveal(name)                  -> ()
handler_scaffold(args)                -> String          // 回傳新 handler 名稱
handler_delete(name)                  -> ()
handler_rename(old, new)              -> ()

// Plan 8 — 建置 command（詳見第 8 部分 §8.5.3）
handler_build(name: String)           -> BuildOutcome    // async；emit handlers:list

// Plan 9 — handler 記錄 commands（見第 3 部分 §3.9）
handler_log_tail(exec_id, attempt_id, max_lines?)   -> Vec<HandlerLogLine>
    // 從 handler_log.log 讀取至多 max_lines（預設 5000）行。
    // 檔案不存在（Plan 9 之前的 attempt）時回傳空 vec。
handler_log_subscribe(exec_id, attempt_id)          -> ()
    // Async。啟動批次泵，以 handler_log:<attempt_id> 事件（100 ms /
    // 64 行批次）發送行。attempt 非 active 時回傳錯誤。
handler_log_unsubscribe(attempt_id)                 -> ()
    // 取消由 handler_log_subscribe 啟動的泵。

// Plan 10 — 執行刪除（見第 3 部分 §3.10）
execution_delete(exec_id)                           -> ()
    // 刪除單一執行。成功時 emit exec_list:refresh。
    // session 正在執行時回傳 ExecutionInUse。
    // 不存在時回傳 NotFound。
execution_delete_bulk(exec_ids)                     -> ExecDeleteBulkResult
    // 串列刪除多個執行。任何成功刪除後 emit exec_list:refresh。
    // 不提早中止；部分失敗回傳至 ExecDeleteBulkResult.failed。

// Plan 12 — handler 從資料夾匯入 + Fork（見第 8 部分 §8.4.6–§8.4.7）
handler_import_from_folder(source_path: String, name: String) -> ()
    // 將 source_path 原封不動複製到 <workspace>/handlers/<name>/。
    // source_path 必須包含 rowforge.yaml；否則以 InvalidArg 拒絕。
    // <name> 已存在時以 HandlerExists 拒絕。名稱不合法時以
    // InvalidHandlerName 拒絕。成功後 emit handlers:list。
    // 不做複製過濾 — .git / node_modules / 建置產物全部複製。
    // 符號連結以 tracing::warn 略過。
handler_fork(source_name: String, new_name: String) -> ()
    // 將 <workspace>/handlers/<source_name>/ 複製為
    // <workspace>/handlers/<new_name>/。透過 serde round-trip 改寫
    // manifest name: 欄位（YAML 註解與鍵排序不保留）。
    // 同名、來源不存在（HandlerNotFound）、目標已存在（HandlerExists）、
    // 新名稱不合法（InvalidHandlerName）時均拒絕。成功後 emit handlers:list。

// Plan 13 — handler smoke test（見第 8 部分 §8.4.3）
handler_smoke_run(request: SmokeRunRequest) -> SmokeRunResult
    // Async。透過 handler binary 派發至多 100 列，不建立 execution。
    // 先跑 Plan 8 build gate（needs_build 為 true 時重建）。
    // 此 handler 有 exec attempt 正在執行時回傳 HandlerBusy（跨進程 sqlite gate）。
    // 不 emit 事件。
handler_smoke_load_fixtures(path: string, limit: number) -> Vec<Map>
    // Sync。從 .jsonl / .ndjson / .json（頂層陣列）/ .csv 檔案或目錄
    // （優先順序：jsonl > ndjson > json > csv）讀取列。
    // limit 夾緊 1..=100。rows 不存在、副檔名不支援、目錄無符合檔案時回傳 InvalidArg。
```

**SmokeRunRequest：** `{ handler_name: string, rows: Map[] }`
**SmokeRunResult：** `{ outcomes: SmokeOutcome[], stderr_tail: string, exit_code: number | null, elapsed_ms: number }`
**SmokeOutcome：** `{ seq, status: "success"|"error"|"crash", code, message, dur_ms, data }`

錯誤：`invalid_handler_name`、`invalid_arg`（rows 為空 / >100 列 / 不支援副檔名 / fixtures 空）、`handler_not_found`、`handler_busy`（此 handler 有 active exec attempt）、`build_failed` / `toolchain_missing` / `no_build_command`（Plan 8 gate）、`io`。

`handler_build` 說明：此 command 宣告為 `async` 但目前在建置期間
阻塞 Tauri async runtime（未使用 `spawn_blocking`）。已標記為後續
重構項目；典型 Go/Rust 建置在 30 秒內完成。

`BuildOutcome` 型別（住在 `rowforge-core::build`）：

```rust
struct BuildOutcome {
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
    exit_code: i32,
    command: Vec<String>,   // 跑時的 entry.build 副本
    stdout: String,
    stderr: String,
}
```

完整型別說明見第 8 部分 §8.3。

Run 生命週期指令說明（Plan 5）：

- `run_start` 回傳 `RunStartedHandle { handle, attempt_id }`，UI
  可以單次往返就組出 `/exec/:id/attempt/:aid?run=<handle>` URL。
- `row_limit` (`Option<u64>`) 上限,限制本次派發 row 數。配合
  `skip_attempted` 可以跨多次 run 累進採樣不重複的 row。
- `skip_attempted` (`Option<bool>`) — true 時計算這個 exec 的
  `RowResolution`，把已 attempt 過的 seq（任何非 `NeverAttempted`
  狀態）當作 `skip_seqs` 傳給 pipeline。UI「sample fresh rows」用。
- `run_snapshot` 回傳該 handle 當前的 `ProgressSnapshot`。React 的
  `useRun` hook 在 `listen()` 掛載後立刻呼叫，補回 listen 起作用前
  已 emit 的 tick（Tauri 事件 fire-and-forget，沒裝 listener 就丟）。
  若 run 已結束 → 回 `UnknownHandle`，React 端視為 fallback 到
  attempt_show 靜態資料。
- `attempt_active_handle` 把 `AttemptId` 解析回對應的活動
  `RunHandle`（若有）。用於使用者不帶 `?run=` URL 進入 in-flight
  attempt 時提供「Watch live」按鈕。
- `only_row_ids`（Plan 11，`Option<Vec<u64>>`）— 若提供，pipeline
  reader 僅派發 seq 值在清單中的 row，並覆蓋任何 `skip_seqs` 過濾。
  TypeScript binding 用 `onlyRowIds`（Tauri 自動 camelCase）。`seq`
  識別符與 `outcomes.jsonl` 信封中使用的 u64 相同（磁碟 JSON 欄位名稱：
  `seq`）。由重跑失敗 dialog 中的 `useRunStart` 提供。

事件（單向,core → UI）：

```
run:<handle>                          ProgressEvent payload
runs:active                           RunRollupTick payload   (第 6 部分 §6.6)
handlers:list                         ()                      // Plan 7：scaffold/delete/rename 後 emit 的粗粒度 refresh 提示；Plan 12 的 handler_import_from_folder + handler_fork 成功後亦 emit
handler_log:<attempt_id>              HandlerLogBatch payload // Plan 9：批次 100 ms / 64 行
exec_list:refresh                     ()                      // Plan 10：任何成功的 execution_delete / execution_delete_bulk 後 emit；React 使 exec_list query 失效
```

`HandlerLogBatch` payload：
```typescript
interface HandlerLogBatch {
  lines: HandlerLogLine[];
  dropped: number;          // 自上批次以來因 broadcast 背壓而丟失的行數
}
```

**`HandlerLogLine` 與 `HandlerStream` 型別（Plan 9）：**

```typescript
type HandlerStream = "stdout" | "stderr";

interface HandlerLogLine {
  timestamp: string;         // RFC 3339，例如 "2026-05-25T14:32:01.423Z"
  worker_id: number;
  stream: HandlerStream;
  line: string;
}
```

Rust 端對應型別：

```rust
// rowforge_core::handler_log
pub enum HandlerStream { Stdout, Stderr }
pub struct HandlerLogLine {
    pub timestamp: DateTime<Utc>,
    pub worker_id: u32,
    pub stream: HandlerStream,
    pub line: String,
}
```

**`ExecDeleteBulkResult` 與 `ExecDeleteFailure` 型別（Plan 10）：**

```typescript
interface ExecDeleteBulkResult {
  deleted: string[];             // 成功刪除的 exec_id 清單
  failed: ExecDeleteFailure[];
}

interface ExecDeleteFailure {
  exec_id: string;
  reason: string;                // 例如 "execution is in use" 或 "not found"
}
```

第 2 部分 §2.2.10 的 TypeScript 鏡像。`useExecutionDeleteBulk` hook
包裝 `execution_delete_bulk`，任何成功刪除（包含 `deleted.length > 0`
的部分成功）後使 `exec_list` query 失效。

## 5.6 設定

- 檔案路徑：`<app_data_dir>/rowforge-studio/settings.json`。
- 格式：JSON,有 schema 版本。
- 型別住在 `studio-core::settings`;路徑解析與 IO 屬於 Tauri 層。
- `studio-core` 暴露 `Settings::load_from(reader)` 與
  `Settings::save_to(writer)`,接 `Read`/`Write`,使自身不涉檔案
  系統政策。

**`max_concurrent_runs` 重載語意：** 此值在 `workspace_open` 時讀取，並以
workspace 範圍限制傳入 `SessionRegistry::new`（第 3 部分 §3.4）。透過
`workspace_settings_save` 更改此值**不會**影響正在運行的 SessionRegistry —
新限制只在下次 `workspace_open` 時生效（發生於 boot autoload 或透過 Settings
頁的「Switch workspace」按鈕）。Settings 頁在表單值與已載入伺服器值不同時，
會顯示「Will apply on next workspace open」提示 banner。

**Plan 13 smoke 設定（新欄位）：**

```rust
struct Settings {
    // ... 現有欄位 ...

    /// smoke run 的預設派發列數（預設 5，夾緊 1..=100）。
    smoke_default_rows: usize,

    /// smoke run 的每列逾時秒數（預設 30；0 = 不逾時 / 1 小時上限）。
    smoke_timeout_per_row_secs: u64,
}
```

兩個欄位在 `workspace_open` 時透過 `OpenOpts` 注入 `StudioCore` 的原子欄位。
透過 `workspace_settings_save` 修改後，下次 `handler_smoke_run` 呼叫即生效（不須重啟）。

## 5.7 版本管理與 API 穩定性

- `rowforge-studio-core` 是**內部** crate;不發布到 crates.io。版本
  隨 app 走。
- `rowforge-core` 以路徑（`{ path = "..." }`）引用;同樹同步發佈。
  core 任何破壞性變更須在同 PR 內更新 studio-core。
- `studio-core` 所有公開 `enum` 帶 `#[non_exhaustive]`。
- `studio-core` 所有可增長欄位的公開 `struct` 帶 `#[non_exhaustive]`。
- API 版本政策：`studio-core` 不對外部程式碼承諾穩定性。Tauri app 與
  `studio-core` 同步發佈。
