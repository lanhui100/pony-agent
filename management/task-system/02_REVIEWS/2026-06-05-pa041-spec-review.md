# 2026-06-05 PA-041 Spec Review

## 审核方式

- 独立子智能体 `Raman` 对 `PA-041` 的任务卡、proposal、design、tasks 与 delta spec 做只读审阅
- 主线程只采纳与“边界清晰度、验收可验证性、避免第二 truth-source”直接相关的意见

## 采纳的核心意见

1. 必须把 history-state hooks evidence 明确收紧为“persisted audit chain”，不得成为 restore / submission / history cursor 的仲裁输入
2. spec 需要补一个负向场景：缺少 hooks persistence evidence 时，reload 后只能表现为“无 hooks evidence”，不能据此重建 restore 结论
3. `transform / patch` 词汇需要统一到 hooks foundation 的 `patch` 语义，避免把请求侧收紧误写成结果侧改写
4. 验收矩阵需要显式要求 control-plane 与 runtime view 对同一条 history-state evidence 的投影一致
5. 需要补一条 source-of-truth non-regression 测试任务，专门约束 `workspace_rollback_applied / degradation_reason / history cursor` 在 hook 存在时仍只由既有合同决定

## 已执行的修订

- 更新 `PA-041` 任务卡：
  - 新增“history-state evidence 只作为 persisted audit chain”的完成标准
  - 补充缺少 hooks evidence 的负向验收与 control-plane/runtime-view 一致性要求
- 更新 `proposal.md`：
  - 明确本 change 的关键约束不是“能持久化 evidence”，而是“evidence 不长成新的 restore truth-source”
- 更新 `design.md`：
  - 统一 `patch` 术语
  - 明确 persisted evidence 不参与 restore / submission / cursor 仲裁
  - 补充 missing evidence、non-regression 与双读面一致性的验证策略
- 更新 `tasks.md`：
  - 增加 source-of-truth non-regression test
  - 将独立 spec 审核标记为已完成
- 更新 `spec.md`：
  - 补充缺少 hooks evidence 的负向场景
  - 补充 control-plane 与 runtime view 投影一致性场景
  - 补充 blocked checkout 不生成 resolved evidence 场景
  - 补充 persisted evidence 仅作 audit 载体的显式 requirement

## 结论

`PA-041` 的范围收口方向成立，且与 `PA-028 / PA-032 / PA-037 / PA-033 / PA-035` 没有明显职责冲突；本轮修订后，最关键的“第二 restore truth-source”风险已经在 spec 层被更明确地压住，可作为下一轮实现基线继续推进。
