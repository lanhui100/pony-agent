# Backlog

## 进行中

暂无

## 待办

- [ ] `B001` - P2: `TokenUsage` 序列化字段计数与实际字段对齐
  - 路径：`crates/pony-agent-core/src/agent/provider/mod.rs:348`
  - 描述：`serializer.serialize_struct("TokenUsage", 9)?` 声明容量 9 与实际序列化的 8 个字段不符（serde hint，无功能危害，但有微量序列化容量预分配漂移）。
  - 登记来源：wave-55 遗留债登记

- [ ] `B002` - P2: `weighted_turn_speed` 与 `record_turn_speed_truth` 补齐直接单测
  - 路径：`crates/pony-agent-core/src/agent/turn_flow.rs:2377,2421`
  - 描述：加权速度计算与真值落盘函数目前仅经 `TurnTraceGuard::drop` 派生路径被间接覆盖，缺乏针对多 hop / 零耗时 / 溢出等边界情况的直接单元测试断言。
  - 登记来源：wave-55 遗留债登记

- [ ] `B003` - P2: 补齐「跨轮 calls 继承」回归测试用例
  - 路径：`crates/pony-agent-core/src/agent/turn_flow.rs` 及相关 stream/sync 运行时
  - 描述：跨轮对话与历史继续流转时的 `calls` 继承与累计逻辑缺乏专门的隔离回归用例验证，需补齐端到端契约与状态重载测试。
  - 登记来源：wave-55 遗留债登记

## 已完成

- [x] `B000` - P0: 修复 `npx vitest run` 全量门禁偶发 exit=1（ReferenceError: window is not defined @ src/stores/runtime.ts:904）
  - 路径：`src/lib/runtime/utils.ts` 与 `src/stores/runtime.ts`
  - 描述：通过环境销毁检测中断延迟宏任务回调，收口 scheduleDeferredPersist/cancelDeferredPersist 对 window 的裸解引用。
  - 验收：10 次全量连跑 exit=0 且 Errors 0。
