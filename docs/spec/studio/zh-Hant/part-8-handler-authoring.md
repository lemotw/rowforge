# 第 8 部分 — Handler 撰寫

定義 handler 管理面板:使用者如何在 Studio 內探索、編輯、新建、build、
smoke test、刪除 handler 程式。

本部分**取代**第 1 部分 §1.4 與第 5 部分 §5.4 中「留錨點、後續實作」
對 handler 撰寫的立場。v1 現在以下列範圍出貨 handler 撰寫功能。

跨章節引用內嵌;對照表見 §8.7。

## 8.1 目的與範圍

v1 Studio 涵蓋兩個使用者目標(第 1 部分 §1.1):

1. 管理 execution — 第 2–7 部分。
2. **管理 handler 實作 — 本部分。**

### v1 包含
- 從 `<workspace>/handlers/*` 探索 handler(單一來源)。
- 列出、檢視、新建、刪除、改名 handler 資料夾。
- 透過外部編輯器編輯(Studio 啟動;不內建 code editor)。
- Reveal in Finder / Explorer。
- Manifest 驗證列為一級功能。
- 透過 `manifest.build` 命令 build。
- 使用者貼入 input rows(≤ 100)做 smoke test。

### 延後
- Studio 內建 code editor(Monaco / CodeMirror):明確非目標;v1 契約是
  外部編輯器。
- Fixture 檔 / 從 exec 取 smoke test 輸入(§8.9 Q1)。
- 從 Studio 跑 `rowforge pack`(§8.9 Q5)。
- 結構化 manifest 編輯器(寫回磁碟;§8.9 Q6)。
- 跨 workspace 的 handler registry。

## 8.2 Manifest 擴充

`rowforge-core::Manifest::entry` 包含兩個欄位，驅動執行與建置。
CLI 與 Studio 共用此型別。欄位形狀由先前 Plan 建立；
Plan 8 使 `entry.build` 實際執行。

```rust
struct Entry {
    cmd:   Vec<String>,              // 例如 ["./handler"]  或  ["python3", "handler.py"]
    build: Option<Vec<String>>,      // 例如 ["go", "build", "-o", "handler", "./..."]
    // ...其他 entry 欄位...
}
```

語意:

- `entry.build` 為選填。存在時，CLI 與 Studio 以
  `cwd = <handler_dir>` 透過 `std::process::Command` 在 spawn
  `entry.cmd` 前執行之。
- `entry.cmd` 必填。同 `cwd` 語意。
- 每個欄位的第一個 token 走 `PATH` 查找。`entry.build` 第一個
  token 解析不到 → `UiError::ToolchainMissing`。
- 不做 shell 展開 — token 直接傳給 `exec`；無引號處理或 glob 展開。

驗證路徑：`validate_manifest`（位於 `rowforge-studio-core`，
依 Plan 7 細節）擴充為兩個新 `ManifestWarning` 變體（見 §8.4.2）。
PATH 解析失敗為警告，非 error（他機器 `PATH` 不同仍可能跑得起來）。

## 8.3 模型

投影位於 `studio-core`。全部帶 `#[non_exhaustive]`(第 5 部分 §5.7)。

```rust
struct HandlerSummary {
    name: String,                       // handlers/ 下的資料夾名
    path: PathBuf,
    manifest_status: ManifestStatus,    // Valid | Invalid | Missing
    last_modified: DateTime<Utc>,       // 對整個 handler dir 的 max(mtime)
    version: Option<String>,            // manifest.version
    language: Option<String>,           // manifest.language(僅顯示)
}

enum ManifestStatus { Valid, Invalid, Missing }

struct HandlerDetail {
    summary: HandlerSummary,
    manifest: Option<Manifest>,
    manifest_errors: Vec<ManifestError>,
    manifest_warnings: Vec<ManifestWarning>,
    source_files: Vec<SourceFileSummary>,  // 僅頂層
    last_build: Option<BuildOutcome>,       // 記憶體;見 §8.4.7
    has_fixtures_dir: bool,                 // v1.1 錨點(§8.9 Q1)
}

struct SourceFileSummary {
    name: String,
    size_bytes: u64,
    is_directory: bool,
}

/// Plan 8：住在 rowforge-core::build；由 studio-core 重新匯出。
struct BuildOutcome {
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
    exit_code: i32,
    command: Vec<String>,                  // 跑時的 entry.build 副本
    stdout: String,                        // 完整 stdout 捕獲
    stderr: String,                        // 完整 stderr 捕獲
}

struct SmokeTestArgs {
    handler_name: String,
    rows: Vec<JsonValue>,                  // 使用者貼;v1 上限 100
    timeout_secs: u32,                     // 預設 30, 上限 300
    skip_build: bool,                      // 預設 false
}

struct SmokeTestReport {
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
    build_skipped: bool,
    build_failed: bool,                    // true 時 outcomes 為空
    outcomes: Vec<RowOutcome>,             // len == args.rows.len()
    stderr_tail: String,                   // ≤ 64 KiB
    handler_version: Option<String>,       // 來自 handshake
}

struct ScaffoldArgs {
    name: String,                          // ^[a-z0-9][a-z0-9-]*$
    template: ScaffoldTemplate,
    primary_field: String,                 // ^[a-zA-Z_][a-zA-Z0-9_]*$ — 範例預期的輸入欄位名
}
enum ScaffoldTemplate { GoStdio, GoBatch, Empty }
```

**Scaffold 欄位驗證：**
- `name` 必須符合 `^[a-z0-9][a-z0-9-]*$` — 由伺服器端 `handler_scaffold`
  與 `handler_rename` 強制執行；失敗時 emit `InvalidHandlerName`。
- `primary_field` 必須為合法識別碼：`^[a-zA-Z_][a-zA-Z0-9_]*$`
  （字母、數字、底線；不可以數字開頭）— 由伺服器端 `handler_scaffold`
  強制執行；失敗時 emit `InvalidArg`。此限制防止腳手架檔案中的
  YAML/Go 注入。

成本級別(第 2 部分 §2.1):

- `HandlerSummary` 清單:**warm**(目錄掃描 + 每個 manifest 讀;以 mtime
  probe 快取,同 `ExecSummary`)。
- `HandlerDetail`:**warm**。
- `BuildOutcome`、`SmokeTestReport`:**hot** 記憶體內;不跨 Studio
  重啟持久化(§8.9 Q3/Q4)。

## 8.4 Runtime

### 8.4.1 編輯器啟動

`handler_open_editor(name)` 解析外部編輯器,順序:

1. `Settings.preferred_editor`(§8.6.4)。
2. `$VISUAL`、然後 `$EDITOR`。
3. 在 `PATH` 中探測 `code`、`cursor`、`nvim`、`vim`、`nano`。
4. 失敗 → `UiError::EditorNotFound`。

選定的命令以 handler 資料夾作為唯一引數,detached 模式 spawn。Studio
不等編輯器退出、不追蹤其生命週期。

`handler_reveal(name)` 用 Tauri `shell::open(handler_dir)`,交給 OS 檔案
管理員處理。

### 8.4.2 Build 生命週期

```
Pending → Building → BuildSucceeded
                  ↘ BuildFailed
```

Build 從呼叫端角度是同步的。CLI 在主執行緒執行；Studio 的 Tauri
command 宣告為 `async`，但目前未使用 `spawn_blocking` — 非同步
runtime 在建置期間會被阻塞。（重構標記為後續項目；典型建置在數秒內
完成。）

v1 無中途取消。完整 `stdout` + `stderr` 捕獲後以 `BuildOutcome` 回傳。

`needs_build`（呼叫端過期檢查，由 CLI 使用）：
- `entry.build` 為 `None` 時回傳 `false`。
- `entry.cmd[0]` 為絕對路徑或 PATH 可解析的裸名（直譯器情境：無
  binary 概念）時回傳 `false`。
- 其他情況將 `entry.cmd[0]` 視為 `handler_dir` 中的相對 binary。
  binary 不存在，或頂層原始碼最大 mtime（`.go .rs .py .js .ts .mjs
  .java .c .cpp .h .hpp`）超過 binary mtime 時回傳 `true`。

CLI `exec run` 在 spawn workers 前遵循 `needs_build`；建置失敗時
CLI 以 exit code 2 結束。CLI `handler build` 子命令以失敗數（上限
125）作為 exit code。Studio 點擊 Build 按鈕時永遠強制（不做過期
檢查）。

終止狀態寫入記憶體內 `BuildOutcome` 快取
（`StudioCore.build_cache: Mutex<HashMap<String, BuildOutcome>>`），
每 handler 保留至 Studio 重啟。

**驗證器警告**（`validate_manifest` 在 `rowforge-studio-core`）：
- `BuildToolNotInPath { tool }` — `entry.build` 第一個 token 不在
  `PATH` 中。
- `CmdTargetMissing { path }` — `entry.cmd` 第一個 token 為磁碟上
  不存在的相對路徑。`entry.build` 為 `Some` 時抑制此警告（建置
  步驟預期會產出該 binary）。

### 8.4.3 Smoke test 生命週期

> **延後自 Plan 8** — 見設計文件 §10。Smoke test 將在後續 Plan 落地。

```
Pending → (Building →) Handshaking → Running → Done
                                             ↘ Aborted
                                             ↘ TimedOut
                                             ↘ BuildFailed
```

流水線:

1. 若 `manifest.build` 存在且 `args.skip_build = false`,先跑 build。
   Build 失敗:`BuildFailed`、outcomes 空、`build_failed = true`、停止。
2. 以 `cwd = handler_dir` spawn `manifest.run`。
3. 標準 rowforge handshake。失敗:`Aborted { reason: HandshakeFailed }`。
4. 對 `args.rows` 每列寫一個 JSON-Lines payload 到 stdin、等一個 outcome
   從 stdout。整體 wall-clock 受 `timeout_secs` 限制。
5. 最後一列後送 EOF,等 ≤ 2 秒優雅退出。否則 force-kill,回報 `TimedOut`。

取消:3 秒軟、然後 hard kill。

### 8.4.4 並發

| 限制 | 預設 | 顯現為 |
|---|---|---|
| 同 handler 的 build | 1 | `HandlerBusy { reason: BuildInFlight }` |
| 同 handler 的 smoke | 1 | `HandlerBusy { reason: SmokeInFlight }` |
| 同 handler 的 build + smoke | 互斥 | smoke 自動 build;並發 build 拒絕 |
| Workspace 內 smoke 總數 | 2 | `HandlerBusy { reason: WorkspaceLimit }` |
| 同 handler 有 exec run 在跑時的 build/smoke | 拒絕 | `HandlerBusy { reason: ExecRunInFlight }` |

### 8.4.5 與 exec-run 的互鎖

> **延後自 Plan 8** — 見設計文件 §10。Smoke test 與 exec-run 互鎖
> 將在後續 Plan 落地。

exec run 持有 handler 期間(第 3 部分),Studio 拒絕對同 handler 名做
build / smoke,避免中途重寫 binary。對稱地,Run launcher(Part 7 §7.3)
拒絕在有 build / smoke 進行中的 handler 上啟動 exec run。互鎖住在
`SessionRegistry`(第 5 部分 §5.2 註解),為雙向真相來源。

### 8.4.6 Scaffold 模板

v1 僅出與既有 example handlers(`examples/handlers/`)相符的模板:

- `GoStdio` — 單列 stdio handler。對應 `golang-apple-refund`。
- `GoBatch` — batch handler。對應 `golang-billing-channel`。
- `Empty` — 僅 `manifest.json` + 空 source dir。

Scaffold 寫到 `<workspace>/handlers/<name>/`。資料夾已存在 →
`UiError::HandlerScaffoldConflict { name }`。

模板於 v1 烘焙在 Studio binary 內。未來模板來源(registry、URL)未設計
也未錨定 — 值得自己 brainstorm。

### 8.4.7 關閉時清理

Studio 退出時(第 3 部分 §3.6):

1. Active build / smoke 子進程軟取消,1 秒期限,然後 hard kill。
2. 記憶體內的 `BuildOutcome` / `SmokeTestReport` 丟棄。重啟後 UI 不得
   顯示陳舊的「last build」。

## 8.5 API

> **Plan 7 已出貨。** §8.5.1–§8.5.3 所有項目均已落地。已落地檔案路徑：
>
> - `crates/rowforge-studio-core/src/handler.rs` — 模組主體
>   （`handler_list`、`handler_show`、`handler_open_editor`、`handler_reveal`、
>   `handler_scaffold`、`handler_delete`、`handler_rename`、`resolve_editor`）
> - `crates/rowforge-studio-core/src/handler_templates/` — 內嵌 scaffold
>   模板（GoStdio、GoBatch、Empty）
> - `crates/rowforge-studio-core/src/error.rs` — `UiError` 變體，含 Plan 7
>   新增項目
> - `apps/rowforge-studio/src-tauri/src/commands.rs` — 7 個新 command 的
>   Tauri command shell
> - `apps/rowforge-studio/src/ipc/types.ts` — TypeScript 映射
> - `apps/rowforge-studio/src/ipc/use-handlers.ts` — TanStack Query hooks
> - `apps/rowforge-studio/src/pages/HandlersPage.tsx`
> - `apps/rowforge-studio/src/pages/HandlerDetailPage.tsx`
> - `apps/rowforge-studio/src/components/ScaffoldDialog.tsx`
> - `apps/rowforge-studio/src/components/RenameHandlerDialog.tsx`
> - `apps/rowforge-studio/src/components/DeleteHandlerDialog.tsx`

### 8.5.1 `StudioCore` 新增

```rust
impl StudioCore {
    pub fn handler_list(&self) -> Result<Vec<HandlerSummary>, UiError>;
    pub fn handler_show(&self, name: &str) -> Result<HandlerDetail, UiError>;
    pub fn handler_open_editor(&self, name: &str) -> Result<(), UiError>;
    pub fn handler_reveal(&self, name: &str) -> Result<(), UiError>;

    /// Plan 8：同步；將 outcome 快取至 build_cache 供 handler_show 使用。
    pub fn handler_build(&self, name: &str) -> Result<BuildOutcome, UiError>;

    // 延後至後續 Plan：
    pub fn handler_smoke_test(&self, args: SmokeTestArgs)
        -> Result<SmokeTestHandle, UiError>;
    pub fn handler_cancel_build(&self, h: &BuildHandle, mode: CancelMode)
        -> Result<(), UiError>;
    pub fn handler_cancel_smoke(&self, h: &SmokeTestHandle, mode: CancelMode)
        -> Result<(), UiError>;
    pub fn handler_subscribe_build(&self, h: &BuildHandle)
        -> Result<BuildStream, UiError>;
    pub fn handler_subscribe_smoke(&self, h: &SmokeTestHandle)
        -> Result<SmokeStream, UiError>;

    pub fn handler_scaffold(&self, args: ScaffoldArgs) -> Result<String, UiError>;
    pub fn handler_delete(&self, name: &str) -> Result<(), UiError>;
    pub fn handler_rename(&self, old: &str, new: &str) -> Result<(), UiError>;
}
```

`StudioCore.build_cache: Mutex<HashMap<String, BuildOutcome>>` — 每 session
的記憶體內儲存；`handler_show` 將快取的 outcome 注入 `HandlerDetail.last_build`。
Studio 重啟後遺失（§8.4.7）。

`BuildHandle` 與 `SmokeTestHandle` 是不透明 ID,類比 `RunHandle`(Part 5
§5.2)。兩種獨立 handle 型別,讓型別系統排除交叉取消。（僅供延後的
smoke-test 路徑使用。）

### 8.5.2 事件

```
handler:build:<name>          BuildEvent
handler:smoke:<name>          SmokeEvent
handlers:list                 ()                      // 粗粒度 refresh 提示
```

```rust
enum BuildEvent {
    Started { command: String, at_ms: u64 },
    StderrLine { line: String, at_ms: u64 },
    Done { exit_code: i32, dur_ms: u32, stderr_tail: String },
    Cancelled,
}

enum SmokeEvent {
    BuildPhase(BuildEvent),
    Handshake { handler_version: Option<String>, dur_ms: u32 },
    Outcome { row_index: u32, outcome: RowOutcome },
    Done(SmokeTestReport),
    Aborted { reason: SmokeAbortReason },
    TimedOut { row_index: Option<u32>, elapsed_ms: u32 },
}

enum SmokeAbortReason {
    UserCancelled,
    HandshakeFailed { stderr_tail: String },
    HandlerCrashed { stderr_tail: String, signal: Option<i32> },
    BuildFailed,
    Internal { message: String },
}
```

Smoke test **不**套用第 6 部分 §6.2 的 4 Hz / 20 Hz 合併預算,因 N ≤ 100,
每個 outcome 都發送。

`StderrLine` 事件套用每 handler 20 行/秒的 token bucket(第 6 部分 §6.2
模式),避免雜訊 build 打爆 broadcast channel。

### 8.5.3 Tauri commands

> **Plan 7 已出貨：** `handler_list`、`handler_show`、`handler_open_editor`、
> `handler_reveal`、`handler_scaffold`、`handler_delete`、`handler_rename`。
>
> **Plan 8 新增：** `handler_build`。

```
handler_list()                              -> Vec<HandlerSummary>
handler_show(name)                          -> HandlerDetail
handler_open_editor(name)                   -> ()
handler_reveal(name)                        -> ()
handler_build(name: String)                 -> BuildOutcome     // Plan 8
handler_smoke_test(args)                    -> SmokeTestHandle  // 延後
handler_cancel_build(handle, mode)          -> ()               // 延後
handler_cancel_smoke(handle, mode)          -> ()               // 延後
handler_scaffold(args)                      -> String
handler_delete(name)                        -> ()
handler_rename(old, new)                    -> ()
```

`handler_build` 副作用：建置後（成功或失敗皆然）emit `handlers:list`
事件，讓 `HandlerSummary.last_modified` 能偵測到新 binary 的 mtime。

`handler_build` 宣告為 `async` 但目前未使用 `spawn_blocking` — 建置
期間 runtime 被阻塞。已標記為後續重構項目；典型建置在數秒內完成。

### 8.5.4 新 `UiError` 變體

擴充第 5 部分 §5.3:

**Plan 7 變體：**

```rust
EditorNotFound,
HandlerBusy { name: String, reason: HandlerBusyReason },
HandlerScaffoldConflict { name: String },
ToolchainMissing { name: String, tool: String },  // Plan 8 重整 payload
SmokeRowsTooMany { limit: u32 },                   // v1 > 100

enum HandlerBusyReason {
    BuildInFlight,
    SmokeInFlight,
    ExecRunInFlight,
    WorkspaceLimit,
}
```

**Plan 8 變體：**

```rust
/// 建置子進程以非零值結束。
BuildFailed { name: String, exit_code: i32 },

/// entry.build 第一個 token 無法透過 `which` 解析。
ToolchainMissing { name: String, tool: String },

/// 嘗試建置一個 manifest 無 entry.build 的 handler。
NoBuildCommand { name: String },
```

Plan 8 變體說明：

| 變體 | 序列化 `kind` | Payload | 由何 emit | UI 文案 |
|---|---|---|---|---|
| `BuildFailed { name, exit_code }` | `build_failed` | `{ name, exit_code }` | `handler_build` 建置以非零值結束時 | "Build failed for 'NAME' (exit N). See the Last build section for details." |
| `ToolchainMissing { name, tool }` | `toolchain_missing` | `{ name, tool }` | `handler_build` 當 `entry.build[0]` 不在 `PATH` 時 | "Build tool 'TOOL' not found in PATH. Install it or update entry.build in your manifest." |
| `NoBuildCommand { name }` | `no_build_command` | `{ name }` | `handler_build` 當 manifest 無 `entry.build` 時 | "Handler 'NAME' has no entry.build command in rowforge.yaml." |

全部帶 `#[non_exhaustive]`(第 5 部分 §5.7)。

## 8.6 UI(擴充第 7 部分)

> **Plan 7 已出貨。** `/handlers` 與 `/handlers/:name` 均為已上線路由。
> IA 更新見第 7 部分 §7.3；scaffold/rename/delete user flow 見第 7 部分
> §7.4 Flow H–J。

Part 7 §7.3 的 Sidebar / shell 其他不變。**Authoring** 群組不再 disabled。

### 8.6.1 IA 新增

- Sidebar `AUTHORING / ● Handlers` 變為可用（Plan 7：已出貨）。
- 路由（Plan 7：全部已上線）:
  - `/handlers` — Handler 清單（`HandlersPage.tsx`）。
  - `/handlers/:name` — Handler 詳情（`HandlerDetailPage.tsx`）。Tabs:**Source**(檔案列表)、
    **Manifest**(驗證報告)、**Smoke test**、**Build log**。
  - `/handlers/new` — Scaffold wizard(modal-as-route)。
- Run launcher(Part 7 §7.3):`HandlerSource` picker 變為從
  `handler_list()` 來的下拉。仍保留「Browse external folder…」作 fallback。
  內部仍構造 `HandlerSource::Dir`(Part 5 §5.4 錨點不變)。

### 8.6.2 主要 flow

**Flow E — 編輯既有 handler**

| # | 步驟 | Command |
|---|---|---|
| 1 | Sidebar → Handlers | `handler_list` |
| 2 | 列 → `[Edit]` | `handler_open_editor(name)` |
| 3 | 外部編輯器開啟;Studio 顯示 toast | — |
| 4 | 儲存後 → Smoke test tab → 貼列 → `[Run smoke]` | `handler_smoke_test` |
| 5 | 訂閱 `handler:smoke:<name>` | event |

**Flow F — 新建 handler(scaffold)**

| # | 步驟 | Command |
|---|---|---|
| 1 | Handlers → `[+ New handler]` | — |
| 2 | Wizard:名稱 + 模板 + primary 欄位 | — |
| 3 | 提交 | `handler_scaffold` |
| 4 | 路由到 `/handlers/:name`;提示「Click Edit to start」 | `handler_show` |

**Flow G — Build + smoke test**

| # | 步驟 | Command |
|---|---|---|
| 1 | Detail → Smoke test tab → 貼 JSON 列 | — |
| 2 | `[Run smoke]` | `handler_smoke_test` |
| 3 | UI:Build phase log → Handshake → 每列 outcome | events |
| 4 | 失敗 → 右側 Sheet 顯示 stderr tail | — |

### 8.6.3 邊界狀態(擴充 Part 7 §7.7)

| # | 狀態 | 觸發 | 顯示 |
|---|---|---|---|
| H1 | 空 `handlers/` | `handler_list → []` | Empty state + `[+ New handler]` + 「Handlers live in `<workspace>/handlers/*`」 |
| H2 | Manifest 缺失 | 資料夾沒有 `manifest.json` | 列上 `⚠ no manifest` 標籤;Smoke / Build disabled |
| H3 | Manifest invalid | `manifest_errors` 非空 | inline 紅標;Manifest tab 列錯誤 |
| H4 | `EditorNotFound` | 找不到編輯器 | Toast + 「Set $EDITOR or install `code` CLI」+ Reveal-in-Finder fallback |
| H5 | `HandlerBusy` | Build/smoke 或 exec-run 鎖 | inline disabled 按鈕 + tooltip 指出哪個鎖 |
| H6 | `ToolchainMissing` | `manifest.build` 第一字不在 PATH | Modal 指出缺的命令 + 安裝提示 |
| H7 | Smoke timeout | 超過 `timeout_secs` | banner + 「Retry with longer timeout」 |
| H8 | `HandlerScaffoldConflict` | 名稱已存在 | Wizard inline error;submit disabled |
| H9 | `SmokeRowsTooMany` | 貼超過 100 列 | inline error + 顯示計數 |
| H10 | 編輯器已開、未確認儲存 | 永遠 | 軟提示「Saved your edits? Smoke test below」(不阻塞) |

### 8.6.4 設定新增

擴充第 2 部分 §2.2.9 與第 5 部分 §5.6:

```rust
struct Settings {
    // ... 既有
    preferred_editor: Option<String>,              // 例如 "code"、"cursor"  [Plan 7：已出貨]
    smoke_test_default_timeout_secs: Option<u32>,  // 預設 30               [延後]
}
```

> **實作修正（Plan 7）：** `preferred_editor` 以容忍 reader 方式加入，
> 未升 `schema_version`。原設計描述由 1 升至 2；Plan 7 維持版號 1。
> 以第 2 部分 §2.2.9 的說明為準；§8.6.4 保留原設計文字以供參照。
> `smoke_test_default_timeout_secs` 為延後項目，Plan 7 未出貨。

### 8.6.5 線框圖(示意)

ASCII;同 Part 7 §7.13 caveat。

#### W-H1 Handler 清單

```
┌──────────────────────────────────────────────────────────────────────────────────────────────┐
│  Handlers                                                                  [+ New handler]   │
├──────────────────────────────────────────────────────────────────────────────────────────────┤
│  ┌────────────────────────────────────────────────────────────────────────────────────────┐  │
│  │ Name                          Lang   Version   Manifest    Modified                    │  │
│  ├────────────────────────────────────────────────────────────────────────────────────────┤  │
│  │ golang-apple-refund           go     0.1.0     ✓ valid     2026-05-22 09:14   [Edit] ⏵│  │
│  │ golang-billing-channel        go     0.1.0     ✓ valid     2026-05-21 17:02   [Edit] ⏵│  │
│  │ golang-refund-backfill        go     0.1.0     ✓ valid     2026-05-21 11:30   [Edit] ⏵│  │
│  │ scratchpad                    go     —         ⚠ missing   2026-05-22 12:01   [Edit] ⏵│  │
│  └────────────────────────────────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────────────────────────────────┘
```

#### W-H2 Handler 詳情 — Smoke test tab

```
┌──────────────────────────────────────────────────────────────────────────────────────────────┐
│  Handlers / golang-billing-channel                          [Edit] [Reveal] [Delete]         │
├──────────────────────────────────────────────────────────────────────────────────────────────┤
│  ┌─Source──Manifest──Smoke test──Build log────────────────────────────────────────────────┐ │
│  │                                                                                         │ │
│  │  Input rows (paste JSON, one per line; max 100)                                         │ │
│  │  ┌─────────────────────────────────────────────────────────────────────────────────┐   │ │
│  │  │ {"billid":"b0001"}                                                              │   │ │
│  │  │ {"billid":"b0042"}                                                              │   │ │
│  │  │ {"billid":""}                                                                   │   │ │
│  │  └─────────────────────────────────────────────────────────────────────────────────┘   │ │
│  │  Timeout: [30] s         [ ] Skip build                                                 │ │
│  │                                                                            [ Run smoke ]│ │
│  │                                                                                         │ │
│  │  Last run · 2026-05-22 14:08 · 1.2 s                                                    │ │
│  │  ┌─Outcomes──────────────────────────────────────────────────────────────────────────┐ │ │
│  │  │ row 0   ● success   {"billid":"b0001","channel":"alipay"}                  142 ms │ │ │
│  │  │ row 1   ● error     BILLING_NOT_FOUND                                       11 ms │ │ │
│  │  │ row 2   ● error     MISSING_BILLID                                           2 ms │ │ │
│  │  └────────────────────────────────────────────────────────────────────────────────────┘│ │
│  │  stderr (tail) · [Open full log]                                                        │ │
│  └─────────────────────────────────────────────────────────────────────────────────────────┘│
└──────────────────────────────────────────────────────────────────────────────────────────────┘
```

## 8.7 跨章節對照

| §8.x | 依據 |
|---|---|
| 8.1 範圍 | 第 1 部分 §1.1、§1.4(被取代);第 5 部分 §5.4(錨點實現) |
| 8.2 manifest | 第 4 部分 §4.6 schema 版本;第 5 部分 §5.4 |
| 8.3 模型 | 第 2 部分 §2.1 成本級別;§2.4 投影契約 |
| 8.4 runtime | 第 3 部分 §3.5 取消模式(較短閾值);§3.6 清理;§3.4 並發 |
| 8.4.5 互鎖 | 第 5 部分 §5.2 SessionRegistry |
| 8.5 API | 第 5 部分 §5.2、§5.3 錯誤、§5.5 commands、§5.7 穩定性 |
| 8.5.2 事件 | 第 6 部分 §6.1 分類;§6.2(註明 smoke test 不合併) |
| 8.6 UI | 第 7 部分 §7.3 IA;§7.7 邊界狀態;§7.13 線框慣例 |
| 8.6.4 設定 | 第 2 部分 §2.2.9;第 5 部分 §5.6 |

## 8.8 UI 不得做的事(handler 專屬)

擴充第 7 部分 §7.10:

1. **不可內建 code editor。** 僅外部編輯器(§8.4.1)。
2. **Scaffold 不可靜默覆寫。** 衝突顯示為 `HandlerScaffoldConflict`。
3. **同 handler 有 exec run 時不可 build / smoke。** 互鎖 §8.4.5。
4. **Smoke test 事件不可合併。** 每個 outcome 必須渲染(§8.5.2)。
5. **`BuildOutcome` / `SmokeTestReport` 不可跨重啟持久化。** v1
   記憶體內；Studio 重啟後 UI 不得顯示陳舊「last build」。

## 8.9 開放問題

1. **Fixture 檔 / 從 exec 取 smoke 輸入。** v1 僅貼上;大 fixture 使用者
   會撞到 100 列上限。v1.1 候選(§8.1 已延後)。
2. **Studio 內 diff 檢視。** 外部編輯器存檔後,「自上次 build 以來變了
   什麼?」是否有用?還是編輯器自帶的 diff 已夠?
3. **Smoke test 歷史寫磁碟。** 每 handler 持久化最後 N 份報告,讓重啟
   後仍保留除錯情境。
4. **`BuildOutcome` 寫磁碟。** 同 Q3。
5. **從 Studio 跑 `rowforge pack`。** 目前 CLI 限定。
6. **Manifest 寫回 / 結構化編輯器。** 原 `ManifestSource::Draft` 錨點
   (第 5 部分 §5.4)是為此設計;需要真正的編輯器介面才划得來。
