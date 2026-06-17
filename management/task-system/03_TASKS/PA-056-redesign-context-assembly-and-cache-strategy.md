# PA-056 重构上下文构建与缓存命中策略：System Prompt / AGENT.md / Workspace / Memory Hooks

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change
- 归档路径：
  [2026-06-16-redesign-context-assembly-and-cache-strategy](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-16-redesign-context-assembly-and-cache-strategy>)

## Delta Spec
- 归档路径：
  [context-assembly-and-cache-strategy/spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-16-redesign-context-assembly-and-cache-strategy/specs/context-assembly-and-cache-strategy/spec.md>)

## Canonical Spec
- 已同步到：
  [context-assembly-and-cache-strategy/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/context-assembly-and-cache-strategy/spec.md)

## Spec 状态
- Proposal: `archived`
- Spec: `archived`
- Design: `archived`
- Tasks: `archived`

## 背景
当前项目已经建立了 `BuildContextObservation`、`stable_prefix / semi_stable_context / volatile_input` 等观测口径，但真实请求构建仍存在以下问题：

- `system + capability note + semistable context + history + user` 每轮重建
- `Session summary / Run goal / Long-term memory / Planner skills / Image note / Truncation note` 被放在历史前部
- provider-native transcript 在 tool flow 中仍以全量重放为主
- `AGENT.md`、workspace 作用域、长期记忆入口、system prompt profile 尚未统一分层

这导致当前缓存命中长期停留在 `40%-60%`，明显低于 `Reasonix / Codex / Claude Code` 一类实现的常见区间。

## 目标
建立一套统一的上下文构建架构，使后续 system prompt、project instructions、workspace 作用域、conversation carry 与长期记忆扩展都能在同一套缓存友好边界下演进。

## 输出
- `redesign-context-assembly-and-cache-strategy` OpenSpec change
- system prompt / runtime facts / project instructions / memory / conversation carry / volatile input 的正式分层 spec
- `AGENT.md` / workspace instructions 的作用域与注入规则
- 长期记忆扩展点设计，但不在本卡实现长期记忆本体
- 至少一轮基于 `opencode / deepseek-v4-flash-free` 的三维只读审核记录
- 根据采纳意见调优后的 proposal / design / spec / tasks

## 范围边界
- 本卡负责上下文构建架构、缓存边界与文档对齐
- 本卡不直接实现完整长期记忆产品能力
- 本卡不直接改造 Browser / Thread / Automation / Workflow 独立能力面
- 本卡可以定义 coding / work 双 profile 的 system prompt 结构，但不要求本轮完成所有 UX 切换入口
- 本卡优先收口“上下文如何进 prompt”，而不是先改具体工具实现

## 验收标准
- spec SHALL 明确稳定前缀、半稳定上下文与 turn-local 易变输入的正式分层
- spec SHALL 明确 `system prompt`、`runtime facts`、`AGENT.md/project instructions`、`memory injection`、`conversation carry` 的注入层级
- spec SHALL 明确 `AGENT.md` 与 workspace scope 的关系，以及目录级覆盖优先级
- spec SHALL 为跨 session 长期记忆预留扩展接口，但明确本轮非目标
- spec SHALL 明确哪些上下文变化会破坏缓存前缀，以及允许的低频 cache-reset 点
- 至少完成一轮使用 `opencode / deepseek-v4-flash-free` 的独立只读 spec 审核，且覆盖至少 3 个维度

## 当前进展
- 已完成当前项目与 `reasonix / codex / claude code` 的上下文构建路径对照分析
- 已确认本轮应协同设计，而不是拆成多个独立 change
- 已确认主 change 需要覆盖：
  - system prompt builder
  - `AGENT.md` / workspace instructions
  - cache-friendly conversation carry
  - memory extension points
- 已完成 `PA-056` 任务卡与 `redesign-context-assembly-and-cache-strategy` 的 proposal / design / spec / tasks 初稿
- 已使用 `opencode / deepseek-v4-flash-free` 完成一轮三维独立只读 spec 审核
- 已新增独立审核记录：
  [2026-06-16-pa056-spec-review.md](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-16-pa056-spec-review.md>)
- 已采纳本轮高优先级意见并完成文档调优：
  - 补 `Why This Grouping`
  - 补 `Migration Path`
  - 补 `AGENT.md` 文件级覆盖规则
  - 补 continuation 失败的显式回退策略
  - 补动态说明项的显式层分配
  - 补观测与实现桥接任务
- 已完成 `Implementation Bridge` 实现落地：
  - `LayeredTurnContext`
  - `context_refresh_reason / instruction_scope_sources / conversation_carry_mode`
  - normalized/native request observation 对齐
  - native memory layer 注入与长期记忆去重
- 已完成 `coding / work` 显式模式闭环：
  - 独立 `AppSettings` 持久化
  - Tauri `load_app_settings / save_app_settings`
  - 前端设置 store 与左侧边栏尾部设置入口
  - `SettingsPanel` 模式切换 UI
  - `workspaceMode -> TurnInput -> TurnContext -> domain profile` 透传主链
- 已完成 3 轮 `opencode / deepseek-v4-flash-free` 独立代码审核，并采纳高收益意见完成一轮代码调优：
  [2026-06-16-pa056-code-review.md](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-16-pa056-code-review.md>)
- 已完成 canonical spec 同步与 OpenSpec change 归档收口
- 已完成实现态验证：
  - `cargo check -p pony-agent-core --lib`
  - `cargo test -p pony-agent-core build_request_ -- --nocapture`
  - `npm exec vitest -- run tests/runtime-store.spec.ts tests/HomeSidebar.spec.ts`
- 已完成本轮显式模式闭环验证：
  - `npm run cargo:check -- -p pony-agent-core --lib`
  - `npx vitest run tests/settings.store.spec.ts`
  - `npx vitest run tests/App.spec.ts tests/HomeSessionSidebar.spec.ts tests/settings.store.spec.ts`
  - `npx vitest run tests/runtime-store.spec.ts -t "forwards workspace mode to start_graph_run_stream input payload"`

## 下一步动作
1. 后续如继续推进，应把全局设置从当前 `workspaceMode` 扩展为正式的可组合配置面，并补 `backend type / surface / layout` 等同层能力
2. 后续如继续推进，应把 `instruction_scope_sources` 从当前最小落点升级为真实的 workspace / `AGENT.md` source 枚举与 diff 检测
3. 后续如推进 provider continuation / compaction，应把 `conversation_carry_mode` 的未来变体真正接通
4. 后续如继续收紧缓存诊断，可把 `context_refresh_reason` 从“主因”提升为更强的 diff-based 机制

## 当前卡点
- 暂无

## 断点续跑提示
继续前先看：

- [PA-025](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-025-build-context-and-cache-friendly-prompt-boundary.md>)
- [context.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/context.rs)
- [app_settings.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/app_settings.rs)
- [SettingsPanel.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/SettingsPanel.vue)
- [settings.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/stores/settings.ts)
- [provider.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/provider.rs)
- [protocol_v1.md](/C:/Users/HUAWEI/Documents/pony-agent/codex-openai/codex-rs/docs/protocol_v1.md)
- [default.md](/C:/Users/HUAWEI/Documents/pony-agent/codex-openai/codex-rs/protocol/src/prompts/base_instructions/default.md)
- [SPEC.md](/C:/Users/HUAWEI/Documents/pony-agent/reasonix-esengine/docs/SPEC.md)
