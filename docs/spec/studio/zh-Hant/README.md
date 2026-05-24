# rowforge-studio — 規格書

`rowforge` CLI 的姊妹規格（見 [`../../cli/`](../../cli/)）。描述桌面 GUI
（`apps/rowforge-studio`，Tauri + React）以及其後盾 Rust crate
`rowforge-studio-core`。

本規格書的範圍**比 v1 里程碑更廣**。第一個里程碑聚焦於執行管理；後續里程碑
才會加入 handler 撰寫與更豐富的可觀測性。每個章節都會註明哪些屬於 v1、哪些
被延後。

## 章節

1. [`part-1-overview.md`](part-1-overview.md) — 目的、原則、範圍、非目標、與 CLI 的關係
2. [`part-2-model.md`](part-2-model.md) — 實體、投影、衍生視圖
3. [`part-3-runtime.md`](part-3-runtime.md) — 進程模型、run 狀態機、並發、取消、崩潰恢復
4. [`part-4-data.md`](part-4-data.md) — 來源 artifact、快取、側車索引、schema 版本
5. [`part-5-api.md`](part-5-api.md) — `studio-core` API、Tauri commands、錯誤、設定、版本管理
6. [`part-6-observability.md`](part-6-observability.md) — 事件分類、吞吐安全、即時 vs 重播、指標、多 run
7. [`part-7-ui.md`](part-7-ui.md) — 技術棧、設計語言、資訊架構、主要 flow、狀態色、互動模式、邊界狀態
8. [`part-8-handler-authoring.md`](part-8-handler-authoring.md) — Handler 探索、編輯器啟動、scaffold、build、smoke test(取代第 1 部分 §1.4 / 第 5 部分 §5.4 的錨點)

## 配套文件

一份簡短的 v1 實作計畫位於
[`../../../superpowers/specs/2026-05-19-rowforge-studio-mvp-design.md`](../../../superpowers/specs/2026-05-19-rowforge-studio-mvp-design.md)。
該文件是 **MVP 設計**；本規格書是 **目標形態**。若兩者衝突（MVP 範圍較窄），
v1 里程碑以外的工作以本規格書為準。
