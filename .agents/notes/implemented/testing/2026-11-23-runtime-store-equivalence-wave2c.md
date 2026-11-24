# Agent Note: runtime store 等价测试 Wave 2c——激活态 / inflight 守卫 / 节流归属 / 续跑入口

Status: implemented

## Problem

Wave 2b 之后还剩 19 条未覆盖 action。其中这一组有个共同特征：**失败是无声的**。

- `activateWorkspace` / `normalizeActiveWorkspace` 维护「新会话创建目标」的 localStorage 单一真相源：
  激活项失效时不回退 ⇒ 后续轮次带着孤儿工作区 id 盖章进会话（B267 那次事故的形态）。
- `isSidebarOpInflight` 是重命名/归档等操作的并发闸门：键语义一旦从「操作+id」漂移成别的形状，
  两个并发点击会同时打到宿主。
- `scheduleThrottledTraceTimeline` 带**归属守卫**（B279 的跨会话串味修复）：定时器存活期间切会话时，
  旧归属的 pending timeline 必须丢弃；这条守卫没有任何测试，改坏了只会表现为"切会话后思考链闪一下旧内容"。
- `retryLastTurn` 的两个拒绝分支（提交中 / 无可重试失败轮）如果反过来，用户会看到"点续跑没反应"。

## Decision

新增 `src/stores/runtime/__tests__/equivalence-wave2c.spec.ts`（8 用例 / 5 条 action）：

- `activateWorkspace`：设置 `activeWorkspaceId` 并写入 `ACTIVE_WORKSPACE_STORAGE_KEY`（用真实常量，不硬编码字符串）。
- `normalizeActiveWorkspace`：激活项仍在注册表 ⇒ 不动；已失效 ⇒ 回退 `default` **并清掉存储键**。
- `isSidebarOpInflight`：判据即「键存在性」，同前缀不同 id 不串。
- `scheduleThrottledTraceTimeline`：
  - 首次调度落 `pending` 并起定时器；重复调度**不新建**定时器（复用同一 id）；
  - 定时器到期但 `sessionId` 已变 ⇒ **丢弃投递**，`pending` 保持、计时器句柄归零（B279 判据）。
  - 时间推进一律走 harness 的 `advance()`（假定时器），不引入真实 sleep。
- `retryLastTurn`：提交中 ⇒ `false` 且不写草稿；无可重试失败轮 ⇒ `false`。

覆盖对账同步：`COVERED_ACTIONS` 78 → **83**，`UNCOVERED_ACTIONS` 19 → **14**，
「∪ == 97」与「两侧不相交」断言保持。

## Verification

- `npx vitest run equivalence-wave2c.spec.ts concurrency-lifecycle.spec.ts` → 12 passed
- `npm test` → 全量通过（见 PR 描述）

## Alternatives considered

- **用真实 `setTimeout` + `await sleep(260)` 验证节流**：harness 已 `vi.useFakeTimers()`，
  真实等待会让用例挂在 5s 超时（本轮实测就是这样红过一次），且违背「禁止真实 sleep」的既有约定。
- **硬编码 `"pony-active-workspace-id"` 存储键**：真实键是 `pony-agent.active-workspace.v1`，
  硬编码等于给未来重命名埋一颗哑弹；改前实测同样红过一次。
- **`retryLastTurn` 成功分支一起测**：成功分支会走 `submitTurn()` 全链路（宿主 IPC + 流式事件），
  属于既有 golden 场景的判据，本波只锁「两个拒绝分支」这个最容易反转的边界。
- **把 `deleteSession` / `createSession` / `forkHistoryNode` 一并塞进来**：需要 UUID 注入与宿主分叉语义，
  夹具成本高于本波主题；留在 Wave 2d 单独处理，避免为凑覆盖数写脆弱断言。
