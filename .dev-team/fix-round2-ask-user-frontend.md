# PA-114 前端修复第二轮（Reviewer-1 致命发现：F1 接线断链）

## 裁决背景
前端对抗审查 FAIL：`mergeToolCalls`/`agentTurnEvents`（HomeWorkspace.vue）产出的
`MergedToolCall` 恒无 `callId`/`runId`/`arguments`，WorkspaceTurnItem 原样传入后卡片
`matchingPending` 退化为通配匹配恒命中 `pendingAsks[0]`：
- 同 run 两条不同 callId 的 ask 行绑定同一 ask；
- 历史已完成 ask 行在当前 ask 挂起时渲染交互控件并显示当前 prompt；
- `ask-user-waiting` 仅 pendingAsks 为空才可达（生命周期语义错）。

## 事实（已核实）
- 后端 `TurnToolActivity.id == call_id`（projection.rs 同 call_id 去重 push），
  `arguments_text` 携带参数 JSON 字符串；前端 ToolActivity（types/runtime.ts L108）已有
  `id`/`argumentsText`。
- `ChatMessage`（tool 角色）目前无 `callId`/`argumentsText`；`syncToolMessages`
  （runtime.ts L2722+）从 ToolActivity 构造 message 时丢弃 id/arguments。
- `mergeToolCalls`（HomeWorkspace.vue L358+）把连续同 mergeKey 的 ask 行合并成 count>1 单行。

## 修复指令（Executor，业务代码）
1. `src/types/runtime.ts` `ChatMessage`：增 `callId?: string | null`、`argumentsText?: string | null`。
2. `src/stores/runtime.ts` `syncToolMessages`：新增与既有 message 同步两处均透传
   `callId: tool.id ?? null`、`argumentsText: tool.argumentsText ?? null`。
3. `src/components/HomeWorkspace.vue` `mergeToolCalls`：
   - MergedToolCall 增 `callId`/`argumentsText` 透传；
   - **ask 行不合并**：`mergeKey` 合并条件追加 `!isAskToolName(tool.toolName)`（每个 ask 调用独立成行，
     避免两条不同问题合并丢失 callId）。
4. `src/components/chat/WorkspaceTurnItem.vue` `MergedToolCall` 类型（L40-64）：增可选
   `callId`/`argumentsText`。
5. `src/components/ask/AskUserToolCallCard.vue`：
   - `matchingPending`：**callId 缺失时不得通配**（两值都非空才匹配：`ask.callId === tool.callId`，
     且 `tool.runId` 非空时要求 `ask.runId === tool.runId`）；无匹配返回 null。
   - `questionText`：pending prompt → 解析 `tool.argumentsText`（JSON，question→text→prompt）→ description。
   - 终态：无匹配时按 `tool.status` 分支——`"done"` 显示"已回答/已完成"静态态、`"error"` 显示失败态、
     其余（undefined/pending/running）显示 `ask-user-waiting` 待命态（保留 spinner）。
   - 脏值：watch 匹配 pending 的 requestId，变化时清空 `typedAnswer`。
6. 保持红相 spec 10/10 绿相不动摇（调用点：F1-2/F1-3 直传 callId/runId 的用例必须仍绑定；
   F1-3 waiting 用例 callId 不匹配必须仍 waiting）。

## 测试补充（Test Agent）
在 `tests/acceptance/stage-1-ask-user.spec.ts` **追加**（不修改既有断言）：
- T1：多 pending ask（不同 callId）时，卡片按 callId 绑定正确的那个（reviewer 探针场景）。
- T2：tool 无 callId → 不绑定任何 pending ask（waiting/静态，绝不命中 pendingAsks[0]）。
- T3：tool.status "done" 且无匹配 → 渲染静态完成态（无 spinner/无交互控件）。
- T4：绑定 ask 变化（同组件 rerender 换 callId）时 typedAnswer 被清空。
- T5（可选）：HomeWorkspace `mergeToolCalls` 对 ask 行不合并、其余行照常合并（可在 HomeWorkspace.spec.ts 或直接组件测试）。

## 门禁
`npx vitest run tests/acceptance/stage-1-ask-user.spec.ts` 全绿（含新增）；`npm run build` 通过；
相关既有前端测试（HomeWorkspace/runtime-store/WorkspaceTurnItem 相关）回归全绿。
