# 前端工作台架构

## 目标

Pony Agent 第一阶段前端不是产品官网，也不是普通聊天页，而是用于承接 Rust 智能体核心的调试工作台。

## 当前结构

### `App.vue`

作为工作台页面容器，负责组织整体布局。

### `components/ChatPanel.vue`

负责展示对话入口和消息区占位，后续将接入真实输入与消息流。

### `components/RuntimeStatusPanel.vue`

负责展示 Rust 后端健康状态、运行阶段和基础运行时标签。

### `components/GraphTracePanel.vue`

负责展示 Graph 状态轨迹和工具调用预演区域。

### `components/StrategyPanel.vue`

负责展示当前重构主线和阶段重点，帮助工作台保持方向一致。

### `stores/runtime.ts`

负责集中管理运行时状态。

当前承接：

- health check
- phase
- trace steps
- tool activities

## 为什么这样拆

### 1. 用组件映射领域边界

让“聊天”“运行时”“轨迹”“策略”各自有独立容器，避免后续都堆在一个大组件里。

### 2. 用 Pinia 承接全局状态

运行时状态天然跨组件共享，不适合继续用原生 DOM 和局部变量维护。

### 3. 为 `run_turn()` 留接入位

后续 Rust runtime 打通后，可以自然接入：

- 发送输入
- 状态切换
- 工具日志
- 执行轨迹

## 下一步演进

1. 将聊天输入接入真实 store action
2. 为 runtime 增加 turn 结果结构
3. 将 trace 和 tool activity 改为由 Rust 返回
4. 增加可回放的运行日志面板
