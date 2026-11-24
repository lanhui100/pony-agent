# Agent Note: runtime store 等价测试 Wave 2b——会话目录写 + 工作区注册表

Status: implemented

## Problem

Wave 2a 之后仍有 26 条 action 没有等价测试（B277）。其中 D1「会话目录写操作」与
D2「工作区注册表」这两组属于**用户每天都会触发、但断言最难写**的部分：它们都要先过一串
本地前置校验（空名 / 超 64 / 会话不存在 / inflight / 提交期 / 运行中），再落到一条宿主命令，
最后还要刷新列表。这类代码最容易在重构时"校验顺序被改掉"而无人发现——
症状是错误文案变了、或本该在本地拒绝的请求打到了宿主。

## Decision

新增 `src/stores/runtime/__tests__/equivalence-wave2b.spec.ts`（16 用例 / 7 条 action），
沿用 Wave 2a 的 harness（假定时器 + 假 IPC），断言口径固定为「宿主命令边界 + 状态转移」：

- **D2 工作区（4 条）**：`loadWorkspaces` / `createNewWorkspace` / `renameWorkspace` / `deleteWorkspace`
  - `loadWorkspaces`：成功落值 + `workspaceListLoaded=true`；非数组返回 ⇒ 空列表仍标记 loaded（防御通道）；
    宿主失败 ⇒ 空列表且 **loaded 保持 false**（允许重试，这是既有的可重试语义，不是遗漏）。
  - `createNewWorkspace`：去重后追加；空名/空路径本地拒绝；宿主报「重复 root」⇒ 重拉注册表命中既有记录。
  - `renameWorkspace`：替换同 id 记录；空白名与超 64 字符都在本地拒绝且不触达宿主。
  - `deleteWorkspace`：默认工作区拒绝；成功后移除记录并把 `sessionWorkspaceId` 归一为 `default`。
- **D1 会话目录（3 条）**：`renameSession` / `archiveSession` / `unarchiveSession`
  - `renameSession`：成功时透传 `{sessionId, title}`（标题已 trim）；空白/超 64/会话不存在三种本地拒绝。
  - `archiveSession`：当前会话提交期拒绝（"正在提交"）、后台运行中拒绝（"运行中"）、成功调 `session_archive` 并刷新。
  - `unarchiveSession`：成功调 `session_unarchive` 并刷新。

覆盖对账同步：`COVERED_ACTIONS` 71 → **78**，`UNCOVERED_ACTIONS` 26 → **19**，
「已覆盖 ∪ 未覆盖 == 97」与「两侧不相交」两条机械断言保持不变。

## Verification

- `npx vitest run src/stores/runtime/__tests__/equivalence-wave2b.spec.ts concurrency-lifecycle.spec.ts` → 20 passed
- `npm test` → 全量通过（见 PR 描述）

## Alternatives considered

- **把断言写成「调用后列表快照全量比对」**：会与既有 golden 场景重复（golden 已经冻结全量快照），
  且快照一改就大面积红；本波坚持只冻结"命令边界 + 关键状态位"。
- **跳过"本地拒绝"分支，只测成功路径**：这恰恰是重构最易破坏的部分——把校验顺序调换、
  或把本地拒绝改成打到宿主再报错，成功路径仍然绿。
- **把 `activateWorkspace` / `normalizeActiveWorkspace` 也塞进本波**：它们依赖 localStorage 与
  注册表投影归一，属于"激活态"主题，与本波"写操作"判据不同，留待 Wave 2c 单独处理。
- **直接给 `deleteSession` / `createSession` / `forkHistoryNode` 补测试**：这几个需要宿主删除/分叉
  语义与 UUID 注入，夹具成本显著高于本波两条主线，放在下一波更合适（避免为赶覆盖数写出脆弱断言）。
