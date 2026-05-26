# 第 7 部分 — UI

定義 `rowforge-studio` 的 UI 層：技術棧、設計語言、資訊架構、主要 flow、
狀態到視覺的映射、互動模式、空 / 邊界狀態、載入策略。

本部分對**契約**有強規範性（狀態的色彩 token、UI 不得做的事、push vs
pull 的資料分流），對**視覺**為建議（元件庫、密度、具體 px 值）。
視覺建議的存在是為了 v1 不必再次決策；有理由的偏離可以接受。

對其他 part 的引用內嵌於各節。整合對照表見 §7.12。

## 7.1 技術棧與元件庫

- **Shell：** Tauri（見第 1 部分 §1.3）；webview 內 React。
- **元件庫：** **shadcn/ui + Radix Primitives + Tailwind CSS**。
  - copy-paste 模式 ⇒ 無樹外 breaking change（呼應第 5 部分 §5.7 內部
    crate、同樹同步發版政策）。
  - Radix 在 macOS / Linux / Windows 三平台 webview 上提供完整的鍵盤
    與無障礙原語。
  - Tailwind utility-first 適合高密度的檢視器 UI；與
    `font-variant-numeric: tabular-nums` 與等寬字體搭配自然。
- **虛擬化：** 事件 tail 與失敗列頁皆用 `@tanstack/react-virtual`（§7.6）。
- **圖示：** `lucide-react`。v1 不含插畫資產（§7.7）。

此為建議棧；契約只要求所選棧能提供 Radix 等價的鍵盤原語。未來換 lib
不算破壞規格。

## 7.2 設計原則

1. **資訊密度優先，動畫殿後。** 工具不是 consumer app。動畫只用於
   狀態轉移提示（例如第 3 部分 §3.3 的 `Cancelling → Aborted`），
   不裝飾。
2. **身分欄位一律等寬。** `ExecutionId`、`AttemptId`、`HandlerInstanceId`、
   `worker_id`、`seq`、`row_index`、byte offset、檔案路徑、error code、
   SHA digest、`handler_version`。
3. **語意色 token 全域一致。** 同一個 `RowOutcomeKind` 在進度區、
   事件 tail、失敗列表格、`ExecRollup`、`RowHistory` 用同一個 token。
   使用者不必重學配色。
4. **即時數字用 `tabular-nums` + ≤ 150 ms 過渡。** Tick 驅動的計數器
   （`processed`、`rate_1s`、`rate_10s`、`in_flight`、`queue_depth`、
   `eta_ms`；見第 6 部分 §6.1）不得抖動欄寬。數字以 ease 過渡，
   不直接跳。
5. **暗色為預設，淺色為對等替代。** 長時間執行的批次工具住在暗色 surface 上。
   每個語意 token 提供暗 / 淺雙值。
6. **destructive 動作需顯式摩擦。** Force kill（第 3 部分 §3.5）、
   切 workspace、orphan mark-aborted 一律走 `AlertDialog` 加上顯式
   確認 token（例如輸入 exec 名稱前 4 字），不允許單擊觸發。

## 7.3 資訊架構

### 頁面樹（v1）

- **Workspace Picker / Boot** — 當 `Settings.workspace_root` 為 `None`
  或不可讀時顯示。實體：`Workspace`。呼叫：`workspace_open`、
  `workspace_settings_load`。
- **Workspace Home（Exec list）** — 預設落點。實體：`Vec<ExecSummary>`。
  呼叫：`exec_list`。包含「New execution」CTA 與 header 中的**選取模式**
  切換開關（Plan 10）。欄位順序：[核取方塊（僅選取模式）] | 名稱 | 列數 |
  Attempts | 大小 | 建立時間。名稱欄位以 `title={exec_id}` 顯示提示文字。
  進行中執行的列顯示停用的核取方塊，提示文字為「Cancel active run first」
  （由 `last_attempt_state === "running"` 偵測）。選取 ≥ 1 列後，標頭
  出現「Delete N execution(s)」按鈕，開啟 `DeleteExecutionsDialog`。批量
  刪除部分失敗時，清單上方顯示黃色警告；點擊「Dismiss」可清除。
- **New Execution Wizard**（modal-as-route `/new`）— 實體：
  `StartExecArgs`。呼叫：`manifest_validate`、`exec_start`，可選的
  `run_start`。
- **Execution Detail** `/exec/:id` — 實體：`ExecDetail`。呼叫：
  `exec_show`。Tabs：
  - **Attempts**（預設）— 渲染 `ExecDetail.attempts`。
  - **Rollup** — 實體：`ExecRollup`。呼叫：`exec_rollup`。Cold 載入
    （第 2 部分 §2.2.5、第 4 部分 §4.3）。
  - **Bindings** — 唯讀檢視 `handler_binding`、`field_mapping`、
    `config_overrides`。
  - **404 fallback（Plan 10）：** 當 `exec_show` 回傳 `NotFound`（例如
    該執行已被另一視窗或 CLI 刪除）時，頁面渲染「This execution has been
    deleted or is unavailable.」加上 ← 返回 `/` 的連結，取代正常的
    詳情視圖。延伸現有的 `isError` 分支 — 不需新路由。
- **Attempt Detail** `/exec/:id/attempt/:aid` — 實體：`AttemptDetail`。
  呼叫：`attempt_show`。Sub-tabs：
  - **Live / Summary** — 計數器 + Phase chip bar；attempt 為 active
    run 時訂閱 `run:<handle>`。
  - **Failed rows** — 實體：`FailedRowPage`。呼叫：
    `attempt_failed_page`。cursor 式分頁（第 2 部分 §2.2.6；
    `limit ≤ 500`）。
  - **Errors by code** — 渲染 `AttemptDetail.by_error_code`（32 上限
    `OTHER` 溢位）。
  - **Logs** — handler stderr/stdout 記錄 tail。Bootstrap 透過
    `handler_log_tail`；即時更新透過 `handler_log_subscribe`
    （Plan 9 §7.4 Flow K）。
  - **Artifacts** — `AttemptDetail.paths` 加上「在 Finder 顯示」
    （第 2 部分 §2.3）。
- **Row History drawer** — 從 Failed rows 點開。實體：`RowHistory`。
  呼叫：`attempt_row_history`。嚴格按需（第 2 部分 §2.2.7）。
- **Run launcher**（Execution Detail header）— 主「Run」按鈕用預設
  值快速啟動；齒輪 icon 副按鈕展開 inline 選項面板。面板欄位:
  Handler 目錄(localStorage 跨 session 持久化)、Sample first N rows
  (`row_limit`)、Workers 覆寫、「Skip rows already attempted」勾選盒
  (驅動 `skip_attempted`,用 `RowResolution.attempted_seqs` 跨多次
  run 累進採樣不重複的 row)、Dry run。面板 header 同時顯示來自
  `exec_rollup` 的 `total / attempted / fresh` 列計;面板底部即時
  預覽「Will dispatch N rows」。呼叫 `run_start`。
- **Export dialog** — 構造 `ExportOpts`，呼叫 `exec_export`。
- **Settings** `/settings` — 實體：`Settings`。呼叫：
  `workspace_settings_load`、`workspace_settings_save`、`run_active`
  （透過 workspace 切換按鈕）、`workspace_open`（切換時）。

  版面：單欄表單，分四個區塊。
  1. **Workspace** — 以唯讀等寬文字顯示目前的 `workspace_root`；
     **Switch workspace…** 按鈕開啟目錄選取器。當 `run_active().len()
     > 0` 時按鈕停用並顯示琥珀色警告（頁面掛載時每 2 秒刷新）
     — 切換會使進行中的 run 成為孤兒。
  2. **Concurrency** — `max_concurrent_runs` 數字輸入框。當值與已載入
     伺服器值不同時，輸入框下方顯示藍色「Will apply on next workspace
     open」banner，因為此欄位僅在 `workspace_open` 時才會被讀取
     （第 5 部分 §5.6）。
     （Per-run worker 數量在 RunButton 選項面板裡指定，不在 Settings;
     `Settings.default_workers` 是 dead code,studio-core 的 `start_run`
     從來沒讀過,已移除。）
  3. **Telemetry** — `telemetry_opt_in` 核取方塊。
  4. **Logs**（Plan 9）— `handler_log_capture_raw_stdout` 切換開關
     （「Capture raw stdout」）。標籤副文字：「啟用後，handler stdout
     中含有效 outcome JSON 的行也會寫入 handler_log.log，可能增加
     檔案大小。」預設關閉。下次 run 後生效（非追溯）。

  底部有 Save / Cancel 按鈕。Save 透過 `workspace_settings_save` 持久化
  並使快取查詢失效；Cancel 從已載入值還原。

### 已留錨點、v1 不建

- **Sidebar「Authoring」群組** — *本部分原寫為* disabled「Coming soon」。
  **第 8 部分取代此立場**:v1 Authoring 群組為可用,含 Handlers 路由。
  下方剩餘的錨點項目(Manifest editor、Pack)仍適用。
- **`/handlers` 與 `/handlers/:name`** — **Plan 7：v1 兩路由均已上線。**
  Sidebar Handlers 項目已啟用；清單與詳情頁均已出貨。完整 IA 說明見第 8
  部分 §8.6.1，關聯 user flow 見下方 §7.4 Flow H–J。
- **`ListFilter` filter bar** — exec list 上方保留區域；v1 隱藏
  （第 5 部分 §5.2 `ListFilter`）。
- **`HandlerSource` picker** — v1 僅 `Dir`，picker 為單欄位。v2 升級
  為 segmented control（`Dir` / `Sandbox`），版面不變（第 5 部分 §5.4）。

#### Smoke test section（Plan 13）

在「Last build」下方、「Files」上方，`<SmokeSection />` 讓使用者不建立
execution，直接透過 handler binary 派發 1–100 列並觀察結果。資料來源：

- **Paste JSON** — 每列一個 JSON 物件；即時顯示「N rows parsed」指示
- **Fixtures…** — 選取 `.jsonl` / `.ndjson` / `.json`（頂層陣列）/
  `.csv` 檔案或包含其中之一的目錄（優先順序：jsonl > ndjson > json > csv）
- **One synthetic row** — 派發 `{ "row": 1 }`

強制單 worker 逐列模式（即使是 batch handler 也逐列處理）。結果以 5 欄
表格呈現：`seq / status / message / dur_ms / data`。計數條顯示 success/error/
crash 合計、elapsed ms 及 handler exit code。`stderr` tail（最後 4 KiB）可
摺疊顯示。

Run 按鈕在解析錯誤存在、無可用列或 smoke 執行中時停用。Plan 8 build gate
先跑（來源比 binary 新時重建）。

當此 handler 在任何 workspace 進程中有 exec attempt 正在執行時，smoke 被
拒絕（`handler_busy`，跨進程 sqlite gate）。

> **與規格的偏差說明：** 設計規格描述的是「Smoke test TAB」。實作使用
> **section**（非 tab），因為 `HandlerDetailPage` 採用 section 而非 tab
> 結構。功能上完整符合設計。

### 全域導航

- **左側 sidebar（持久）：** Workspace 群組（Executions、Settings）
  + Authoring 群組（Handlers 已啟用 — Plan 7）。
- **頂部 header（持久）：** workspace 名稱 + 路徑 tooltip；麵包屑
  （`Executions / <exec> / Attempt #N / Failed rows`）；右側 **Active
  runs pill**。
- **Active runs pill：** 消費 `active_runs_stream()`（第 5 部分 §5.2）
  與 `runs:active` Tauri 事件（第 5 部分 §5.5、第 6 部分 §6.6）。
  顯示 `n running`；hover 展開每個 run 的迷你進度；點擊跳轉到該
  attempt 的 Live tab。`n = 0` 時隱藏。
- **v1 無浮動視窗、無 dock badge。** 多 run UX 用 header pill 加上
  per-tab spinner dot；對預設 ≤ 3 並發（第 3 部分 §3.4）足夠。

## 7.4 主要 user flow

每步列出對應的 Tauri command（第 5 部分 §5.5）。

### Flow A — 新建 execution、首次 run（空 workspace）

| # | 步驟 | Command |
|---|---|---|
| 1 | Boot → Workspace Picker（偵測空狀態） | `workspace_settings_load` |
| 2 | 選 workspace 資料夾 → 儲存 | `workspace_settings_save`、`workspace_open` |
| 3 | Workspace Home 空狀態 → 「New execution」 | — |
| 4 | Wizard 步驟 1：name + input path | — |
| 5 | Wizard 步驟 2：handler dir + 「Validate」 | `manifest_validate` |
| 6 | 提交 → 導向 Execution Detail | `exec_start` |
| 7 | 點「Run」→ 配置 `RunOpts`，提交 | `run_start` |
| 8 | 自動跳到 Attempt Detail (Live)；訂閱 | event `run:<handle>` |

步驟 7 對應第 3 部分 §3.3 的 `Starting → Running` 轉移。Session 直接從 `Starting` 開始注冊。

### Flow B — 觀察進行中的 run 並 cancel

| # | 步驟 | Command |
|---|---|---|
| 1 | 在 header pill 點選一筆 active run | `run_active` |
| 2 | 進入 Attempt Detail (Live)；進度 + tail | event `run:<handle>` |
| 3 | 點「Cancel」(destructive 樣式) | — |
| 4 | 確認 dialog → soft cancel | `run_cancel(handle, Soft)` |
| 5 | UI 顯示 `Cancelling` + 「n rows in flight」倒數 | event Tick |
| 6 | 10 秒後「Force kill」按鈕淡入 | — |
| 7 | 高摩擦確認（輸入 token）→ hard kill | `run_cancel(handle, Hard)` |
| 8 | `Aborted { reason: UserCancelled }` → 最終 summary | event |

嚴格對應第 3 部分 §3.5 的 soft/hard 語意。

### Flow C — 檢視失敗、重跑失敗列

| # | 步驟 | Command |
|---|---|---|
| 1 | Execution Detail → Attempts → 點一個 `Done` attempt | `exec_show` |
| 2 | Failed rows tab → 載入第一頁 | `attempt_failed_page({offset:0, limit:200})` |
| 3 | 頂端摘要來自 `by_error_code`（快取） | （`attempt_show` 內已有） |
| 4 | 滾到底 → 「Load more」帶 `next_offset` | `attempt_failed_page` |
| 5 | 點某列 → Row History drawer | `attempt_row_history` |
| 6 | 點「Retry failed only」 | — |
| 7 | Run launcher 開啟，`retry_failed=true` 預勾，確認 | `run_start` |
| 8 | 自動跳到新 attempt 的 Live tab | event |

`FailedPageQuery` 語意見第 2 部分 §2.2.6。

### Flow D — 跨 attempt rollup 與 export

| # | 步驟 | Command |
|---|---|---|
| 1 | Execution Detail → Rollup tab | `exec_show`（快取） |
| 2 | Cold 載入 skeleton（第 2 部分 §2.2.5） | `exec_rollup` |
| 3 | 渲染 `resolved / failed_last / crashed_last / too_large / never_attempted` + `by_error_code` | — |
| 4 | 「Export」→ dialog | — |
| 5 | 選 `format = Both`；勾 `require_complete` | — |
| 6 | 確認 → progress toast | `exec_export` |
| 7 | 完成時 toast 提供「Reveal output dir」via `ExportReport.output_dir` | — |

### Flow H — 新建 handler（scaffold，Plan 7）

| # | 步驟 | Command |
|---|---|---|
| 1 | Sidebar Handlers → `/handlers` | `handler_list` |
| 2 | 點「New Handler」→ `ScaffoldDialog` 開啟 | — |
| 3 | 輸入名稱（正則 `/^[a-z0-9][a-z0-9-]*$/`，前端即時驗證）、選模板（`GoStdio` / `GoBatch` / `Empty`）、輸入 `primary_field` | — |
| 4 | 提交 → `handler_scaffold` mutation | `handler_scaffold` |
| 5 | 成功：toast + 關閉 dialog + 導向 `/handlers/<name>` | `handler_show` |

負向路徑：`HandlerExists` / `InvalidHandlerName` 錯誤以 inline 方式顯示在 dialog 中；使用者更正名稱或取消前 dialog 保持開啟。

### Flow I — 改名 handler（惰性，Plan 7）

| # | 步驟 | Command |
|---|---|---|
| 1 | `/handlers/:name` → 「Rename…」→ `RenameHandlerDialog`（預填目前名稱） | — |
| 2 | 修改名稱（必須與目前不同且通過正則）→ 點「Rename」 | — |
| 3 | `handler_rename` mutation | `handler_rename` |
| 4 | 成功：toast + 關閉 dialog + 導向 `/handlers/<新名稱>` | — |

惰性語意：SQLite 不更新；舊有 `ExecSummary.last_handler_dir` 列仍引用舊名稱（資訊性，非可載入依據——見第 2 部分 §2.2.2 惰性改名說明）。

### Flow J — 刪除 handler（輸入 token 確認，Plan 7）

| # | 步驟 | Command |
|---|---|---|
| 1 | `/handlers/:name` → 「Delete…」→ `DeleteHandlerDialog` | — |
| 2 | 使用者輸入完整 handler 名稱（區分大小寫）以啟用 Delete 按鈕 | — |
| 3 | 點「Delete」→ `handler_delete` mutation | `handler_delete` |
| 4 | 成功：toast + 關閉 dialog + 導向 `/handlers` | `handler_list` |

惰性語意：`executions` 列中舊有的 `last_handler_dir` 引用在刪除後仍保留。
符號連結防護：三層 — (1) 正則驗證名稱、(2) 路徑 canonicalize、(3) 斷言以 workspace `handlers/` 父目錄為前綴。

### Flow K — 查看 handler 記錄（Logs tab，Plan 9）

| # | 步驟 | Command |
|---|---|---|
| 1 | 導向 Attempt Detail；點選 **Logs** tab | — |
| 2 | Tab 掛載 → bootstrap 載入 | `handler_log_tail(exec_id, attempt_id, 5000)` |
| 3 | 若 `handler_log.log` 不存在 → 顯示「No log file. This attempt predates Plan 9 log capture.」 | — |
| 4 | 若檔案存在但為空 → 顯示「Handler has not produced any output yet.」 | — |
| 5 | 若 attempt 仍在執行（`isLive`）：訂閱即時行 | `handler_log_subscribe(exec_id, attempt_id)` |
| 6 | 即時行透過事件 `handler_log:<attempt_id>` 在 ~100 ms 內抵達 | event |
| 7 | Worker chip 多選篩選 → 清單縮小至選取的 worker | — |
| 8 | Stream 篩選（stdout / stderr / 兩者）→ 進一步縮小 | — |
| 9 | 文字搜尋（子字串）→ 進一步縮小 | — |
| 10 | 篩選後無任何行 → 顯示「No lines match the current filters.」 | — |
| 11 | 自動捲動開啟：新即時行抵達時 viewport 保持在底部 | — |
| 12 | 使用者手動向上滾動 → 自動捲動解除 | — |
| 13 | 點 **Pause**：即時行在緩衝區累積，可見清單凍結 | — |
| 14 | 點 **Resume**：緩衝行沖入可見清單，自動捲動重新接合 | — |
| 15 | 批次 payload 中 `dropped > 0` → 琥珀 banner「⚠ N 行 handler 記錄已丟棄 — 請開啟記錄檔取得完整內容」 | — |
| 16 | 點 **Reveal log file** → OS 檔案管理員開啟 `<attempt_dir>/handler_log.log` | `shell::open` |
| 17 | Tab 卸載或 attempt 完成 → 取消訂閱 | `handler_log_unsubscribe(attempt_id)` |

**元件結構：**
- `LogsToolbar` — worker chips、stream 切換、搜尋輸入、Pause/Resume 按鈕、Reveal 按鈕。
- `LogsVirtualList` — `@tanstack/react-virtual` 清單；每列 28 px；帶顏色 stream chip（黃色 stderr / 藍色 stdout）；等寬內容。
- `AttemptLogsTab` — 協調 bootstrap、即時訂閱、篩選組合、丟棄 banner。

### Flow L — 選取模式 + 批量刪除（Plan 10）

| # | 步驟 | 呼叫的 Command |
|---|---|---|
| 1 | Exec list header → 點 **Select**（選取） | — |
| 2 | 核取方塊欄出現於名稱左側；每列都有核取方塊；Cancel 按鈕取代 header 中的 Select | — |
| 3 | 進行中執行的列核取方塊**停用**；hover 顯示「Cancel active run first」（由 `last_attempt_state === "running"` 偵測） | — |
| 4 | 點擊非停用列 → 切換選取狀態；選取模式中列點擊不再導航 | — |
| 5 | 點擊 **Cancel** → 退出選取模式並清除所有選取 | — |
| 6 | 選取 ≥ 1 列 → header 出現 **Delete N execution(s)** 按鈕 | — |
| 7 | 點擊 Delete N → `DeleteExecutionsDialog` 開啟：標題「Delete N execution(s)?」；列出最多 10 筆所選名稱 + 「…及 M 筆更多」；顯示總大小；destructive **Delete** 按鈕 | — |
| 8 | 確認 → mutation | `execution_delete_bulk(exec_ids)` |
| 9 | 成功：Sonner toast「N execution(s) deleted」；`exec_list` query 失效；dialog 關閉；退出選取模式 | event `exec_list:refresh` |
| 10 | 部分失敗：清單上方黃色警告，顯示失敗的 exec_id 及原因；Dismiss 按鈕清除警告 | — |
| 11 | 已刪除執行的 ExecDetail 頁面：`exec_show` 回傳 `NotFound`；渲染「This execution has been deleted or is unavailable.」+ ← 返回連結 | `exec_show` |

**元件結構：**
- `DeleteExecutionsDialog` — shadcn `AlertDialog`；項目清單（最多 10 + 溢位計數）；透過 `formatBytes` 顯示總大小；destructive 確認按鈕。
- `useExecutionDelete` — 單一刪除 mutation hook；成功後使 `exec_list` 失效。
- `useExecutionDeleteBulk` — 批量刪除 mutation hook；任何成功刪除後使 `exec_list` 失效；暴露 `bulkFailures` 狀態。
- `formatBytes` helper — 住在 `apps/rowforge-studio/src/lib/format.ts`；由 dialog 與 ExecList 大小欄共用。

### Flow M — 重新執行失敗的 row（Plan 11）

| # | 步驟 | Command |
|---|---|---|
| 1 | 導向已完成且有失敗的 attempt 的 Attempt Detail | `attempt_show` |
| 2 | 點選 **Failed rows** tab；頂端顯示 N 筆失敗 row | `attempt_failed_page` |
| 3 | Tab header 出現 **Re-run N rows** 按鈕 | `attempt_failed_row_ids`（掛載時呼叫） |
| 4 | 當 N = 0 時，按鈕**停用**，tooltip 為「No failed rows to re-run」 | — |
| 5 | 當 `hasActiveRun` 為 true（目前 attempt 非 terminal）時，按鈕**停用**，tooltip 為「Cancel active run first」 | — |
| 6 | 當 `exec.last_handler_dir` 不存在時，按鈕**停用**，tooltip 為「Source attempt has no handler reference」 | — |
| 7 | 點選已啟用的 **Re-run N rows** 按鈕 → 開啟 `RerunFailedDialog` | — |
| 8 | Dialog 標題：「Re-run N failed rows?」；顯示 `exec.last_handler_dir` 路徑；顯示來源 attempt id | — |
| 9 | 點 **Cancel** → dialog 關閉，無 mutation | — |
| 10 | 點 **Re-run** → 送出 mutation；dialog 關閉 | `run_start(exec_id, last_handler_dir, onlyRowIds=[...seq 值])` |
| 11 | 成功後：Sonner toast；UI 自動導向新 attempt 的 **Live** tab | event `run_start` response |
| 12 | 新 attempt 的 pipeline 僅派發 N 個失敗的 seq 值；其他 row 不重新處理 | — |
| 13 | 新 attempt 完成後：同一 seq 嘗試兩次 → exec rollup 採「最後一次 attempt 勝出」語意；最新的每 seq 結果為準 | `exec_rollup` |

`hasActiveRun` 由目前 attempt 的狀態是否為非 terminal 推導（見第 3 部分
§3.3）。這是近似值：若同一 exec 上的*另一個* attempt 正在執行，後端仍會
拒絕並回傳 `UiError::RunBusy`，UI 以 toast 呈現。

**元件結構：**
- `RerunFailedDialog` — shadcn `Dialog`；顯示 row 數量、`last_handler_dir`
  路徑、來源 attempt id；Cancel + Re-run 按鈕。
- `useAttemptFailedRowIds(execId, attemptId)` — React Query hook；呼叫
  `attempt_failed_row_ids`；按 attempt 快取。
- `useRunStart` — 現有 hook，以 `onlyRowIds?: number[]` 參數擴充（Plan 11）。
- `AttemptFailedTab` — Failed rows tab；新增 Re-run 按鈕，含以上三種停用狀態。

## 7.5 顏色與狀態映射

此節為 v1 規範。

### `RunStatus`（第 3 部分 §3.3）

| RunStatus | Token | Hex (dark) | 視覺 | Icon (lucide) |
|---|---|---|---|---|
| Starting | `info-500` | `#3B82F6` | 藍色 dot + spinner | Loader2 |
| Running | `success-500` | `#10B981` | 綠色 dot + heartbeat | Play |
| Cancelling | `warning-500` | `#F59E0B` | 琥珀 dot + spinner | Loader2 + Slash |
| Done | `success-600` | `#059669` | 實心綠 dot | CheckCircle2 |
| Aborted | `neutral-400` | `#9CA3AF` | 灰色 dot + 中線 | XCircle |
| Crashed | `error-500` | `#EF4444` | 紅色 dot + 鋸齒邊框 | AlertOctagon |

### `RowOutcomeKind`（第 2 部分 §2.2.6）

| Kind | Token | Hex | 用法 |
|---|---|---|---|
| Success | `success-500` | `#10B981` | 綠色 left border 2 px |
| Error | `error-500` | `#EF4444` | 紅色 left border 2 px + error-code chip |
| Crash | `error-700` | `#B91C1C` | 深紅 + AlertOctagon + `WORKER_CRASH` chip |
| TooLarge | `warning-600` | `#D97706` | 琥珀 + FileWarning icon |

### `Phase`（第 6 部分 §6.1）

Attempt Detail header 的水平 **chip bar**。當前 phase 高亮；已完成的
phase 打勾並暗化；未來 phase 灰。Phase：`Initializing → Snapshotting
→ Starting → Running → Cancelling（條件） → Persisting`。

| Phase | Chip | Icon |
|---|---|---|
| Initializing | neutral spinner | Settings2 |
| Snapshotting | info spinner | Camera |
| Starting | info spinner | Power |
| Running | success outline (active) | Activity |
| Cancelling | warning solid | StopCircle |
| Persisting | info spinner | Save |

## 7.6 關鍵互動模式

### 7.6.1 進度區（第 6 部分 §6.7）

三欄式 grid，由 4 Hz `Tick`（第 6 部分 §6.2）驅動。150 ms ease 更新；
`tabular-nums` 鎖欄寬。

- **左：** 進度條（`h-3`、`rounded-full`、`success-500` fill 於
  `neutral-800` track），下方 `processed / total (xx.x%)`。若
  `total = None`（input 未 snapshot；第 6 部分 §6.1），隱藏百分比
  並渲染 `processed —`。
- **中：** 兩個大型數字 `rate_1s` / `rate_10s`（`text-2xl tabular-nums`),
  下方 `rows/s` 副字；`ETA` 大型倒數。10 秒緩衝填滿前顯示 `—`。
- **右：** 垂直 stack 顯示 `in_flight`（Activity icon）與 `queue_depth`
  （Layers icon）。
- **Heartbeat：** 每個 Tick 在進度條尾端閃 1 px 白色 highlight 100 ms。
  即使計數器沒動，也傳達「事件還在流」。

### 7.6.2 事件 tail（第 6 部分 §6.2）

200 筆 virtualized list。每列 28 px 高、等寬欄位、左緣 3 px 色帶對應
`RowOutcomeKind`（§7.5）。

欄位：`[seq#]` · `row_index` · error-code chip · message（truncate）·
`dur_ms`（右對齊、`tabular-nums`)。

右上角 filter chips：`All / Errors only / Crashes only`。**預設「Errors
only」**，因為 `OutcomeSample` 90% token budget 給錯誤/崩潰（第 6 部分
§6.2）。

新事件從頂部插入；底部 tail 淡出。

### 7.6.3 Cancel 兩階段（第 3 部分 §3.5）

- **確認 soft cancel：** `AlertDialog` 文案「Soft cancel? In-flight
  rows will finish.」
- **`Cancelling` 狀態：** 琥珀 sticky banner「Cancelling — `n` rows in
  flight」；`n` 由 `Tick.in_flight` 更新。`in_flight` 數字旁 10 秒
  circular countdown。
- **10 秒後：** 「Force kill」紅色 outline button 淡入（第 3 部分 §3.5
  建議閾值）。
- **Hard kill 確認：** destructive `AlertDialog`「Partial outcomes may
  be lost. This cannot be undone.」使用者必須輸入 exec 名稱前 4 字。
  高摩擦是設計目的。

**強制 kill 徽章。** 當 `attempt.state === "aborted"` 且
`attempt.cancelled_reason === "hard_cancel"` 時，狀態徽章以紅色渲染為
"force-killed"，而非預設的 "aborted" 樣式。
同時呈現於 AttemptDetail 標頭與 ExecDetail AttemptsList。

CancelDialog 流程（現有）已驅動此狀態機：軟取消在 10 秒內未完成時
顯示「Force kill」按鈕；確認後觸發 `cancel(handle, Hard)`。
Plan 14 使該後端呼叫真正對 workers 執行 SIGKILL。

### 7.6.4 生命週期 banner（`WorkerCrashed`、`StallWarning`、`PipelineWarning`）

三者皆 **inline 嵌入事件 tail**，使用全寬列（48 px，非標準 28 px）
打破視覺節奏。同時側邊 toast（右下，5 秒自動消失），確保使用者不在
Live tab 時也能看到。

- `WorkerCrashed`：紅底 + AlertOctagon + `worker_id` + `stderr_tail`
  前 3 行摺疊；點開右側 Sheet 展開全文 ≤ 64 KiB（第 6 部分 §6.1）。
- `StallWarning`：琥珀 + Hourglass + `silent_secs`。
- `PipelineWarning`：藍 + Info + `code` + `message`。

### 7.6.5 `EVENT_LAG` sticky banner

`PipelineWarning` 子類（第 6 部分 §6.2）。事件 tail 頂部固定 banner：

> Display lagging — `n` events dropped. Counters are still accurate.
> [Open `outcomes.jsonl`]

「Open」連結用 `AttemptDetail::paths.outcomes_jsonl`（第 2 部分 §2.3）
經 Tauri `shell::open`。30 秒無 lag 後自動消失。

「Counters are still accurate」是契約句；它告訴使用者哪些介面可信
（`Tick` 中的持久計數，不是取樣的 tail）。

### 7.6.6 失敗列表格

- 欄位：`seq` · `row_index` · `kind`（chip）· `error_code`（mono chip）
  · `message`（truncate；hover 顯示全文）· `dur_ms`（右對齊
  `tabular-nums`）。
- 點列 → 原位 accordion 展開，渲染 `raw_record` 為可摺疊 JSON tree
  （等寬、語法高亮）。
- 分頁：**僅 cursor 式**（「Load more」帶 `next_offset`）。v1
  **不**顯示 `n / m` 頁碼，因為 v2 索引未到前
  `FailedRowPage::total_known` 通常為 `None`（第 4 部分 §4.4）。
- 右上「Reveal in Finder」開啟 `paths.outcomes_jsonl`。

## 7.7 空 / 邊界狀態

| # | 狀態 | 觸發 | 顯示 | 可做動作 |
|---|---|---|---|---|
| 1 | 空 workspace | `exec_list` → `[]` | Icon + 「No executions yet」+ primary CTA | 新建；換 workspace |
| 2 | Exec 從未 run | `ExecDetail.attempts == []` | 「This execution has never been run」+ Run CTA；Rollup tab disabled；Failed rows 隱藏 | Run；查看 bindings |
| 3 | Attempt 全 success | `failed + crashed + too_large == 0` | Success icon + 「All rows resolved in this attempt」；Errors-by-code 隱藏 | 返回；Rollup；Export |
| 4 | Schema 不符 | `workspace_open → WorkspaceLocked`（第 5 部分 §5.3) | 全頁 blocking modal：`Workspace.schema_version` vs Studio 版本 + 「Open different workspace」+ 「Copy details」 | 換 workspace；退出 |
| 5a | `RunBusy`(PerExec) | `run_start → RunBusy { scope: PerExec }` | Run launcher 內 inline error + 連到 active attempt | 跳到 active；取消後重試 |
| 5b | `RunBusy`(Workspace) | `run_start → RunBusy { scope: Workspace }` | Toast：「Workspace concurrent-run limit reached (3)」+ Active runs / Settings 連結 | 開 Active runs；改設定 |
| 6a | Orphan、閒置 > 5 min | `open` 自動 mark aborted（第 3 部分 §3.7） | Home 頂端 banner：「N attempt(s) were marked aborted on launch (orphaned)」+ Review 連結 | 關閉；review；retry-failed |
| 6b | Orphan、閒置 ≤ 5 min | 不確定；CLI 可能在跑 | Attempt 上琥珀 banner：「This attempt may still be running externally」+ Mark-aborted + Refresh | 手動 mark；refresh；等待 |
| 7 | Manifest invalid | `manifest_validate → ManifestReport.errors` | Handler picker 下方 inline `ManifestError` 列表；submit disabled | 改檔；重 validate |
| 8 | Cancel 卡住 > 10 秒 | `RunStatus::Cancelling` 過閾值 | 紅色 sticky bar + Force kill button + 高摩擦確認 | 等待；force kill |

狀態 4、5a、5b、6a、6b、7 直接反映第 3 / 第 5 部分的契約;UI 是
讓它們可被使用者感知的唯一介面。

## 7.8 載入策略與時間預算

後端 cost class（第 2 部分 §2.1、第 4 部分 §4.3）轉換為 UI 模式：

| 介面 | Cost | 預算 | UI 模式 |
|---|---|---|---|
| `workspace_open`、header workspace 名稱 | hot | < 10 ms | 直接 render |
| Exec list 切換 / 篩選 | warm（mtime 命中） | < 100 ms | 直接 render，無 skeleton |
| Attempt Detail（terminal） | warm | < 100 ms | 直接 render |
| Attempt Detail（running） | hot（aggregator snapshot） | < 50 ms | render + subscribe |
| `ExecRollup` | cold（線性掃所有 attempt） | 1–10 秒 | indeterminate progress + 「Streaming N attempts...」 |
| `FailedRowPage` 第 N 頁 | cold,隨 offset 線性 | 100 ms（前頁）→ 秒級（後頁） | cursor「Load more」,永不用頁碼 |
| `RowHistory`(單列) | cold,隨 attempt 數線性 | 通常 < 1 秒 | drawer 內 spinner |
| `manifest_validate` | warm | < 500 ms | inline 即時驗證 |

**載入元件：**
- **Spinner**（Loader2 rotate）— 非阻塞、< 500 ms 操作。
- **Skeleton**（`bg-neutral-800 animate-pulse`）— 結構化載入：
  ExecSummary 列、ExecDetail header、AttemptDetail stats grid。
- **確定進度條** — 僅用於 Live tab 內的 `Tick.processed / total`
  （第 6 部分 §6.1）。
- **不定線性條** — `ExecRollup`（mid-stream 不知 total）與
  `exec_export` 長寫入。

**插畫：** v1 沒有。空狀態用單一 lucide icon（neutral-600）+ 標題 +
副標 + CTA。理由：bundle 大小、工具 UI 的調性一致、sprint 成本。

## 7.9 `UiError` 呈現對照（第 5 部分 §5.3）

| 變體 | 介面 | 備註 |
|---|---|---|
| `NotFound { kind, id }` | Inline empty state | 不是 toast;頁面本身為空 |
| `InvalidArg(String)` | Inline 表單欄位錯誤 | 即時;可能時在送出前 |
| `HandlerBuildFailed { stderr }` | Modal / 右側 Sheet | 可滾 stderr + 複製按鈕 |
| `RunAborted { reason }` | Attempt Detail 上的 banner | 依 `AbortReason` 分支（見 §7.6.4 + §7.6.3） |
| `UnknownHandle(String)` | Toast（info）+ 自動 refresh `run_active` | handle 過期;靜默恢復 |
| `WorkspaceLocked { by }` | 全頁 blocking modal | 應用層級;其他都不可用 |
| `ManifestInvalid { errors }` | 側邊 panel 列表 + per-error inline | v2 manifest editor |
| `RunBusy { execution_id, scope }` | PerExec：inline disabled 按鈕 + tooltip；Workspace：toast | 不重試循環;使用者必須處理 |
| `Io(String)` | Toast（error）+ 複製細節 | 通常可重試 |
| `Internal(String)` | Toast（error）+ 複製細節 + 「Report issue」 | 後端 bug;UI 不解釋 |
| `EditorNotFound` | Toast（error）+ Settings → Editor 連結 | Plan 7；僅 `handler_open_editor` |
| `HandlerNotFound { name }` | `/handlers/:name` inline empty state 加返回連結 | Plan 7；書籤過期或同時被刪除 |
| `HandlerExists { name }` | ScaffoldDialog / RenameHandlerDialog 的 inline banner | Plan 7；名稱已被使用 |
| `InvalidHandlerName { name }` | ScaffoldDialog / RenameHandlerDialog 的 inline 欄位錯誤 | Plan 7；未通過 `/^[a-z0-9][a-z0-9-]*$/` |
| `ExecutionInUse { exec_id }` | ExecList 選取模式中核取方塊停用 + 提示文字「Cancel active run first」；批量部分失敗時黃色警告 | Plan 10；`execution_delete` 中的進行中執行防護閘 |

`AbortReason`（第 6 部分 §6.5）是至少 9 變體的 union;Aborted banner
分支到對應的 reason 詳情面板（例如 `AllWorkersCrashed` 開啟
`WorkerCrashRecord` 清單;`SnapshotHashMismatch` 顯示 `expected`
vs `actual` digest;`MissingRequiredInput` 列出欄位）。

## 7.10 UI 不得做的事

以下是規格契約禁止;不管設計師覺得多合理,UI 都必須拒絕渲染。

1. **不可即時逐列 outcome 串流。** `OutcomeSample` 是取樣的（20/秒,
   90% 給錯誤;第 6 部分 §6.2）。要看每列只能事後讀 `outcomes.jsonl`。
2. **不可在進行中的 attempt 上顯示 `ExecRollup`。** Cold-only;進行
   中的 attempt `meta.json` 尚未存在（第 2 部分 §2.2.5、第 4 部分
   §4.3）。
3. **不可有「resume orphan」動作。** Studio 只能 mark aborted;重跑
   走新 attempt 的 `--retry-failed`（第 3 部分 §3.7）。
4. **不可在同 exec 上啟動第二個並發 run。** UI 必須在達單 exec 上限時
   先擋住 Run 按鈕,不可讓使用者按下後收到 `RunBusy`（第 3 部分 §3.4）。
5. **不可有每列 × 每 attempt 矩陣。** 按需用 `RowHistory`(第 2 部分
   §2.3）。
6. **不可有跨 run 合併時間軸或比較圖。** 不在範圍（第 6 部分 §6.6）。
7. **不可在失敗列用「第 N 頁 / 共 M 頁」。** v1 `total_known` 通常為
   `None`;僅 cursor 式（第 4 部分 §4.4）。
8. **不可在 total 未知前顯示「100.0%」。** `Tick.total` 是
   `Option<u64>`(第 6 部分 §6.1）;改顯 `processed —`。
9. **UI 程式碼不可直接讀 `outcomes.jsonl`。** 所有讀取走 `studio-core`
   的投影（第 2 部分 §2.3、第 5 部分 §5.2）。`AttemptDetail::paths`
   僅供「在 Finder 顯示」使用。
10. **不可有 `subscribe_all_runs` 多工。** 改用 `active_runs_stream()`,
    僅計數的聚合（第 5 部分 §5.2、第 6 部分 §6.6）。

## 7.11 設定畫面

Settings 頁逐欄位暴露 `Settings`(第 2 部分 §2.2.9）。

- `workspace_root` — 唯讀顯示;「Switch workspace」開啟 picker。
- `max_concurrent_runs` — 數字輸入,預設 3（第 3 部分 §3.4）。
  降至低於目前 active 數時顯示確認警告。
- `telemetry_opt_in` — switch,預設關;tooltip 註明 v1 不收集。
- **`preferred_editor`**（Plan 7）— 文字輸入，placeholder `"code"`。
  可為空；空時解析器依序嘗試 `$VISUAL` / `$EDITOR` / 探測
  （第 8 部分 §8.4.1）。顯示於 Settings 表單第四個「Editor」區塊。
  透過 `workspace_settings_save` 儲存，下次 `handler_open_editor`
  呼叫即生效（不需重啟）。

注意:`default_workers` **不是** Settings 欄位。Per-run worker 數
在 RunButton 選項面板配置;studio-core 的 `start_run` 從來沒讀過
workspace 全域預設,因此移除。

v1 無進階 JSON 編輯器。路徑解析住在 Tauri 層（第 5 部分 §5.6）。

## 7.12 跨章節對照彙整

| §7.x | 依據 |
|---|---|
| 7.1 技術棧 | 第 1 部分 §1.3 架構;第 5 部分 §5.7 穩定性政策 |
| 7.2 原則 | 第 1 部分 §1.2 原則;第 6 部分 §6.1 事件分類 |
| 7.3 IA | 第 1 部分 §1.4 範圍;第 2 部分 §2.1 實體清單;第 5 部分 §5.5 commands |
| 7.4 flows | 第 3 部分 §3.3 狀態機;§3.5 cancel;第 5 部分 §5.2 API |
| 7.5 顏色 | 第 3 部分 §3.3 `RunStatus`;第 2 部分 §2.2.6 `RowOutcomeKind`;第 6 部分 §6.1 `Phase` |
| 7.6.1 進度 | 第 6 部分 §6.1 `Tick`;§6.2 4 Hz 預算;§6.7 指標 |
| 7.6.2 事件 tail | 第 6 部分 §6.2 token-bucket 取樣 |
| 7.6.3 cancel | 第 3 部分 §3.5 soft/hard,10 秒閾值 |
| 7.6.4 banners | 第 6 部分 §6.1 生命週期事件;§6.5 `WorkerCrashRecord` |
| 7.6.5 `EVENT_LAG` | 第 6 部分 §6.2 `PipelineWarning { code: "EVENT_LAG" }` |
| 7.6.6 失敗列 | 第 2 部分 §2.2.6 `FailedRow`、`FailedPageQuery`;第 4 部分 §4.4 v2 索引 |
| 7.7 邊界狀態 | 第 1 部分 §1.5;第 3 部分 §3.4 / §3.7;第 5 部分 §5.3 |
| 7.8 載入 | 第 2 部分 §2.1 cost class;第 4 部分 §4.3 快取層 |
| 7.9 錯誤 | 第 5 部分 §5.3 `UiError`;第 6 部分 §6.5 `AbortReason` |
| 7.10 禁止事項 | 第 2 部分 §2.3;第 3 部分 §3.4 / §3.7;第 4 部分 §4.3;第 6 部分 §6.2 / §6.6 |
| 7.11 設定 | 第 2 部分 §2.2.9 `Settings`;第 5 部分 §5.6 |

## 7.13 線框圖（示意）

ASCII;只示意比例。寬度約 96 字元。實際版面後續走 Figma;這裡放著
是為了讓評審者能在尚未動到像素之前,先針對資訊密度與分組爭論。

### W-1 Workspace Home(Exec list)

```
┌──────────────────────────────────────────────────────────────────────────────────────────────┐
│  ◇  billing-workspace ▾    Executions                              ◯ 2 running ▾    + New    │
├──────────────┬───────────────────────────────────────────────────────────────────────────────┤
│ WORKSPACE    │  Executions                                                                   │
│ ● Executions │  ┌─────────────────────────────────────────────────────────────────────────┐  │
│   Settings   │  │ Name          Created       Rows     Last attempt  Attempts             │  │
│              │  ├─────────────────────────────────────────────────────────────────────────┤  │
│ AUTHORING    │  │ refund-bf-3   2026-05-22    12,043   ● Running     3            ⏵ open  │  │
│ ░Handlers░   │  │ refund-bf-2   2026-05-21    12,043   ✓ Done        5            ⏵ open  │  │
│  Coming soon │  │ refund-bf-1   2026-05-20    12,043   ✓ Done        4            ⏵ open  │  │
│              │  │ apple-rfd     2026-05-19       487   ✗ Aborted     2            ⏵ open  │  │
│              │  │ billing-test  2026-05-18         3   ⊘ Crashed     1            ⏵ open  │  │
│              │  │ smoke-tiny    2026-05-18         3   — never run   0            ⏵ open  │  │
│              │  └─────────────────────────────────────────────────────────────────────────┘  │
│              │  Showing 6 of 6 · sorted by created desc                                      │
└──────────────┴───────────────────────────────────────────────────────────────────────────────┘
```

狀態:● Running、✓ Done、✗ Aborted、⊘ Crashed、— never run。
Active runs pill hover 展開見 W-2。

### W-2 Active runs pill(hover popover)

```
                                           ┌─────────────────────────────────┐
                                ◯ 2 running│ Active runs                     │
                                           │ ─────────────────────────────── │
                                           │ refund-bf-3  ▓▓▓▓▓░░░  62%  ⏵   │
                                           │   rate 980/s · ETA 1m 04s       │
                                           │ apple-rfd-2  ▓▓░░░░░░  18%  ⏵   │
                                           │   rate  84/s · ETA 4m 22s       │
                                           └─────────────────────────────────┘
```

### W-3 Execution Detail — Attempts tab

```
┌──────────────────────────────────────────────────────────────────────────────────────────────┐
│  Executions / refund-bf-3                                          ◯ 2 running ▾   ▸ Run     │
├──────────────┬───────────────────────────────────────────────────────────────────────────────┤
│ ● Executions │  refund-bf-3      input: refund_records_dump.csv (12,043 rows)                │
│   Settings   │  handler: golang-refund-backfill 0.1.0   created: 2026-05-22 09:14            │
│ ░Handlers░   │                                                                               │
│              │  ┌─Attempts──Rollup──Bindings──Artifacts────────────────────────────────────┐ │
│              │  │                                                                          │ │
│              │  │ #  State        Started        Run type    success / failed / crashed   │ │
│              │  │ ── ──────────── ───────────── ─────────── ──────────────────────────── ──│ │
│              │  │ 3  ● Running    05-22 14:02   full          7,489  /     12  /     0  ⏵ │ │
│              │  │ 2  ✓ Done       05-22 11:30   retry-failed    412  /      0  /     0  ⏵ │ │
│              │  │ 1  ✗ Aborted    05-22 10:18   full          5,820  /    387  /    24  ⏵ │ │
│              │  │                                                                          │ │
│              │  └──────────────────────────────────────────────────────────────────────────┘ │
└──────────────┴───────────────────────────────────────────────────────────────────────────────┘
```

### W-4 Attempt Detail — Live tab

```
┌──────────────────────────────────────────────────────────────────────────────────────────────┐
│  Executions / refund-bf-3 / Attempt #3 / Live                      ◯ 2 running ▾   ■ Cancel  │
├──────────────┬───────────────────────────────────────────────────────────────────────────────┤
│ ● Executions │  Attempt #3   ● Running    started 05-22 14:02 (12m 04s ago)                  │
│              │                                                                               │
│              │  Phase:   ✓ Init  ✓ Snap  ✓ Start  ◉ Running  ·  Cancel  ·  Persist           │
│              │  ┌─Live──Failed rows──Errors by code──Artifacts──────────────────────────────┐│
│              │  │                                                                           ││
│              │  │ ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░░░░░░░  7,501 / 12,043   62.3%          ││
│              │  │                                                                           ││
│              │  │   980        950        1m 02s        12         3                       ││
│              │  │   rate/1s    rate/10s   ETA           in-flight  queue                   ││
│              │  │                                                                           ││
│              │  │ ┌─Recent events ───────────────────  [All] (Errors) [Crashes]            ││
│              │  │ │ [#7498]  row 7501  ● BILLING_NOT_FOUND  no billing row for billid   12ms││
│              │  │ │ [#7491]  row 7494  ● BILLING_NOT_FOUND  no billing row for billid   11ms││
│              │  │ │ [#7480]  row 7483  ● DB_ERROR           connection timeout          1.2s││
│              │  │ │ ─── WorkerCrashed  worker_id=2  signal=11  ─ click to expand ─────  ████││
│              │  │ │ [#7420]  row 7423  ● MISSING_BILLID     row has no 'billid'          2ms││
│              │  │ │ ...                                                                    ││
│              │  │ └────────────────────────────────────────────────────────────────────────││
│              │  └───────────────────────────────────────────────────────────────────────────┘│
└──────────────┴───────────────────────────────────────────────────────────────────────────────┘
```

### W-5 Cancelling 狀態(10 秒閾值到達)

```
│  Attempt #3   ◐ Cancelling    soft cancel issued 11s ago                                     │
│  ┌──────────────────────────────────────────────────────────────────────────────────────┐   │
│  │ ⚠  Cancelling — 4 rows still in flight                                  ◷ 11s        │   │
│  │    Soft cancel is taking longer than expected.                  [ Force kill ]       │   │
│  └──────────────────────────────────────────────────────────────────────────────────────┘   │
│  Phase:   ✓ Init  ✓ Snap  ✓ Start  ✓ Running  ◉ Cancel  ·  Persist                          │
│                                                                                              │
│   點 [Force kill]                                                                            │
│   ┌──────────────────────────────────────────────────────────────────────┐                  │
│   │ Force-kill workers?                                                  │                  │
│   │ ─────────────────────────────────────────────────────────────────── │                  │
│   │ Partial outcomes may be lost. This cannot be undone.                │                  │
│   │ Type "refu" (first 4 chars of exec name) to confirm:                │                  │
│   │ [____]                                          [Cancel] [Force kill]│                  │
│   └──────────────────────────────────────────────────────────────────────┘                  │
```

### W-6 失敗列(展開一列)

```
│  ┌─Live──Failed rows──Errors by code──Artifacts──────────────────────────────────────────┐  │
│  │  Errors: BILLING_NOT_FOUND 342  ·  DB_ERROR 38  ·  MISSING_BILLID 7      ⊙ Reveal     │  │
│  │  ┌────────────────────────────────────────────────────────────────────────────────┐   │  │
│  │  │ seq    row    kind    error_code         message                   dur_ms     │   │  │
│  │  │ ───── ───── ─────── ─────────────────── ─────────────────────────── ────────── │   │  │
│  │  │ 102    105   ● err   BILLING_NOT_FOUND   no billing row for billid       14   │   │  │
│  │  │ ▼ 198  201   ● err   DB_ERROR            connection timeout            1240   │   │  │
│  │  │   ┌──────────────────────────────────────────────────────────────────────┐    │   │  │
│  │  │   │ raw_record                                                           │    │   │  │
│  │  │   │ {                                                                    │    │   │  │
│  │  │   │   "id": "rec_201",                                                   │    │   │  │
│  │  │   │   "billid": "b0042",                                                 │    │   │  │
│  │  │   │   "channel": null                                                    │    │   │  │
│  │  │   │ }                                                            [Copy]  │    │   │  │
│  │  │   └──────────────────────────────────────────────────────────────────────┘    │   │  │
│  │  │ 241    244   ● err   BILLING_NOT_FOUND   no billing row for billid       11   │   │  │
│  │  │ ...                                                                            │   │  │
│  │  └────────────────────────────────────────────────────────────────────────────────┘   │  │
│  │  Showing 1–200 of unknown        [ Load more ]              [ Retry failed only ▸ ]   │  │
│  └────────────────────────────────────────────────────────────────────────────────────────┘ │
```

### W-7 空 workspace 狀態

```
┌──────────────────────────────────────────────────────────────────────────────────────────────┐
│  ◇  billing-workspace ▾    Executions                                              + New     │
├──────────────┬───────────────────────────────────────────────────────────────────────────────┤
│ ● Executions │                                                                               │
│              │                                                                               │
│              │                                  ▭ ▭                                          │
│              │                                Inbox                                          │
│              │                                                                               │
│              │                         No executions yet.                                    │
│              │                Start by creating one — or run                                 │
│              │                rowforge exec start in a terminal.                             │
│              │                                                                               │
│              │                       [ + New execution ]                                     │
│              │                                                                               │
│              │                Or [ Open a different workspace ]                              │
│              │                                                                               │
└──────────────┴───────────────────────────────────────────────────────────────────────────────┘
```

### W-8 Orphan attempt banner(不確定、閒置 ≤ 5 min)

```
│  Attempt #3   ⚠ Possibly running externally                                                  │
│  ┌──────────────────────────────────────────────────────────────────────────────────────┐   │
│  │ ⚠  This attempt may still be running externally (e.g. via the CLI).                  │   │
│  │    State shown below may be stale.                                                   │   │
│  │                                            [ Refresh ]    [ Mark aborted manually ]  │   │
│  └──────────────────────────────────────────────────────────────────────────────────────┘   │
```

這些線框非規範。它們是草圖。**規範部分**是 §7.3(頁面樹)、
§7.5(色彩 token)、§7.7(邊界狀態)、§7.10(禁止事項)。

## 7.14 開放問題

1. **Active runs UI 在高並發時。** v1 上限 3（第 3 部分 §3.4),
   header pill 夠用。若上限提高,pill 是否變成帶搜尋的 popover？
   等使用者撞到上限再說。
2. **v2 索引之前的失敗列 filter UI。** 不依靠索引而對 `error_code`
   篩選需要全掃。提供「可能很慢」動作,還是延到 v2（第 4 部分 §4.4)？
3. **macOS App Nap UX 提示。** 規格不要求 opt-out（第 3 部分 §3.8）。
   是否在首次長時間 run 時被動提示「保持視窗在前景以獲得最順暢更新」,
   或留給 docs？
4. **高摩擦 force-kill 的確認 token。** Exec 名稱前綴還是固定字串
   「FORCE KILL」？前者具情境,後者通用但需打更多字。
