# PA-060 统一 checkpoint / cursor / view 合同以支持多端宿主

## 状态

- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change

- 已归档：`openspec/changes/archive/2026-06-22-unify-session-cursor-view-contract/`

## Canonical Spec

- `openspec/specs/session-cursor-view-contract/spec.md`

## Spec 状态

- Proposal: `archived`
- Spec: `archived`
- Design: `archived`
- Tasks: `archived`

## 背景

当前系统已经具备 `history node / branch / checkout / restore / fork / switch branch` 最小闭环，也已经在 Tauri 前端接入 `load_session_runtime_view` 与 `load_retrieved_context` 作为正式读面。

但本轮 checkpoint 问题暴露出仍存在以下结构性风险：

- `checkpoint graph`、`history cursor`、`runtime view` 的职责边界还不够收紧
- 前端 local cache 与后端权威 cursor 在部分路径上仍共同参与恢复语义
- 不同消费面可能继续复制恢复规则，而不是统一消费宿主投影
- 未来扩展到 `TUI / CLI / HTTP` 时，若继续沿用多条恢复链，会放大状态漂移和兼容成本

## 目标

把 Pony Agent 的历史恢复体系正式收口为：

1. `HistoryGraph` 负责保存可恢复历史节点与分支关系
2. `Cursor` 负责表达“当前正在看哪里”
3. `View` 负责表达“当前应该渲染什么”
4. 任意客户端只通过统一 command/read-model 与宿主交互，而不是本地补偿恢复语义

## 输出

- 一份面向 `Tauri / TUI / CLI / HTTP` 共享的 `session-cursor-view-contract` canonical spec
- 对现有 `history-node-management`、`retrieval boundary`、`session runtime view` 的边界澄清
- 明确 local cache、browser preview fallback 与宿主权威 cursor 的职责收口规则
- 一份从现状演进到统一架构的迁移分阶段方案

## 范围边界

### In Scope

- `checkpoint graph`、`cursor`、`view` 的职责切分
- 统一 command/read-model 的宿主交互边界
- 多端消费面的恢复语义统一
- local cache 的语义降级规则
- browser preview 与正式宿主链路的合同差异

### Out of Scope

- 本卡不直接完成全部 Rust / TS 实现迁移
- 本卡不直接设计新的 UI 交互样式
- 本卡不一次性重写所有已有 history / checkpoint spec，只要求定义新母合同和兼容边界

## 验收标准

- 系统 SHALL 明确把 `checkpoint` 视为历史节点图，而不是“当前显示位置”的真相源
- 系统 SHALL 明确把 `cursor` 视为单一权威状态，用于表达当前可见节点、当前分支与模式，而 `branch head` 真相源由 `HistoryGraph` 承担
- 系统 SHALL 通过统一 `view` 读模型返回客户端渲染所需数据，而不是要求客户端本地重建
- 任意客户端 SHALL 通过统一 command 修改 `cursor` 或 `graph`，而不是本地直接改消息视图
- local cache SHALL 被定义为性能与 UX 辅助层，而不是历史恢复真相源
- 正式宿主链路 SHALL 支持在未显式指定 `nodeId` 时按权威 cursor 恢复当前视图
- browser preview / 非正式宿主 SHALL 明确其恢复能力边界，不得伪装成与正式宿主等价
- Canonical spec SHALL 显式说明未来 `TUI / CLI / HTTP` 不应复制前端 fallback 逻辑
- Canonical spec SHALL 明确定义旧调用面到新合同的兼容桥接与废弃顺序
- Canonical spec SHALL 明确定义前端 fallback 的退场条件、观测信号与删除门槛
- Canonical spec SHALL 明确定义多 surface 并发冲突与 cursor 版本化要求
- 至少完成一轮 3 路并行 spec 审核，并根据采纳意见完成一版调优

## 当前进展

- 已完成问题诊断：现有实现存在前端 cache / 宿主 cursor 双恢复来源的结构性风险
- 已完成一次针对当前 bug 的最小修复，用于阻止空 runtime view 覆盖历史状态
- 已完成 canonical spec、任务系统接入、两轮并行审核采纳与第一版实现接线
- 已完成前端 runtime/store 对 authority/read-model 字段的最小消费闭环
- 已完成定向前端测试验证；Rust 全量/定向验证受仓库现存无关编译问题阻断，已留痕

## 下一步动作

已完成；后续如继续推进，应新拆：

1. `PA-061` 宿主权威 session view 硬切与旧读面安全清理
2. `PA-062` browser preview fallback 退场与安全降级收口
3. `PA-063` cursor versioning 与多端冲突保护

## 当前卡点

- 已无本卡 blocker；当前残余风险已转化为后续实现卡议题：
  - 真正的 `cursorVersion` 尚未落地，只保留了合同字段并显式避免伪造版本号
  - Rust 更广泛验证被仓库现存无关编译错误阻断，需要独立修复后再做全量验收

## 审核记录

- 已完成 3 路并行只读审核；当前已采纳的核心意见包括：
  - `branch head` 权威应归属于 `HistoryGraph`，而不是由 cursor 成为第二真相源
  - 新 spec 与 `history-node-management` 需要明确分层与所有权边界
  - 需要补 non-host / browser preview 的能力声明与 degraded authority 要求
  - 需要补前端 fallback 退场、旧接口兼容桥接与多 surface 并发版本化约束
- 已完成 2 路代码审核；已采纳的核心意见包括：
  - 不伪造 `cursorVersion=0`，避免制造假的并发安全语义
  - 前端 store 需真正消费顶层 host-projected read-model 字段，不能只依赖旧 `historyCursor` 镜像
  - `PersistedRuntimeState` 的 fallback 字段需补齐类型约束，避免继续无声漂移

## 验证与结果

- `npm run verify` 全绿通过
- 前端定向测试通过：
  - `passes nodeId through runtime and retrieved context requests and hydrates history cursor state`
  - `preserves host authority metadata on runtime views`
  - `hydrates host-projected history state even when legacy historyCursor mirror is omitted`
  - `preserves historical checkout when switching away and back in browser fallback mode`
- Rust 回归集通过：
  - `session_regression` 5 passed
  - `tool_router_regression` 12 passed
  - `provider_registry_regression` 7 passed
- 外部编译阻塞已修复：
  - `src-tauri/tests/provider_registry_regression.rs` — 已补齐新增字段初始化
  - `crates/pony-agent-core/src/bin/non_tauri_harness.rs` — 已修正导入路径

## 断点续跑提示

继续前先看：

- `management/task-system/01_TASK_BOARD.md`
- `openspec/specs/history-node-management/spec.md`
- `src/stores/runtime.ts`
- `crates/pony-agent-core/src/agent/control_plane.rs`
- `crates/pony-agent-core/src/agent/session.rs`
