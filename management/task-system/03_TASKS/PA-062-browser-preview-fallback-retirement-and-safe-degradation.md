# PA-062 browser preview fallback 退场与安全降级收口

## 状态

- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## 依赖

- 前置：`PA-060`
- 建议串行于：`PA-061`

## Canonical Spec

- `openspec/specs/session-cursor-view-contract/spec.md`

## 背景

当前 browser preview / local preview 仍承担一部分非正式恢复职责。对于未发布项目，我们不做兼容保留，但也不能粗暴删掉而让浏览器预览彻底不可用。

需要单独一张卡来决定：哪些 fallback 必须保留为受控降级，哪些旧逻辑应该删除，哪些 UI/contract 必须显式标注 `local_preview`，从而避免 preview 模式继续伪装成正式宿主语义。

## 目标

1. 明确 browser preview 的能力边界
2. 清理不再合理的 fallback 恢复语义
3. 保留最小可用的 preview 体验，但不再让其承担正式历史恢复真相源职责

## In Scope

- preview 模式 authority 标记、文案、动作禁用/降级
- local cache 在 preview 模式下的最小保留策略
- 清理误导性的“看起来像正式宿主恢复”的旧逻辑

## Out of Scope

- 正式宿主链路的硬切收口
- 多端并发控制

## 验收标准

- preview 模式 SHALL 明确声明 `local_preview` / degraded authority
- 客户端 SHALL NOT 在 preview 模式下暗示与正式宿主等价的恢复保证
- 不再合理的旧 fallback 路径 SHALL 被删除或显式降级
- 相关 UI / store / tests SHALL 对 preview 与 host 模式作出清晰区分

## 下一步动作

已完成；后续如继续推进，转入：

1. `PA-063` cursor versioning 与多端冲突保护

## 断点续跑提示

- `src/stores/runtime.ts`
- `tests/runtime-store.spec.ts`
- `tests/e2e/browser-preview.spec.ts`
- `openspec/specs/session-cursor-view-contract/spec.md`

## 当前进展

- 已完成 browser preview / local preview 的动作收口：不再支持 `restore branch head / fork / switch branch` 这类正式宿主历史控制动作
- 已保留 preview 的最小可用能力：会话创建、切换、提交、取消恢复、历史 transcript 回看
- 已将 preview 的 branch/history 控制从“看起来像正式宿主能力”降级为显式不可用，并通过 `sessionError` 暴露原因

## 验收结果

- 通过定向前端测试：
  - `creates a transient browser-preview session while keeping the previous session persisted`
  - `returns from a transient browser-preview session to the saved session when deleted`
  - `initializes browser-preview mode from the latest persisted session`
  - `completes submitTurn in browser-preview mode and records the turn`
  - `restores a cancelled browser-preview turn from persisted canonical terminal evidence`
  - `disables restore, fork, and branch switching in browser-preview degraded mode`
  - `preserves historical checkout when switching away and back in browser fallback mode`

## 残余风险

- 本卡不处理 preview UI 上更细粒度的禁用提示与呈现差异，如需进一步收口可另开 UI 卡
- 本卡不处理多端并发与 cursor revision，相关议题转交 `PA-063`
