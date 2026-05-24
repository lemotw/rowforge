# 第 1 部分 — 概述

## 1.1 目的

`rowforge-studio` 是 `rowforge` CLI 的桌面 GUI。它存在的兩個使用者目標：

1. **管理執行（exec）** — 啟動、觀察、取消、匯出 `rowforge exec` 已支援的
   逐列批次任務，免於切換到終端機。
2. **撰寫 handler** — 鷹架、編輯、驗證、煙霧測試與打包驅動這些執行的
   handler 程式。

v1 里程碑端到端涵蓋目標 (1)。目標 (2) 分階段在後續里程碑推進，並以錨點
（型別、hook、檔案佈局）的形式貫穿本規格書，v1 不得讓未來無法落地。

## 1.2 原則

- **延伸而非包裝。** `rowforge-studio-core` 是 `rowforge-core` 的延伸。
  任何對 CLI 也有用的能力都應下沉到 `rowforge-core` 共用。`studio-core`
  只放 CLI 不需要的能力。
- **不為第二個消費者設計。** TUI / web / 遠端前端不在範圍內。公開介面
  專為單一 Tauri app 設計。
- **真相在磁碟上。** Studio 是 CLI 磁碟 artifact（SQLite registry、
  `outcomes.jsonl`、`meta.json` 等）之上的檢視器與啟動器。Studio 不會
  虛構出 CLI 無法重現的資料。
- **Studio 與 CLI 共用同一個 workspace。** 任意時刻它們看到相同的狀態,
  除了 Studio 的記憶體快取以外（第 4 部分）。
- **串流，不要載入。** 沒有任何投影會把整份 `outcomes.jsonl` 載入記憶體。
  失敗列瀏覽分頁；rollup 採串流。
- **Core 與 Tauri 解耦。** `studio-core` 內不出現 `tauri::` 型別。Tauri
  層只是薄薄的膠水：參數轉換、IPC、事件發送。

## 1.3 架構概覽

```
apps/rowforge-studio (Tauri + React)
        │
        │  薄膠水: commands.rs
        ▼
crates/rowforge-studio-core   ← Tauri 無關；v1 範圍
        │
        │  只透過公開 API 消費
        ▼
crates/rowforge-core          ← engine, 不動 + 少量上抬
```

「少量上抬」到 `rowforge-core` 是刻意安排：workspace 探索、SQLite registry
開啟 / migrate、`compute_resolution`、manifest 驗證、磁碟 artifact 解析
都是 CLI 與 studio 共同立足點，理應放在 core。見第 5 部分 §5.1。

## 1.4 範圍

### v1 包含
- 執行管理：list、show、start、run、attempts、attempt 細節、cancel、export。
- 即時進度顯示：進度條 + 最近事件尾巴。
- 失敗列檢視：分頁，未來可選擇性篩選（第 4 部分 §4.4）。
- 多執行並發（有上限；第 3 部分 §3.4）。
- 孤兒 attempt 的崩潰恢復（第 3 部分 §3.7）。

### v1 已留錨點、後續實作
- 已完成 attempt 的事件串流重播（第 6 部分 §6.4）。
- 結構化 manifest 編輯器與從 Studio 跑 `rowforge pack`(見第 8 部分
  §8.9 延後清單)。

> Handler 撰寫已進入 v1 — 探索、編輯器啟動、scaffold、build、smoke test。
> 見**第 8 部分**。§1.4 對 handler 撰寫的「已留錨點」立場被取代。

### 不在範圍內
- 多 workspace registry、遠端 workspace、daemon 模式。
- 跨執行分析、BI 視圖、排程 run。
- 超出 Tauri 預設的 i18n / 主題 / 無障礙調整。
- 側車 runner 進程。v1 為 in-process（第 3 部分 §3.1）。

## 1.5 與 CLI 的關係

| 主題 | CLI | Studio |
|---|---|---|
| Workspace 擁有權 | 讀寫 | 讀寫 |
| Schema 遷移 | 擁有 | 拒絕開啟更新的 schema |
| `outcomes.jsonl` | 寫入（append-only） | 只讀 |
| `executions.db` | 讀寫 | 只讀 |
| `outcomes.idx`（未來） | 增量寫入 | 讀取；缺失時重建 |
| 與 CLI 的並發 | 先寫者勝 | 透過 mtime 偵測外部變更 |

使用者可以在 Studio 開啟時於終端機執行 `rowforge exec run`；契約是
Studio 中下一次互動就會顯示新狀態（第 4 部分 §4.5）。

## 1.6 非目標（明列）

- **不重新實作 CLI 邏輯。** 行為若已存在於 CLI，Studio 透過
  `rowforge-core` 引導過去。
- **不為 Tauri 而生的抽象。** 若為了「讓 Tauri 開心」而加入抽象,
  它住在 Tauri crate，不進 `studio-core`。
- **不過早做多消費者設計。** 出現第二個前端那天再重構。
