# Tauri + Rust 重构说明（历史存档）

> **状态: 已归档**
> 本文档记录项目早期（2026-05）从 Python Hermes 到 Tauri + Rust 重构的初始设计方案。
> 当前代码结构已被 PA-044 等后续重构显著改变：
> - core 已迁入独立 workspace member `crates/pony-agent-core`
> - Tauri 壳层位于 `src-tauri/`（而非早期规划的 `apps/desktop-tauri`）
> - 完整阶段路线见 [重构阶段计划](roadmap/phases.md)
> - 当前项目结构见 [AGENT.md](../AGENT.md)
