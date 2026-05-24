# 第 4 部分 — 資料

描述 Studio 讀取的磁碟 artifact、如何由它們導出投影、快取策略、選用的
側車索引、以及 schema 版本管理。CLI 產生的磁碟佈局見
[`../../cli/part-4-data.md`](../../cli/part-4-data.md);本部分引用之。

## 4.1 來源 artifact（讀取面）

| Artifact | 來源 | Studio 如何讀取 |
|---|---|---|
| `executions.db` (SQLite) | CLI 寫入 | 只讀的 registry 查詢 |
| `executions/<e>/manifest.json` | CLI 寫入 | `ExecDetail` 鏡像;與 SQLite 重複 |
| `executions/<e>/attempts/<r>/meta.json` | CLI 在 terminal 狀態寫入 | `AttemptDetail.stats`、`by_error_code` |
| `executions/<e>/attempts/<r>/outcomes.jsonl` | CLI 在 run 中串流 | 掃描以提供 `FailedRowPage`、`ExecRollup`、`RowHistory` |
| `executions/<e>/attempts/<r>/handler-snapshot/` | CLI 在 attempt 起始寫入 | 僅透過「在 Finder 顯示」檢視 |
| `executions/<e>/exports/<ts>/resolution.json` | CLI 在 export 時寫入 | 有 export 後才讀取 |

Studio 不寫入 `start_exec` 與 `start_run` 間接（透過 `rowforge-core`）
所產生以外的 artifact。

## 4.2 讀取策略

對第 2 部分中的每個投影：

| 投影 | 策略 | 最壞成本 |
|---|---|---|
| `Workspace` | 開啟 SQLite，讀 `schema_version` | 常數 |
| `ExecSummary` | 一次 SQLite 查詢 + 一次最新 attempt 的 `meta.json` 讀取 | 每執行常數 |
| `ExecDetail` | 一次 SQLite 查詢 + N 次 attempt-summary 查詢 | attempts 數線性 |
| `AttemptDetail` | 一個 SQLite row + 一個 `meta.json` | 常數 |
| `ExecRollup` | 摺疊每個 attempt 的 `outcomes.jsonl` 串流 | outcomes 總數線性 |
| `FailedRowPage`(v1) | 從 `offset` 線性掃描 | `offset + limit` 線性 |
| `FailedRowPage`(v2,有索引) | 透過 `outcomes.idx` 定位 | 常數 + 一頁 IO |
| `RowHistory` | 每個 attempt 掃一次，僅失敗列 | 失敗列數線性 |

「串流摺疊」是 `tokio::task::spawn_blocking` + 帶緩衝的逐行讀取;
outcomes 採惰性解析（僅反序列化投影所需欄位）。任何投影都不會把整份
解析後的 `outcomes.jsonl` 物化到記憶體。

## 4.3 快取

三層，並有明確失效機制：

### Hot — 永遠快取
- `Workspace` 及其 `schema_version`。
- SQLite 連線池。

失效：僅在進程重啟時。

### Warm — 以 mtime + TTL 快取
- `ExecSummary` 清單。
- 僅 **terminal** attempts 的 `ExecDetail` 與 `AttemptDetail`。

失效：
- 回傳快取項前對來源 artifact 做 mtime probe。陳舊則丟棄並重讀。
- 不論 mtime，TTL 上限 30 秒,以抓住粗粒度錯誤。
- 顯式刷新：使用者觸發的 refresh 按鈕、Tauri `WindowEvent::Focused`、
  run 完成通知。

### Cold — 永不快取
- `ExecRollup`。
- `FailedRowPage`。
- `RowHistory`。
- 任何進行中 attempt 的 `AttemptDetail`。

錯誤代價：在 CLI 啟動的 run 外部完成後顯示陳舊計數，比慢一點刷新更快
侵蝕使用者信任。因此 warm 層的 mtime probe 是**必須**的（不是「有最好」）。

檔案系統 watcher（`notify` crate、FSEvents/inotify/ReadDirectoryChangesW）
在 v1 明確**不**使用：macOS 電池成本、平台脆弱性、與 mtime probe 已能
在單次使用者互動內抓到外部變更時的複雜度，皆不划算。

## 4.4 側車索引（`outcomes.idx`）

### v1 狀態
非必要。v1 對 `FailedRowPage` 與 `ExecRollup` 採線性掃描。1 GB 的
`outcomes.jsonl` 在 SSD 上線性掃描約 5–10 秒;這對使用者點擊後有
spinner 是可接受的，但每個 UI tick 都掃描就無法接受。

### v2 格式（在這裡固定下來，使其落地時格式已定）

固定大小的記錄檔，little-endian，**每 outcome 24 bytes**，與
`outcomes.jsonl` 同目錄：

```
record (24 B): {
    seq:           u32,
    byte_offset:   u64,    // outcomes.jsonl 內位移
    line_offset:   u32,    // 一列一 outcome 時為 0，否則為 batch 行內位置
    outcome_kind:  u8,     // 0=success 1=error 2=crash 3=too_large
    error_code_id: u16,    // 每 attempt 內 intern
    _pad:          u8,
    dur_ms:        u32,
}
trailer (16 B): { magic: b"RFIDX01\0" (8 B), outcomes_jsonl_size_at_finalize: u64 }
companion file: error_codes.txt        // 每行一個 code，行號 = id
```

### 擁有與生命週期
- 在 run 期間由 **`rowforge-core` 寫入**（廉價的增量 append）。
- 在 `PERSISTING` 階段以原子 `rename` 就位。
- 部分 / 中止的 run 留下 `outcomes.idx.tmp`；Studio 下次開啟時重建。
- Magic-byte 版本鎖：`RFIDX01` 為 v1；未來升至 `RFIDX02`，舊 Studio
  重建。

### 陳舊偵測
trailer 的 `outcomes_jsonl_size_at_finalize` 必須等於 `outcomes.jsonl`
的即時大小。不符 → 丟棄重建。

### 索引缺失時
Studio 按需重建（在 `spawn_blocking` task 內，UI 顯 spinner）。重建結果
與 CLI 寫的等同。

### 反方論點
若 GUI 的失敗列瀏覽願意**只掃描、不篩選、不 seq 定位**，則不需要索引。
資料流事件（第 6 部分）已涵蓋進行中 attempt 的情境。v1 採此立場，v2 解除。

## 4.5 外部變更（Studio 開著時 CLI 在跑）

契約：Studio 下次使用者主動讀取時顯示新狀態。機制：
- Warm 層 mtime probe 抓住完成的 CLI run。
- 視窗 focus 事件觸發清單刷新。
- Studio 不主動輪詢。

Studio 不試圖把 CLI 進行中的 run 當作即時事件串流觀察。那是延後的
`watch.rs` 能力（見第 6 部分 §6.4）。

## 4.6 Schema 版本管理

三類 artifact，三套契約：

### SQLite `executions.db`
- 硬鎖：`schema_version` 必須 ≤ Studio 已知最大值。
- 更高 → Studio 拒開，回明確錯誤「此 workspace 由較新版 rowforge 寫成；
  升級 Studio」。
- 更低 → Studio 拒開（無 compat shim）。使用者同步升級 core 或 Studio。
- 遷移由 CLI 獨佔。

### JSON metadata（`meta.json`、`manifest.json`、`resolution.json`）
- 容忍子集 reader：缺欄位以 `#[serde(default)]` 處理、未知欄位靜默丟棄、
  已知欄位型別不符則硬失敗。
- 每份 JSON 頂層有 `schema_version: Option<u8>`，存在時可降級顯示並表示
  已知版本。

### `outcomes.jsonl`
- 每行寬鬆解析：未知 `type` 判別子合成為「未知 outcome」並計入
  `failed_last`（安全起見，絕不計入 `resolved`）。未知 error code
  作為字串透傳。
- 自 v3.4（CLI 決策 D13）以來格式穩定。

### `outcomes.idx`（若存在）
- 嚴格 magic-byte 鎖（§4.4）。版本不符不會致命 — 索引可重建。

## 4.7 跨 attempt 解決（即時）

`ExecRollup` 使用 [`../../cli/part-2-model.md`](../../cli/part-2-model.md)
中定義的 `RowResolution` 規則跨所有 attempts 摺疊。摺疊邏輯位於
`rowforge-core::compute_resolution`;`studio-core` 透過僅計數的入口
呼叫,不物化 canonical-success 映射。

每列的 `RowHistory` 按需計算,做法是讀每個 attempt 的 `outcomes.jsonl`
找對應 `seq`。有 v2 索引時每 attempt 常數成本;沒有則為線性。

矩陣視圖（每列 × 每 attempt）刻意不建。見第 2 部分 §2.3。
