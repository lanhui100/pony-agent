# 0035 Token 指标口径：输入上下文取单次调用、生成速度按调用级即时产出

Status: implemented

## 背景

桌面端存在两处用户可见的 token 指标缺陷（用户实测，wave-15 修复）：

1. **输入 token 统计错误**：状态栏「输入」与输入框上下文圆环在多跳工具轮次（一次用户请求触发多次模型调用）下显示的是**跨调用累加值**（各跳把整段 prompt 重新计费），导致圆环可超过上下文窗口 100%、状态栏数字明显偏大。
2. **输出生成速度滞后**：生成速度只在轮次结束（或干脆没有值）时才出现，无法在每次模型调用完成时即时给出。

根因（file:line 见 `.dev-team/evidence/wave-15-root-cause.md`）：

- 后端把跨跳累加 usage 写入 hop 事件（`turn_stream.rs:697-704`），且 per-call 记录只在终态事件下发；
- 持久化 timeline 只把**整轮聚合**值盖在最后一条 `call_model` 条目上（`trace_timeline.rs:749-756`）；
- 无 per-call 记录时后端会构造一条「伪单次」记录，其 input 恰为整轮累加（`turn_flow.rs:1146-1152`）；
- 前端活跃轮合成记录不含 per-call 数据，回退到 store 的累加值（`useTraceProjection.ts:144-186` + `stores/runtime.ts:3698`）；
- 速度计算依赖轮级累加 output / 整轮 elapsed，且活跃轮分支刻意跳过本轮。

## 决策

1. **S1 当前上下文输入** := 会话中**最近一次已完成模型调用**的 `input_tokens`；严禁跨调用求和。状态栏「输入」与输入框上下文圆环**同源**使用该口径（用户裁定）。
2. **S2 会话累计计费输入** 保留在数据模型（轮级/消息级 `tokenCount`、ModelMonitor 汇总），但**不再**作为 S1 的显示来源。
3. **S3 生成速度** := 单次调用的 `output_tokens / ((duration_ms − first_token_latency_ms)/1000)`；TTFT 仅在 `latencyKind === "provider_stream"` 时抵扣；该值在该次调用完成后**立即可用**（含轮内中间 hop，不等 `turn:completed`）。
4. **S4 降级纪律**：S1 候选值若已知为多调用聚合值，一律不得显示；此时回退到「最后一条带 per-call 指标的 `call_model` 条目」，仍无则显示无数据。
5. **数据面**：`turn:hop_complete` 与 `turn:output_end` 必须携带**截至该事件的全量 per-call 记录**（不变量 I1/I2/I3，见契约 §7.10）；持久化 timeline 的每条 `call_model` 条目携带其**自身**指标，整轮聚合只落 `checkpoint_persist`。
6. 既有 wire 字段语义零变更（累加字段保持累加），新增仅为「记录透传」与前端口径消费。

## 备选方案与取舍

- **A. 直接把状态栏「输入」改成会话累计计费值**（保留跨跳累加）：与用户预期冲突（用户裁定为「当前上下文占用」），且会让状态栏与圆环语义不一致 —— 否决。
- **B. 为 hop 事件新增专用 per-call 字段（callInputTokens 等）**：`emit_stream_event` 为 25+ 位置参数，新增字段需改动全部发射点，爆炸半径大；复用既有 `provider_call_records` 字段（已存在且语义一致）更小闭合 —— 否决 B，采纳复用。
- **C. 修改 `projection.rs` 的 TurnEnd settle，使 per-call duration 不被整轮 elapsed 覆盖**：属事件折叠重建路径，改动需同步既有冻结内联断言，且仅影响「重建后且无 records」的轮次；本波登记为债务（state.json `wave15-D1`）不在最小闭包内 —— 延后。

## 影响

- 用户可见：状态栏「输入」与圆环在多跳轮次不再虚高（如 370K 而非 1.08M）；生成速度在每次调用完成即可见。
- 契约面：`turn:hop_complete` / `turn:output_end` 载荷新增 per-call 记录（向后兼容，消费者按需读取）。
- 测试面：新增跨语言 wire-replay 验收（真实 mock SSE 事件序列 → 前端投影）；修正 2 处冻结用例自身缺陷（A4 夹具矛盾、wire 累加基线自指）。
- 已知限制：初始调用完成时无 hop 事件，其 per-call 值在首个 followup 完成前不作为「最新已完成调用」展示（P3，见契约 §7.10 残余延迟）。
