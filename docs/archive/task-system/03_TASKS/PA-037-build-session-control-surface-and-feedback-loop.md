# PA-037 构建 session 控制交互面与反馈闭环

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change
- [add-session-control-surface-and-feedback-loop](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-session-control-surface-and-feedback-loop)

## Delta Spec
- [session-control-surface-and-feedback-loop/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-session-control-surface-and-feedback-loop/specs/session-control-surface-and-feedback-loop/spec.md)

## Canonical Spec
- 交付后沉淀到 `openspec/specs/` 的 session control surface canonical spec

## Spec 状态
- Proposal: `ready`
- Spec: `ready`
- Design: `ready`
- Tasks: `ready`

## 目标
把已经存在于 runtime store / control-plane / history graph 中的 stop、resume、continue、checkout、restore、fork、switch 等 session 控制能力，收口成用户可见、可理解、可验收的前端交互闭环，而不是只保留“底层能力已存在”的工程完成态。

## 输出
- session 控制入口的显式前端交互
- 历史态 / paused / recovery-capable / degraded restore 的统一状态语言
- checkout / restore / stop / resume 的结果反馈与 disabled reason
- 对 session 控制交互的前端验收测试
- 与 `PA-028 / PA-032 / PA-034 / PA-035` 的范围隔离说明

## 范围边界
- 本卡只消费既有 runtime store / control-plane / history graph 合同，不新增 stop / resume / replay 后端命令
- `replay` 仅作为用户文案或入口语义，最终只能派发到既有 `submitTurn()` / `submissionPlan` / runtime store 边界
- `paused`、`historical`、`historical_dirty`、`recovery-capable` 必须绑定到现有真相源，不新增前端私有状态机

## 验收标准
- 用户在 paused run、存在 recovery checkpoint，或后端 submission plan 明确指向 non-default resume / continue path 时，界面上能看到明确的 `继续 / 恢复 / 重新开始` 入口
- 用户在运行中能看到明确的 `停止` 入口，并在 stop 后得到可验证反馈
- 历史 checkout 若降级为 transcript-only，界面必须显式展示“未恢复工作区”及原因
- restore / fork / branch switch 后，界面必须显式反馈当前 branch / visible node / mode 的变化
- `historical_dirty`、disabled actions 与 degrade reason 必须有用户可读文案，不能只暴露原始 mode/flag；disabled reason 至少覆盖运行中不可切换、空白 transient session、无 resumable run / checkpoint、workspace rollback 不支持四类
- 前端测试覆盖 stop CTA、resume/replay CTA、history degrade feedback 与 mode 切换四类路径

## 当前进展
- 已完成能力底座：
  - [PA-028](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-028-build-history-node-management-and-branching.md)
  - [PA-032](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-032-stabilize-trace-persistence-and-recovery-contract.md)
  - [PA-034](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-034-implement-checkpoint-lifecycle-boundary.md)
- 立项时缺口：
  - `HomeSessionSidebar` 已有 checkout / restore / fork / switch 入口，但 stop / resume / continue / replay 仍主要停留在 store 内隐式仲裁
  - `checkout/restore` 的 degrade result 还没有形成用户可见反馈
  - `historical_dirty`、disabled action reason 与“当前控制动作影响哪一层”尚未被 UX 明确表达
  - `paused`、`recovery-capable`、`replay` 等文案还没有和 `submissionPlan / checkpoint / historyCursorMode / activeRunId` 的真相源逐一绑定

- 当前已完成实现：
  - `HomeWorkspace.vue` 已把 `stop / resume / continue / replay` 收口成显式控制面：
    - 运行中显示显式 `停止` CTA，并在 stop 请求发出后展示可验证反馈
    - `submissionPlan / checkpoint` 指向 `resume / continue / replay` 时，发送按钮会切换为 `恢复 / 继续 / 重新开始`
    - 控制文案直接绑定 `latestGraphRunSubmissionPlan / latestExecutionCheckpoint / phase`
  - `HomeSessionSidebar.vue` 已补 session control 状态卡：
    - `historical / historical_dirty / paused / recovery-capable` 已映射为统一用户文案
    - 不再直接向用户暴露原始 `historyCursorMode` 字符串
    - disabled actions 已补用户可读 reason
  - `HomeSessionSidebar.vue` 已补 history result feedback：
    - checkout transcript-only degrade 会明确显示“仅恢复对话，未恢复工作区”及原因
    - restore / fork / branch switch 成功后会明确反馈 `branch / visible node / mode` 变化
  - 前端验收测试已补齐：
    - `tests/HomeWorkspace.spec.ts` 已覆盖 stop / resume / replay CTA 展示与派发
    - `tests/HomeSessionSidebar.spec.ts` 已覆盖状态语言、disabled reason、degrade feedback 与 restore/fork/switch 成功反馈
    - `tests/runtime-store.spec.ts` 已继续验证底层 `stop / resume / continue / replay` 派发合同未回归

## 下一步动作
- 如后续需要扩展真正的“无输入 resume / replay”专用命令，应单独立卡，不回灌 `PA-037`
- 持续以本卡形成的 UX 语言约束后续 session / history 相关前端改动

## 当前卡点
- 暂无；本卡已完成关闭

## 断点续跑提示
继续前先看：
- `src/components/HomeSessionSidebar.vue`
- `src/components/HomeWorkspace.vue`
- `src/stores/runtime.ts`
- `src/types/runtime.ts`
- `tests/HomeSessionSidebar.spec.ts`
- `tests/HomeWorkspace.spec.ts`
