# PA-061 宿主权威 session view 硬切与旧读面安全清理

## 状态

- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## 依赖

- 前置：`PA-060`

## Canonical Spec

- `openspec/specs/session-cursor-view-contract/spec.md`
- `openspec/specs/history-node-management/spec.md`

## 背景

项目当前尚未发布，因此可以不保留新旧合同兼容层，但需要安全清理旧逻辑，避免把旧 fallback、旧字段镜像、旧恢复语义继续留在代码里形成第二套状态机。

`PA-060` 已定义新的母合同并完成第一版接线；`PA-061` 的任务是把正式宿主链路硬切到 host-authoritative `cursor / view` 读模型，并清理不再需要的旧读面依赖。

## 目标

1. 正式宿主链路只信 host-authoritative `SessionRuntimeView`
2. 前端/TUI/CLI/HTTP 消费面不再依赖旧的“客户端推断恢复语义”
3. 对仍存在的旧字段镜像、旧读面兼容代码进行安全清理，而不是继续双写

## In Scope

- `load_session_runtime_view` / `load_retrieved_context` 的正式消费链路收口
- 前端 store 对 host-projected read-model 字段的主路径消费
- 清理不再需要的旧 `historyCursor` 镜像依赖、重复投影字段或客户端推断分支
- 为未发布项目执行破坏性切换后的测试与验证收口

## Out of Scope

- browser preview / local preview 模式的最终退场策略
- `cursorVersion` 真正的并发实现

## 验收标准

- 正式宿主链路 SHALL 只依赖 host-authoritative `SessionRuntimeView` 恢复当前视图
- 客户端 SHALL NOT 继续通过旧 fallback 逻辑推断正式宿主 session 的历史位置
- 若旧字段仍临时保留，代码中 SHALL 明确其为兼容/过渡字段并有删除计划
- 针对本卡新增或更新的定向测试 SHALL 覆盖“无旧镜像字段时仍可正确落态”的路径

## 下一步动作

已完成；后续如继续推进，转入：

1. `PA-062` browser preview fallback 退场与安全降级收口
2. `PA-063` cursor versioning 与多端冲突保护

## 断点续跑提示

- `src/stores/runtime.ts`
- `src/types/runtime.ts`
- `crates/pony-agent-core/src/agent/control_plane.rs`
- `openspec/specs/session-cursor-view-contract/spec.md`

## 当前进展

- 已完成正式宿主链路硬切：host-backed `loadSessionState()` 不再从 persisted local state 猜测 `nodeId`
- 已删除旧的 `previousHistoryState` host 补偿分支，避免正式宿主继续沿用客户端历史态猜测
- 已让前端在 host-authoritative runtime view 下优先消费 host 投影字段，而不是依赖旧镜像兜底
- 已补充“host-authoritative runtime view 明确清空 stale local historical state”的定向测试

## 验收结果

- 通过定向前端测试：
  - `passes nodeId through runtime and retrieved context requests and hydrates history cursor state`
  - `hydrates host-projected history state even when legacy historyCursor mirror is omitted`
  - `clears stale local historical state when host-authoritative runtime view omits history state`
  - `preserves host authority metadata on runtime views`
  - `preserves historical checkout when switching away and back in browser fallback mode`

## 残余风险

- 本卡不处理 browser preview 的最终退场策略，相关逻辑转交 `PA-062`
- 本卡不处理真实 cursor revision/version，相关并发保护转交 `PA-063`
