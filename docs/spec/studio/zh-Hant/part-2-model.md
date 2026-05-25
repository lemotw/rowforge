# 第 2 部分 — 模型

本部分定義 Studio 暴露給 UI 的實體。每個實體都是磁碟 artifact 的
**投影**；Studio 不會虛構出 CLI 無法重現的欄位。

關於 CLI 端的實體（Execution、Attempt、HandlerInstance、RowResolution
等）見 [`../../cli/part-2-model.md`](../../cli/part-2-model.md)。本部分
引用但不重複那些定義。

## 2.1 實體清單

| 實體 | 來源 | 用途 | 成本級別 |
|---|---|---|---|
| `Workspace` | sqlite 路徑 + 檔案系統根目錄 | 開啟 / 識別 workspace | hot |
| `ExecSummary` | `executions` row + 最新 attempt 的 `meta.json` | 清單列 | warm |
| `ExecDetail` | `executions` row + 所有 `attempts` row + 當前 handler instance | 詳情標頭 | warm |
| `AttemptDetail` | `attempts` row + `meta.json` + handler instance | Attempt 頁 | warm |
| `ExecRollup` | 串流摺疊所有 attempts | 跨 attempt 的解決計數 | cold |
| `FailedRowPage` | 分頁掃描 `outcomes.jsonl` | 失敗列瀏覽器 | cold |
| `RowHistory` | 跨所有 attempts 對單列摺疊 | 「第 N 列發生了什麼？」 | cold, 按需 |
| `RunHandle` | 記憶體 `SessionRegistry` | UI 引用一個在跑的 run | hot |
| `ProgressEvent` | broadcast channel | 即時進度 | hot |
| `Settings` | 設定檔 | 使用者偏好 | hot |

「成本級別」控制快取策略（第 4 部分 §4.3）。

## 2.2 投影型別

具體欄位對 v1 有規範性。

### 2.2.1 `Workspace`
```rust
struct Workspace {
    root: PathBuf,
    schema_version: u8,
}
```

### 2.2.2 `ExecSummary`
```rust
struct ExecSummary {
    id: ExecutionId,
    name: String,
    created_at: DateTime<Utc>,
    input_rows: Option<u64>,             // 輸入尚未快照時為 None
    attempts_count: u32,
    last_attempt_state: Option<AttemptState>,
    last_attempt_counts: Option<AttemptCounts>,   // 只是最後一次 attempt 的 success/failed/crashed
    last_handler_dir: Option<PathBuf>,             // Plan 6：最近一次 run 的 handler 目錄；供 RunButton 預設值使用
}
```
- `last_attempt_counts` 不是跨 attempt 的 rollup。Rollup 是
  `ExecRollup`，採 cold 計算，因為它需要掃描所有 attempts。
- **惰性改名語意（Plan 7）：** `handler_rename` 只執行 `fs::rename`；
  SQLite `executions` 資料表**不**更新。因此，run 啟動時所取得的
  `last_handler_dir` 快照，在改名後仍指向舊目錄名稱。這是刻意的設計：
  handler 以 run 開始時所捕捉的快照作為內容定址基礎，`last_handler_dir`
  為資訊性欄位而非可載入的依據。實際效果：改名後，ExecHistory 中引用該
  handler 的列仍顯示舊名稱；透過新名稱建立的 run 則使用新名稱。
  同樣語意也適用於 `handler_delete` — 刪除 handler 不會從 `executions`
  列中清除過去的 `last_handler_dir` 引用。

### 2.2.3 `ExecDetail`
```rust
struct ExecDetail {
    summary: ExecSummary,
    input_path_snapshot: PathBuf,
    input_format: InputFormat,          // Csv / Jsonl / Ndjson
    handler_binding: HandlerBindingView,
    attempts: Vec<AttemptSummary>,      // 時序排序
    field_mapping: Option<FieldMapping>,
    config_overrides: BTreeMap<String, JsonValue>,
}
```

### 2.2.4 `AttemptDetail`
```rust
struct AttemptDetail {
    id: AttemptId,
    execution_id: ExecutionId,
    state: AttemptState,
    run_type: RunType,                  // 見 cli part-2 §2.4
    started_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    stats: AttemptStats,                // success/failed/crashed/skipped/avg_dur_ms
    by_error_code: BTreeMap<String, u64>,   // 有上限；達 32 時溢位至 "OTHER"
    handler_instance: HandlerInstanceView,
    paths: AttemptPaths,                // outcomes.jsonl, meta.json, stderr.log
}
```

### 2.2.5 `ExecRollup`
```rust
struct ExecRollup {
    resolved: u64,
    failed_last: u64,
    crashed_last: u64,
    too_large: u64,
    never_attempted: u64,
    by_error_code: BTreeMap<String, u64>,
}
```
透過 `compute_resolution`（見 `rowforge-core`）摺疊所有 attempts 的
outcomes 計算得出。Cold — 永遠不快取超過一次 UI 面板渲染的生命週期。
成本隨跨 attempts 的 outcomes 總數線性增加。見第 4 部分 §4.4 的側車
索引計畫以收斂此成本。

### 2.2.6 `FailedRowPage`
```rust
struct FailedPageQuery {
    execution_id: ExecutionId,
    attempt_id: AttemptId,
    offset: u64,
    limit: u32,                         // 上限 500
    error_code_filter: Option<String>,  // v1: 僅 None；v2: 可選
}
struct FailedRowPage {
    rows: Vec<FailedRow>,
    next_offset: Option<u64>,
    total_known: Option<u64>,           // 僅當廉價（已有索引）時填入
}
struct FailedRow {
    seq: u64,
    row_index: u64,
    kind: RowOutcomeKind,               // Error / Crash / TooLarge
    error_code: Option<String>,
    message: Option<String>,
    raw_record: JsonValue,
    dur_ms: u32,
}
```
v1 從 `offset` 線性掃描實作；v2 疊上第 4 部分 §4.4 的索引。

### 2.2.7 `RowHistory`
```rust
struct RowHistory {
    seq: u64,
    rows: Vec<(AttemptId, RowOutcomeKind, Option<String>)>,
    resolved_at: Option<AttemptId>,     // 首個產出 Success 的 attempt
}
```
僅按需；使用者點擊特定列時才開啟。

### 2.2.8 `RunHandle`、`ProgressEvent`、`RunStatus`
狀態機見第 3 部分 §3.3；事件分類見第 6 部分 §6.1。本節摘要視圖：
```rust
struct RunHandle(String);                // 不透明、可序列化、IPC-safe
enum RunStatus { Pending, Starting, Running, Cancelling, Done, Aborted, Crashed }
```

### 2.2.9 `Settings`
```rust
struct Settings {
    schema_version: u8,                  // v1 為 1
    workspace_root: Option<PathBuf>,
    max_concurrent_runs: Option<u32>,    // 預設 3
    telemetry_opt_in: bool,              // 預設 false；v1 不收集
    preferred_editor: Option<String>,    // Plan 7：編輯器命令覆寫，例如 "code"、"cursor"
    #[serde(default)]
    handler_log_capture_raw_stdout: bool, // Plan 9：預設 false；為 true 時，handler stdout
                                          // 中的有效 outcome JSON 行也寫入 handler_log.log
}
```
型別存放於 `studio-core::settings`；load/save 的路徑解析屬於 Tauri 層
（使用 Tauri 的 `app_data_dir`）。

`preferred_editor` 為 `handler_open_editor` 四層編輯器解析器的第一層
（見第 8 部分 §8.4.1）：preferred → `$VISUAL` → `$EDITOR` →
探測 `code`/`cursor`/`nvim`/`vim`/`nano`。儲存於 `settings.json`，與其他
Settings 欄位並列；`schema_version` 維持為 1。透過
`workspace_settings_save` 即時更新至 `StudioCore` — 不需重新開啟
workspace。

`handler_log_capture_raw_stdout` 控制 pool-streaming tee（第 3 部分 §3.9）
是否也將帶有有效 outcome JSON 的 handler stdout 行寫入 `handler_log.log`。
預設 `false`（outcome JSON 量大，記錄入 log 幾乎不提升診斷價值）。
欄位使用 `#[serde(default)]`，因此不含此欄位的現有 `settings.json` 仍可
正確讀取 — 不升 `schema_version`。

> **注意 — 實作修正（Plan 7）：** 第 8 部分 §8.6.4 原先描述此欄位
> 需將 `schema_version` 由 1 升至 2。Plan 7 採容忍 reader 方式加入
> `preferred_editor`，未升版號。以本注意事項為準；§8.6.4 保留原設計
> 文字以供參照。

## 2.3 故意不作為實體

- **HandlerInstance 作為頂層對等實體。** 視為 attempt 的屬性。沒有
  「列出所有 handler instance」的介面。
- **每列 × 每 attempt 矩陣。** 對 100 萬列 × 5 attempts 的執行而言這是
  500 萬格，後續 attempts 中幾乎都是 `NeverAttempted`。改由 `RowHistory`
  按需取得稀疏歷史。
- **原始 `outcomes.jsonl` 路徑逃生口。** UI 不能繞過投影直接透過
  `studio-core` 讀檔。若需要路徑，它存在於 `AttemptDetail::paths`
  以提供「在 Finder 中顯示」之類的功能 — 不供 in-process 解析使用。

## 2.4 投影契約

- 每個投影都 `serde::Serialize`。
- 每個投影都標 `#[non_exhaustive]`，使未來欄位不破壞相容。
- 每個投影都可由磁碟 artifact 與 SQLite registry 計算而得 —
  記憶體中沒有隱藏狀態。
- 投影不暴露 `UiError` 以外的錯誤型別（見第 5 部分 §5.3）。
