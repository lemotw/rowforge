# 第 6 部分 — 可觀測性

描述從進行中的 attempt 到 UI 的事件串流：分類、吞吐安全、即時 vs 重播、
即時指標、失敗診斷、多 run 聚合。

## 6.1 `ProgressEvent` 分類

```rust
#[non_exhaustive]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProgressEvent {
    // 生命週期
    PhaseChanged { phase: Phase, at_ms: u64 },
    WorkerSpawned { worker_id: u32 },
    HandlerReady { worker_id: u32, handler_version: String, startup_ms: u32 },
    WorkerCrashed {
        worker_id: u32,
        last_seq: Option<u64>,
        signal: Option<i32>,
        stderr_tail: String,            // ≤ 64 KiB,溢位時保留首尾
    },
    StallWarning { silent_secs: u32 },

    // 熱路徑進度
    Tick {
        seq: u64,                       // 每 run 內單調;UI 偵測掉包
        at_ms: u64,
        processed: u64,
        total: Option<u64>,
        success: u64,
        failed: u64,
        crashed: u64,
        in_flight: u32,
        queue_depth: u32,
        rate_1s: f32,
        rate_10s: f32,
        eta_ms: Option<u64>,
    },
    OutcomeSample {                     // 取樣;非完整
        row_index: u64,
        kind: RowOutcomeKind,
        code: Option<String>,
        message: Option<String>,
        dur_ms: u32,
    },
    BatchSummary {                      // 僅 batch 模式
        first_seq: u64,
        n: u32,
        success: u32,
        failed: u32,
        dur_ms: u32,
    },

    // 與列失敗區別開
    PipelineWarning { code: String, message: String },
    HandlerStderr { worker_id: u32, line: String },     // 取樣

    // 終止
    Done(RunReport),
    Aborted { reason: AbortReason, at_phase: Phase, partial_report: RunReport },
}

enum Phase {
    Initializing, Snapshotting, Starting, Running, Cancelling, Persisting
}
```

非必要變體（`PhaseChanged`、`WorkerSpawned`、`HandlerReady`、
`WorkerCrashed`、`StallWarning`、`PipelineWarning`、`HandlerStderr`、
`BatchSummary`）的 UI 渲染屬於選用。事件存在是為了未來加 UI 時不必
破壞 enum。

`OutcomeSample` 明定為可丟失。需要每個 outcome 者請讀 `outcomes.jsonl`。

## 6.2 吞吐安全（合併）

10K rows/sec 時每列事件相當於每 100 µs 一個。naive 的 `broadcast::Sender`
在 100 ms 內就溢位;React reconciliation 更早就崩潰。因此合併發生於
`studio-core` 內、broadcast 送出**之前**,於一個小型 `ProgressAggregator`。

發送預算：

| 事件 | 預算 | 策略 |
|---|---|---|
| `Tick` | 4 Hz（每 250 ms） | 由 wall-clock timer 與 count-delta 閾值共同驅動;接收端分辨不出 |
| `OutcomeSample` | 20/秒 | token-bucket;90% 預算保留給 errors/crashes,10% 給 successes |
| `HandlerStderr` | 20/秒/worker,單行 ≤ 2 KiB | 突發溢位合併為 `"... n more lines dropped"` |

Channel 大小：
- `broadcast::channel(256)`。落後的接收端發出一則
  `PipelineWarning { code: "EVENT_LAG", message: "n events dropped" }`
  並繼續。

取消 + backlog：
- 取消是獨立的 `invoke("run_cancel", ...)` 呼叫,絕不走事件串流回程。
  它直接觸發 `CancellationToken`,不受事件串流深度影響。
- `Aborted` / `Done` 時 forwarder 在取消訂閱前再排放一次最終 `Tick`,
  讓使用者即使先前 Tick 有掉包,也一定看到最終計數。

UI 在 10K rows/sec 持續時看到：
- 4 Hz Tick 驅動的平滑進度條。
- 1 秒 / 10 秒速率讀數。
- 200 筆 `OutcomeSample` 的 ring buffer,以 error 為主。
- 永遠看不到「每一列」。那是 `outcomes.jsonl` 的職責。

延遲下限：250 ms 視覺延遲。10K rows/sec 時為 2500 列的可見延遲。
桌面 GUI 可接受。

## 6.3 流水線中合併發生的位置

`rowforge-core` 是真相來源;它不能掉 outcome。它把細粒度事件送進
`ProgressSink` trait。CLI 的 sink 寫 stderr 行（今日行為）。Studio 的
sink 是 `ProgressAggregator`,它：

1. 接收每個 outcome（這是持久的計數）。
2. 更新內部計數器 / 速率緩衝 / 每錯誤碼直方圖。
3. 250 ms tick 時送出 `Tick`。
4. error / crash outcome 時跑 token-bucket 取樣,可能送出
   `OutcomeSample`。

如此把合併留在 `rowforge-core` 之外（CLI 不動)、Tauri 層之外
（其情境不足）。

## 6.4 即時

Tauri 層透過 `core.subscribe(handle)` 直接訂閱進行中的 attempt，
回傳 aggregator 的 broadcast receiver。`snapshot()` 回傳 aggregator
當前計數器；`events()` 為 broadcast receiver。用於 run 仍存活於此
Studio 進程內時。

### 6.4.1 React 訂閱啟動補包(snapshot fallback)

Tauri 的 `app.emit(channel, payload)` 是 fire-and-forget — 在
webview listener 掛載之前發出的 payload 會被丟棄,**不會 queue**。
React `useRun` hook 因此不能只靠 `listen("run:<handle>")`:
`run_start` 回傳到 `listen()` 真正生效之間(實測 50–300 ms),
那段時間的事件全部會掉。

**Bootstrap 協定:**

1. `useRun` 先掛 `listen()`,之後到達的事件全部進 reducer。
2. `listen()` 掛好後立即呼叫 `run_snapshot(handle)`,把回傳的
   `ProgressSnapshot` 用合成 action `_bootstrap` 一次性套到
   reducer(counter / phase / status)。
3. 步驟 1、2 之間到達的真正事件正常累積到 state。`_bootstrap`
   可能會用稍早的 snapshot 暫時覆蓋幾個欄位;下一次真實 `Tick`
   (≤ 250 ms)會把數字校正回來。
4. 若 `run_snapshot` 回 `UnknownHandle`(run 在 listener 掛好之前
   就結束了,sub-200 ms 的 run 常見),hook 派發
   `_terminal_before_listen` 把 `phantomBootstrap = true`。頁面
   反應:隱藏 Live tab、refetch `attempt_show`、切到 Summary。

協定不暴露在 Tauri command surface;是 React `useRun` hook 加上
支援指令 `run_snapshot` 跟 `attempt_active_handle`(第 5 部分 §5.5)
的特性。Tests 鎖在
`apps/rowforge-studio/src/__tests__/run-state.test.ts`。

## 6.5 失敗診斷

`Aborted` 攜帶結構化情境：

```rust
enum AbortReason {
    UserCancelled,
    HandlerStartupTimeout { failed_workers: u32, last_stderr: String },
    AllWorkersCrashed { crashes: Vec<WorkerCrashRecord> },
    Stalled { silent_secs: u32, last_seq: Option<u64> },
    MissingRequiredInput { columns: Vec<String> },
    SnapshotHashMismatch { path: PathBuf, expected: String, actual: String },
    OrphanedOnRestart,
    Crashed { panic_message: String },
    Internal { message: String },
}

struct WorkerCrashRecord {
    worker_id: u32,
    last_seq: Option<u64>,
    exit_code: Option<i32>,
    signal: Option<i32>,
    stderr_tail: String,                // ≤ 64 KiB
}
```

Handler stderr 雙 sink：
- **即時 tail** 以 `HandlerStderr` 事件送 UI(按 §6.2 取樣)。
- **持久檔** `attempts/<id>/handler.stderr.log`,append-only,無速率
  限制。UI 提供「開啟 log」,路徑在 `AttemptDetail::paths`。

列中 crash vs handler 自報失敗：wire protocol 已區別
(`type=error` 為自報,`type=crash` 為列中死亡;CLI 第 4 部分 §7.4)。
Studio 保留此區別：
- `error` outcome → `OutcomeSample { kind: Error, ... }`。
- `crash` outcome → `OutcomeSample { kind: Crash, code: "WORKER_CRASH" }`
  **並**送一筆 `WorkerCrashed` 生命週期事件,附 stderr tail。

## 6.6 多 run 聚合

隔離不變量：
- 每個 `RunHandle` 各自有 broadcast channel。
- 每個 handle 各自有 Tauri 事件名:`"run:<handle>"`。
- aggregator 狀態跨 run 不洩漏。

跨 run 聚合（`runs:active`）：

```rust
struct RunRollupTick {
    active_runs: u32,
    total_processed: u64,
    total_failed: u64,
    total_rate: f32,
    slowest_run: Option<RunHandle>,
}
```
1 Hz 發送。由 `SessionRegistry` 輪詢每個 session 的 aggregator snapshot
組成 — 嚴格僅計數視圖,沒有任何單列資料跨 run。供全域標頭 / dock badge
與「執行中」下拉使用。

明確**不**提供：
- 跨 run 合併時間線 / 比較視圖（不在範圍;BI 領域）。
- 跨 Studio 重啟的持久 active-runs roll-up(`executions.db` registry
  已記錄終止狀態)。

## 6.7 即時指標

永遠送出的計數器（廉價）：
- `processed`、`success`、`failed`、`crashed`。
- `in_flight`、`queue_depth`。
- 250 ms tick 時自取樣 ring buffer 計算的 `rate_1s`、`rate_10s`。
- `eta_ms` = `(total − processed) / rate_10s`;10 秒緩衝填滿前顯示「—」。
- Worker 利用率（每 worker `busy_ms / total_ms`;若廉價可在 `Tick` 中
  聚合）。

可選、不在 v1：
- 每列延遲直方圖（HDR-histogram）。每插入 sub-µs 成本,但每 tick 百分位
  快照記憶體開銷高於計數器。加之需 `RunOpts::observe_latencies` flag
  與額外事件變體;v1 不含。

## 6.8 開放問題

1. 非列事件的 `progress.jsonl` 是否值得儘早排程,或等到需求浮現？
2. Stderr ring 策略：首+尾 vs 連續？影響「感覺被截斷」vs「感覺有遺失」。
3. `runs:active` 應該在 Studio 重啟後透過重掃 SQLite 找 `state = running`
   的 row 而存活,還是嚴格留在記憶體？答案與 §3.7 崩潰恢復互動。
