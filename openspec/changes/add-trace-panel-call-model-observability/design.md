## Context

`HomeSidebar` 当前已经把 trace timeline 归一为 `build_context / call_model / call_tool / return_result` 语义，并且能从 `turn.traceTimeline`、`turn.providerCallRecords`、turn 级 token 统计中恢复部分 hop 指标。

但现状仍有三个问题：

1. `call_model` 的 cache hit 与 TTFT 只在部分数据形态下可见，缺少“单次调用必须可解释”的稳定合同
2. `call_model` 明细只展示 `reasoningContent` 和 `text`，无法如实反映“这次调用输出的是工具调用而不是消息”这类关键事实
3. 多 hop 轮次里，`call_tool` 明细与触发它的 `call_model` 是分散展示的，用户很难从 `call_model` 视角理解该 hop 实际产出了什么

本变更只修复 trace 面板的读面与归因，不改 provider/runtime 的执行协议。

## Goals / Non-Goals

**Goals:**

- 让每个 `call_model` 节点都优先展示单次调用级别的缓存命中、TTFT、输出 token 与耗时
- 让 `call_model` 明细能够保真呈现该次调用输出的工具调用和消息输出
- 让多 hop 时的工具调用归因稳定，不把最终 assistant 文本错误回填到前一个 hop
- 以现有 runtime/store contract 为主完成前端增强，并用高层测试锁住行为

**Non-Goals:**

- 重写 Rust runtime、provider adapter 或 trace 事件协议
- 重新设计 `ModelMonitorPage` 的监控读面
- 新增远程 telemetry、统计后台或独立浏览器 E2E 基建
- 把所有 trace 节点都改造成新的可视化范式

## Decisions

### 1. 指标解析继续以现有 `traceTimeline + providerCallRecords + turn fallback` 为主

原因：

- `src/types/runtime.ts` 与 `src/stores/runtime.ts` 已经具备 `firstTokenLatencyMs`、`cacheHitInputTokens`、`providerCallRecords`、`traceTimeline` 等字段
- 对最后一个 `call_model`，当前实现已经允许从整轮 turn 级聚合值回退
- 继续复用这条链路可以避免引入新的后端写面

替代方案：

- 直接重做 Rust trace payload，让每个 `call_model` 都携带完整 metrics
- 该方案改动更大，不符合本卡“前端 trace 读面补完”的边界

### 2. `call_model` 输出采用“本次调用产出块”模型

具体做法：

- 保留已有的 `思考链` 与 `模型输出` 区块
- 新增从相邻 `call_tool` 节点派生的“工具调用输出”区块
- 归因规则为：某个 `call_model` 之后、下一个 `call_model` 之前出现的 `call_tool` 节点，都视为该次模型调用直接产出的工具调用输出

原因：

- 当前 trace timeline 已有稳定的顺序语义，足够支撑前端在不改后端协议的情况下恢复“这次模型调用产出了哪些工具调用”
- 用户需要在 `call_model` 视角看到真实输出，而不仅是单独点进 `call_tool`

替代方案：

- 只在 `call_tool` 节点展示工具信息，不回挂到 `call_model`
- 这无法满足“单次模型调用的输出必须如实展示”的需求

### 3. 保持 `call_tool` 独立节点，同时允许 `call_model` 明细镜像其输出

原因：

- `call_tool` 节点仍然承担执行期时序与结果明细展示，不能被 `call_model` 吞并
- `call_model` 明细镜像工具调用输出是为了补完“模型发出了什么”，不是为了取消 `call_tool` 的独立语义

### 4. 验证以高层组件/store 联动测试作为当前前端端到端代理

原因：

- 仓库当前没有 Playwright/Cypress 级浏览器 E2E 基建
- `tests/HomeSidebar.spec.ts` 和 `tests/runtime-store.spec.ts` 已覆盖真实 store -> trace history -> UI 渲染链路，是当前前端端到端验收的最高保真现有手段

因此本卡要求：

- 新增覆盖多 hop / 工具输出 / 指标保真的高层用例
- 跑通相关前端测试集并修复回归

## Risks / Trade-offs

- [风险] 前端按相邻时间线关联 `call_model` 与 `call_tool`，若后端未来改变 trace 排序语义会失效
  → 缓解：把相邻关联规则写入 spec 与测试，若后端语义变化必须同步更新 contract

- [风险] 在 `call_model` 与 `call_tool` 两处同时看到工具调用明细，存在信息重复
  → 缓解：`call_model` 仅强调“本次模型调用产出的工具调用”，`call_tool` 继续承担执行结果与耗时细节

- [风险] 某些 provider 只返回 buffered response，TTFT 不应被伪造
  → 缓解：继续沿用现有 latency kind 判定，只有真实 `provider_stream` TTFT 才展示
