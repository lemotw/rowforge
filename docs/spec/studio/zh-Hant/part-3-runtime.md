# 第 3 部分 — Runtime

描述 run 在 Studio 下如何執行：進程模型、狀態機、並發策略、取消語意、
崩潰恢復、handler 子進程清理。CLI 端 runtime（worker pool 內部、
dispatch loop、batch 協定）見
[`../../cli/part-3-runtime.md`](../../cli/part-3-runtime.md)；
本部分引用但不重複。

## 3.1 進程模型 — in-process

Studio 在 Tauri 主進程的 tokio runtime 內執行 `rowforge-core` 流水線。
v1 沒有側車 runner 進程。

風險圍堵：

- **Panic 隔離。** 每個 run 層級的 `tokio::spawn` 都透過 `JoinHandle`
  await，其 `JoinError::is_panic` 路徑被映射為
  `ProgressEvent::Aborted { reason: AbortReason::Crashed { panic_message } }`。
  Panic 不會傳播到進程根。
- **CPU 隔離。** Tokio 設定為 multi-threaded。`studio-core` 內的 CPU
  繁忙工作（CSV 解析、`outcomes.jsonl` 掃描）走 `tokio::task::spawn_blocking`,
  避免拖累 UI command 所用的 reactor。
- **記憶體上限。** 每個 run 預設 `max_in_flight = workers × 2`。
  Queues 有上限；流水線在 dispatch 處施加反壓。

不在範圍：handler 子進程的 native crash（segfault）無法把 Studio
拖下水，因為 handler 跑在自己的進程中。`rowforge-core` 自身的 native
crash 視為 bug，不是設計上的失敗模式。

側車 runner 進程列為 v2 選項，前提是以下任一成立：來自 native handler
的 panic 變常見、繁重 CPU 下的 UI 飢餓難以解、或需要記憶體隔離。

## 3.2 Worker pool 擁有權

Workers（handler 子進程）由 `rowforge-core` 依每個 run 擁有；pool 不跨
run 共用。Studio 強制 `workers × concurrent_runs ≤ logical_cpus × 2`,
若使用者覆寫違反此規則，UI 顯示警告。

## 3.3 Run 狀態機

```
        ┌─────────┐
        │ Pending │  已分配 RunHandle，tokio task 尚未 spawn。
        └────┬────┘
             ▼
        ┌──────────┐
        │ Starting │  Workers spawn 中、handler build / handshake 中。
        └────┬─────┘
             │  首列 dispatch 後
             ▼
        ┌─────────┐  cancel
        │ Running │ ────────────────┐
        └────┬────┘                 ▼
             │                ┌────────────┐
             │                │ Cancelling │
             │                └─────┬──────┘
             │                      │
   pipeline 排乾                     │ token 被觀察，in-flight 排乾
             ▼                      ▼
        ┌──────┐               ┌──────────┐
        │ Done │               │ Aborted  │
        └──────┘               └──────────┘
                              ▲
                              │  run task 內 panic
                              │
                         ┌─────────┐
                         │ Crashed │
                         └─────────┘
```

轉移時的持久化：

- **Pending → Starting**：SQLite 插入 `attempts` row，`state = starting`。
- **Starting → Running**：row 更新為 `state = running`。
- **Running → Done**：outcomes flush 完成、`meta.json` 寫入、SQLite row
  更新為 `state = done` 並寫入最終 stats。`Done` 事件在三者完成**後**
  發送。
- **Any → Aborted**：SQLite row 更新為 `state = aborted` 並寫入部分
  stats；outcomes flush 到最後一個 batch 邊界。
- **Any → Crashed**：盡力與 Aborted 相同，但 `reason = Crashed`。若
  panic 導致無法寫入，§3.7 的恢復會在下次啟動時修正。

即時列計數器（success、failed、in_flight）不會每事件持久化；它們由
`outcomes.jsonl` 按需計算，並由 `ProgressAggregator`（第 6 部分 §6.2）
在記憶體中追蹤。

## 3.4 多 run 並發

預設值（使用者可在 Settings 覆寫）：

| 限制 | 預設 | 理由 |
|---|---|---|
| 每個 execution 的並發 run | 1 | 同一 exec 並發 attempts 會破壞 `RowResolution` 摺疊 |
| 每個 workspace 的並發 run | 3 | 對筆電友善；避免與 sqlite 寫入產生 IO 競爭 |
| 每個 run 的 workers | core 預設 | 與 CLI 相同 |
| `workers × concurrent_runs` | ≤ cpus × 2 | 軟警告強制，硬上限可設定 |

達到單 exec 上限時 `StudioCore::start_run` 回傳
`UiError::RunBusy { execution_id }`；達到 workspace 上限時回傳同變體但
帶 workspace 層級理由。UI 應顯示限制，不應靜默排隊。

## 3.5 取消語意

兩種模式：

### 軟取消（預設）
1. `StudioCore::cancel(handle, CancelMode::Soft)` 設置 core 的
   `CancellationToken`。
2. 流水線停止 dispatch 新列。
3. In-flight 列完成（通常每列 sub-second；以 handler 的單列工作為界）。
4. `outcomes.jsonl` flush 至最後一個 batch 邊界。
5. SQLite row 轉為 `aborted`。
6. 發送 `ProgressEvent::Aborted { reason: UserCancelled, ... }`。

### 硬取消（強制 kill）
僅在軟取消已逾實作定義的閾值（建議 10 秒）後可用。對 handler 子進程
呼叫 `Child::kill()`。可能遺失部分 outcomes；UI 必須明確警告後才能呼叫。

取消過程中的 UI 狀態：
- `RunStatus::Cancelling` 並顯示每秒一次的「n 列尚未完成」進度
- 過閾值後出現具破壞性樣式提示的「強制 kill」按鈕

最糟情況：handler 在單列 dispatch 內陷入無限迴圈。軟取消永遠不會完成；
硬取消是唯一出路。此情況有文件記載；沒有自動升級。

## 3.6 關閉時的資源清理

正常關閉 app 時：

1. `StudioCore::Drop` 走訪 active sessions 並對每個發 `cancel(Soft)`。
2. 每個 session 最多等 1 秒。
3. 仍存活的 worker 強制 kill。

異常結束（crash、OS kill）時：

- macOS / Linux：父進程死亡時子進程被 OS 收割（子進程繼承的預設行為）。
- Windows：除非加入 Job Object，否則子進程**不會**隨父進程死亡。
  `rowforge-core` 必須將 worker 進程放入 Job Object，並設定
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`。（此為 CLI 端修補,Studio 共享；
  住在 core，不在 studio-core。）

## 3.7 崩潰恢復

`StudioCore::open` 時掃描 workspace 找出**孤兒 attempts**：SQLite row
中 `state ∈ {starting, running}` 但其擁有的 Studio 進程已不存在。

偵測啟發法（v1 無 Studio pid 檔）：

| `outcomes.jsonl` mtime | 動作 |
|---|---|
| 閒置 > 5 分鐘 | 自動標記為 `aborted` 並設 `reason = OrphanedOnRestart`；從磁碟 outcomes 寫入部分 stats |
| 閒置 ≤ 5 分鐘 | 不確定（可能是終端機跑的 CLI run）。UI 顯示「可能仍在外部執行」並提供手動標 aborted |

mtime 閾值可由實作調整；規格只要求啟發法存在，且使用者不會被靜默地
看到陳舊 `running` 狀態超過一次。

Studio **不**提供孤兒 attempt 的「resume」。標準的重置動作是在新 attempt
上執行 `rowforge exec run --retry-failed` — 更簡單、可審計、CLI 已支援。

責任劃分：
- 偵測：`studio-core::workspace::open_default` 透過
  `rowforge-core::workspace::scan_for_orphans` 啟動掃描。
- 修補（寫 SQLite + meta）：`rowforge-core::workspace::mark_aborted`。

## 3.8 背景與閒置行為

- macOS App Nap 預設不關閉。Studio 在背景時長時間執行的 attempts 可能
  延遲 UI 更新，但實際工作不會延遲（worker 子進程不受 App Nap 影響）。
- 背景中的 tokio timer 漂移對 run 機制無關（無時間敏感排程）；只影響
  `ETA` 與 `rate_*` 顯示，這些本就用 wall-clock delta。
- 規格不要求 Studio 保持前景。使用者在文件（非規格）中被告知長時間 run
  時把 app 放前景比較順暢。
