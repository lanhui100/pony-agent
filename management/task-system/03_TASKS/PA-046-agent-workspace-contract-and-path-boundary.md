# PA-046 定义 agent workspace 合同与路径边界

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change
- 已归档：
  [2026-06-15-agent-workspace-contract-and-path-boundary](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-15-agent-workspace-contract-and-path-boundary>)

## Delta Spec
- 已同步并归档：
  `openspec/changes/archive/2026-06-15-agent-workspace-contract-and-path-boundary/specs/agent-workspace-contract/spec.md`

## Canonical Spec
- 已同步到：
  `openspec/specs/agent-workspace-contract/spec.md`

## Spec 状态
- Proposal: `validated`
- Spec: `validated`
- Design: `validated`
- Tasks: `validated`

## 背景
当前 `AgentRuntimeBuilder.workspace_root(...)` 与 `ToolRouter::with_workspace_root(...)` 已经提供了工作区注入路径，`tools.rs` 也已经具备 canonicalize、越界防护、相对路径展示与路径修复逻辑。但这些能力还没有被写成正式 workspace 合同。

## 目标
把 workspace 作为工具系统的正式上下文边界收口，明确路径解析、安全边界、展示规则与 future host policy seam。

## 输出
- `agent-workspace-contract-and-path-boundary` OpenSpec change
- `WorkspaceContext` 或等价结构的正式合同
- 路径 canonicalize / relative display / out-of-root 拒绝规则
- workspace read/write/search/execute scope 语义
- host preset 与 runtime 注入的工作区边界说明

## 范围边界
- 本卡不直接定义权限审批 UI
- 本卡不要求实现多工作区产品体验
- 本卡不重写现有工具层逻辑，只要求把已有能力升级为正式合同

## 验收标准
- 系统 SHALL 显式定义 agent run 的 workspace 边界
- 文件与目录类工具 SHALL 默认只在 workspace 边界内工作
- 相对路径、绝对路径、路径修复与展示语义 SHALL 有统一合同
- workspace SHALL 作为 runtime / tool / trace / permission scope 的共同上下文
- 至少完成一轮独立 spec review，并根据采纳意见优化文档

## 当前进展
- 已完成实现、测试、实现态审核与一轮采纳调优
- 已完成 OpenSpec strict validate，并已将 delta spec 同步到 canonical spec
- 已完成 change 归档与任务系统收口

## 下一步动作
已完成，无后续动作；后续若扩展多 workspace 或 host workspace policy，应以新 change 承接。

## 当前卡点
- 暂无。当前已完成归档与收口。

## 断点续跑提示
继续前先看：

- [tools.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/tools.rs)
- [runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/runtime.rs)
