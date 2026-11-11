# PA-060 Spec Review

## 审核对象

- [PA-060 任务卡](../03_TASKS/PA-060-unify-checkpoint-cursor-view-contract-for-multi-surface-hosts.md)
- [Session Cursor View Contract](../../openspec/specs/session-cursor-view-contract/spec.md)
- [History Node Management](../../openspec/specs/history-node-management/spec.md)

## 审核方式

- 模型：并行 3 路只读审核
- 方式：架构边界 / 多端协议 / 迁移落地 三个角度独立审阅，不改文件

## 审核结论

- 初稿方向正确，但**未达到可直接拆实现卡**的程度。
- 主要 blocker：
  1. `branch head` 权威归属未钉死，存在 graph / cursor 双真相源风险
  2. 新 spec 与 `history-node-management` 的分层和所有权不够明确
  3. 多端并发、旧接口兼容、前端 fallback 退场约束不足
  4. browser preview / non-host authority 的降级声明不够硬

## 已采纳问题

### 高优先级

| # | 问题 | 修复 |
|---|---|---|
| 1 | `branch head` 仍可能成为 cursor 的第二真相源 | 在 canonical spec 中明确 `HistoryGraph` 是 branch topology / branch head 唯一权威 |
| 2 | 新旧 spec 分层关系不足 | 补充 `history-node-management` 与 `session-cursor-view-contract` 的 ownership 分层 |
| 3 | non-host / browser preview 仍留灰区 | 补充 degraded authority 明示要求与动作降级要求 |
| 4 | 前端 fallback 退场没有迁移约束 | 新增 phased retirement requirement |
| 5 | 多 surface 并发冲突未定义 | 新增 cursor versioning / stale mutation conflict requirement |

### 中优先级

| # | 问题 | 修复 |
|---|---|---|
| 6 | read model 最小字段合同不够硬 | 要求 host 返回足够字段避免客户端自行推断历史模式 |
| 7 | command taxonomy 不清楚 | 明确 cursor-only 与 graph-mutating commands 的边界 |
| 8 | 旧调用面兼容桥接未定义 | 新增 legacy API compatibility bridge requirement |
| 9 | 任务卡仍停留在“再决定是否拆卡” | 调整为必须拆桥接实现卡、fallback 退场卡、并发保护卡 |

## 调优结果

已应用至：

1. `openspec/specs/session-cursor-view-contract/spec.md`
   - 补 `branch head` 权威归属
   - 补 command taxonomy
   - 补 multi-surface cursor versioning
   - 补 degraded authority / preview 边界
   - 补 fallback retirement
   - 补 legacy API compatibility bridge
   - 补与 `history-node-management` 的 ownership 分层
2. `management/task-system/03_TASKS/PA-060-unify-checkpoint-cursor-view-contract-for-multi-surface-hosts.md`
   - 扩充验收标准
   - 调整下一步动作
   - 回写审核摘要与当前卡点

## 未采纳项

- 第 2 路多端协议审核任务未返回有效文本，因此本轮只采纳了已成功返回的两路审核共识。

## 结果

- 当前 spec 已从“方向正确的初稿”提升为“可继续拆实现卡的母合同候选稿”。
- 后续仍建议在真正拆桥接实现卡前，再做一轮 focused review，专盯 API shape 与 read-model 字段名。
