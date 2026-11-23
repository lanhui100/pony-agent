# Agent Note: runtime store 等价测试 Wave 2d——轮次撤回 / 初始态回滚 / 检索上下文

Status: implemented

## Problem

Wave 2c 之后剩 14 条未覆盖。本波挑「撤回与回滚」这一组，因为它们**出错就丢用户数据**，而此前零测试：

- `rollbackToTurnBoundary(turnId)`：按 turnId 截断消息、过滤 trace、重置活跃轮字段。判据错一位就
  要么多留一轮（撤回没生效）、要么多砍一轮（用户消息消失）。
- `rollbackToInitialState()`：清空轮次级全部真值（provider/token/draft/error/liveMetrics）。
  漏清一个字段 ⇒ 下一轮界面显示上一轮的 provider 与 token。
- `loadRetrievedContextState()`：宿主读取失败必须**回退快照派生**，而不是把空值当成功。

## Decision

新增 `src/stores/runtime/__tests__/equivalence-wave2d.spec.ts`（6 用例 / 3 条 action）：

- `rollbackToTurnBoundary`：空 turnId 与未命中都返回 `false` 且不动消息；命中 ⇒ 截断到该轮末尾、
  `turnTraceHistory` 只保留被保留轮次、`phase` 按剩余消息回 `ready`、`activeTurnId`/`isSubmitting` 归零。
- `rollbackToInitialState`：断言 11 个字段归零（phase/error/draft/summary/provider 五连/token 三连/ttft/liveMetrics）。
- `loadRetrievedContextState`：宿主返回 ⇒ 落值且不是同一对象引用；宿主失败 ⇒ 回退派生、不抛错、仍计一次宿主调用。

覆盖对账同步：`COVERED_ACTIONS` 83 → **86**，`UNCOVERED_ACTIONS` 14 → **11**。

## 刻意留在 UNCOVERED 的一条

`findCheckpointTurnIdByNodeId` 本轮**没有**补测试并已放回未覆盖清单（附原因）：它是 getter
`conversationCheckpointEntries` 的派生查询，而该 getter 由 `historyNodes` / `historyBranches` /
`activeBranchId` / `visibleNodeId` / `branchHeadNodeId` 五路派生。裸 `seedStore` 造不出合法分支图
（实测返回 `null`），硬凑夹具只会写出「假绿」用例。它属于「历史分支」主题，与 Wave 2e 的
`forkHistoryNode` / `restoreBranchHead` 一起做更合适。

## Verification

- `npx vitest run equivalence-wave2d.spec.ts concurrency-lifecycle.spec.ts` → 9 passed
- `npm test` → 全量通过（见 PR 描述）

## Alternatives considered

- **用 `seedStore({ conversationCheckpointEntries: [...] })` 直接喂 getter 数据**：它是个 getter 不是 state 键，
  seed 无效（实测 `findCheckpointTurnIdByNodeId` 返回 null）——这是"看起来写完了其实没测到"的典型陷阱。
- **只断言 `result` 非空就当作检索成功**：宿主失败路径也会返回非空（回退派生），断言无法区分成功与降级；
  因此改为「宿主返回值确实落在 `turnContext` 上」+「失败路径仍发生一次宿主调用」。
- **把 `rollbackToInitialState` 断成 `toEqual(initialState)` 全快照**：与既有 golden 重复且噪声大；
  本轮只锁「上一轮真值必须归零」这 11 个字段。
