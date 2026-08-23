# Proposal: runtime-mod-structured-split

## Why

`crates/pony-agent-core/src/agent/runtime/mod.rs` 已增长至约 16,588 行（653 KB），是全仓库最大热点文件。构成：`impl AgentRuntime` 约 4,055 行（31 个方法）、自由函数约 2,510 行、内联测试 `mod tests` 约 9,525 行（111 个测试，**词法上一直延伸到文件尾，嵌套其中的 `mod parity` 约 1,015 行随迁**）。同目录已有 6 个子模块（turn_prep/tool_exec/turn_persist/turn_runner/ingress_mediation/session_ops），渐进式拆分方向早已确立，但主文件未跟进。持续后果：任何一行改动触发整文件级 diff 与下游重编、AI 辅助开发上下文定位成本过高、多人协作合并冲突集中爆发。

本变更为**纯结构性重构**：零行为变更、零签名变更、外部可观察契约不变。它是 PA-097（core-hotfile-refactor）明确推迟的"P3"部分的落地，执行跟踪卡：`management/task-system/03_TASKS/PA-098-runtime-mod-structured-split.md`。

## What Changes

1. **§1 测试外移（沿 PA-097 Line A 已评审设计执行）**：内联 `#[cfg(test)] mod tests { … }` 整体外移为 `runtime/tests.rs`（实测其词法范围延伸至文件尾，嵌套其中的 `mod parity` 一并随迁——parity 虽写在列 0 但实为 `tests` 子模块、继承 cfg(test)）；原位替换为 `#[cfg(test)] mod tests;` 声明。模块路径与编译条件零变化。
2. **§2 P1 自由函数子模块化**：按职责将生产区自由函数族移入新子模块：`limits.rs`（上限/env 覆盖）、`planner_patches.rs`（hook/graph decision patch 解析与应用）、`blocked_records.rs`（blocked/skill 调用记录构建）、`trace_timeline.rs`（trace 时间线/transcript 构建）、`tool_recovery.rs`（followup 恢复/本地 fallback）、`stream_support.rs`（流式批处理/去重签名/连续失败止损/cache 记录/token 合并）。类型与常量入 `types.rs`，builder 入 `builder.rs`。
3. **§3 P2 impl 分块迁移**：`impl AgentRuntime` 按职责拆为跨文件 impl 块：`turn_stream.rs`（handle_stream_tool_turn + start_turn_stream*）、`turn_sync.rs`（handle_sync_tool_turn + run_turn 族）、`turn_control.rs`（cancel/suspend/bind_ask_wait）；构造器、访问器、trace 管道方法留在 mod.rs。**两个巨型方法（~1,190/~1,390 行）整体平移，不在本变更内做方法内部分解。**
4. 终态：`mod.rs` 收缩为门面（目标 <500 行）：子模块声明 + AgentRuntime 结构体本体 + 构造器/访问器 + `use`/重导出胶水。

## Scope

- 改动仅限：`crates/pony-agent-core/src/agent/runtime/` 目录内文件
- 零改动承诺：所有调用方零 diff——外部依赖面实测为 **7 项**（control_plane 导入 AgentRuntime/AgentRuntimeBuilder/RunTurnFacts/TurnInput/TurnResult/TurnStreamEvent/SUSPENDED_TURN_PHASE；bin/non_tauri_harness.rs 导入 AgentRuntimeBuilder；src-tauri 探针 bin 导入 AgentRuntime），经 mod.rs 重导出保持路径与可见性不变
- 函数体逐字节平移（除缩进与编译器必需的可见性关键字），禁止顺手修改任何逻辑、命名、格式

## Non-Goals

- 不做方法内部分解（两个千行方法的拆分另立后续任务）；**如实登记残余热点：turn_stream.rs 迁入后约 2.7k 行**，收口摘要不得夸大改善幅度
- 不动 session.rs（PA-097 Line B 另行执行）
- 不改 serde、不改既有 6 个子模块的内部结构、不修正 parity 的缩进格式债（随迁保形）、不清理 4942–5205 块注释死代码（原样随迁 blocked_records）
- 不引入新依赖、不触碰 PA-096 在途前端未提交文件、不擅自 commit

## 对 PA-097 Line A 的偏差声明

PA-097 design 要求测试外移"严禁 dedent"，本变更改为 dedent 一级。依据：本文件实测 raw string 仅 2 处且均为单行（11018/11060），dedent 字节安全；该偏差已写入 design.md 前置事实并经双路审核确认可行。

## Risks

- 测试 `use super::*` 引用私有项失联 → mod.rs 私有 `use 子模块::*` 胶水绑定 + 编译器驱动修复循环（Rust 可见性规则：父模块私有绑定对子孙模块可见）
- 跨子模块引用私有项 → 最小放宽 `pub(super)`，禁止越界放宽
- 切割边界错位 → 一律脚本按标记定位 + 行数/字节守恒断言 + 编译器兜底；**禁止硬编码行号**（本次已实测行号相对上一会话漂移 ~750 行，且 parity 嵌套真相系探测发现，缩进不可信）
- 大体积移动引入手抄错误 → 全程脚本切割，LLM 只定边界与校验
- 仓库该文件未严格 rustfmt → 不可依赖"列 0 闭括号"等格式约定，边界断言以内容守恒为准

## Verification

1. 基线先行：`cargo test -p pony-agent-core` 重构前红绿名单（已完成，exit 0 全绿）
2. 每 § 结束：`cargo check -p pony-agent-core` 通过 + `git status` 确认改动仅限 runtime 目录 + 抽样运行 runtime 测试
3. §1 后额外：非测试区域 `git diff` 为空（除尾部声明行）+ 切出行数与删去行数守恒断言
4. 终门禁：`npm run cargo:test` 对比基线零新增失败 + `npm run cargo:check` 下游干净 + rustfmt 默认口径核验
