# PA-146: wave-37 工具面扩展、write_file 观察门与子代理真实执行引擎

## 状态
- **状态**: Completed
- **负责人**: Dev Team (Lead / Protocol / Test / Executor-A / Executor-B)
- **创建日期**: 2026-10-18
- **闭环日期**: 2026-10-18

## 需求背景
三方对比审计（claude-code 2.1.88 / deepseek-harness / pony-agent-core）定位三类 P0：
模型可见工具面 21 项且基准断言漂移（契约门禁红）、write_file 缺 wave-22 读前写观察门、
子代理 `subagent()` 返回伪字符串。wave-37 以 B 级编排修复。

## 核心交付契约
1. **C1 工具面 21→28**: 注册 Present/Find/GitDiff（Read/Search+workspace.read）与
   JobStart/JobOutput/JobKill/JobList（Execute+workspace.execute），七处映射同步，
   job_start 强制携带 workspace root。
2. **C2 write_file 观察门**: 已存在文件 + 无 expectedHash + 未豁免 → `fs_not_observed`
   fail-closed；`overwrite=false` 语义先于观察门（barrier 修补）。
3. **C3 子代理真实引擎**: SubagentTurnEngine trait + HostTurnEngine（独立建栈）+
   busy/oversize/timeout/panic 错误契约 + OnceLock 槽；`subagent_runtime_wired()` 运行时判定。
4. **基准同步**: tools.rs 契约测试与 provider/mod.rs 两处基准更新为 28 项语义。

## 验收收据
- `cargo check -p pony-agent-core --lib`: EXIT 0
- `cargo test -p pony-agent-core --test wave37_tool_surface`: 9 passed / 0 failed（串行 + 5 连跑并行稳定）
- `cargo test -p pony-agent-core --test wave36_adversarial_classification`: 8 passed / 0 failed
- `cargo test -p pony-agent-core --lib`: 1063 passed / 1 failed（PA-141 wave-36 在飞红相，非本波因果）
- 物理收据: `.dev-team/wave37-{red,exec-a,exec-b}-receipts.md`
- ADR: [`docs/decisions/0039-wave37-tool-surface-write-gate-subagent-engine.md`](../../docs/decisions/0039-wave37-tool-surface-write-gate-subagent-engine.md)

## 遗留（dev-backlog 登记）
- HostTurnEngine 宿主装配（src-tauri/CLI/TUI 显式 `set_subagent_engine`）——未装配前委派能力不上线。
- workflow() 真实现（wave-38）。
- wave-36 PA-138..145 收口（8 卡 pending，含 plan_execution_approval 红相转绿）。
