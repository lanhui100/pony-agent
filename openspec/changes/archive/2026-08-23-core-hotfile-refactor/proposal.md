# Proposal: core-hotfile-refactor

## Why

`crates/pony-agent-core` 的两个热点文件已达反作用规模：`agent/runtime/mod.rs` 15,840 行（内联测试 ~8,780 行，近 60 天改动 26 次为全仓库之最）、`agent/session.rs` 10,916 行（内联测试 ~4,700 行，23 次）。二者叠加导致：高频提交全部挤在同一 diff 面、AI 辅助开发时上下文定位成本过高、会话域"数据模型 / 持久化抽象 / 存储门面"三种变更轴耦合在单文件。本变更为**修改既有能力承载方式**的纯结构重构，不改任何外部可观察行为；执行跟踪卡：`management/task-system/03_TASKS/PA-097-core-hotfile-refactor.md`。

## What Changes

1. **P1-a**：`runtime/mod.rs` 内联 `#[cfg(test)] mod tests { … }` 外移为 `runtime/tests.rs`，原位替换为 `#[cfg(test)] mod tests;` 声明。模块路径不变，非测试区域字节级不动。
2. **P1-b + P2**：`session.rs` 转为 `session/` 目录：`mod.rs`（声明 + 分级重导出）、`types.rs`（数据模型/常量/类型别名/EnvironmentInfo）、`backend.rs`（SessionBackend trait + PersistCommand 族）、`file_backend.rs`（File/Memory 后端）、`store.rs`（SessionStore 与辅助函数）、`tests.rs`（外移测试）。经 `pub use`/`pub(crate) use` 保持 `crate::agent::session::*` 全部既有可见性与路径。
3. 实施约束：脚本按行号/标记切割，禁止手抄搬移；两 Line 文件集合不相交，可并行。

## Scope

- 改动：上述两个文件（其一转目录）及新建子文件
- 零改动承诺：所有调用方（control_plane、runtime、sqlite_session、src-tauri 等）、serde 序列化格式、测试断言

## Non-Goals

- 不拆 runtime 生产代码（P3 另立 spec）
- 不改任何函数签名、类型定义内容、可见性放宽超出等效重导出所需
- 不引入新依赖、不触碰 PA-096 在途前端未提交文件

## Risks

- 切割边界错位 → 以括号平衡校验 + 编译器兜底
- 文件私有项移动后失联 → 可见性策略（pub(super) 最小放开 + 重导出清单核对）
- 测试经 `use super::*` 引用私有项 → mod.rs glob/定向重导出保持解析

## Verification

基线全量测试先行记录红绿；每 Line 独立过 `cargo check -p pony-agent-core`；终门禁 `npm run cargo:test` 对比基线零新增失败 + 下游 `cargo:check` 干净。
