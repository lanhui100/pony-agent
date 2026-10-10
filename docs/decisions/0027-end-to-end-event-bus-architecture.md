# 0027 端到端全链路 EventBus 架构设计

Status: implemented

## 背景
Pony Agent 目前的事件机制呈点对点网状分布：
1. Rust Core 内部通过 `TurnEventSink` 处理回合生命周期，通过 channel 单点通知；
2. 宿主与前端之间强依赖 Tauri 专有 `app_handle.emit` / `safeListen` 散落各处，缺乏统一分发中枢；
3. 无法无缝支持未来多宿主（Web / Headless / CLI / Server）扩展。

为了实现 Rust 核心与前端渲染的职责分离，并为未来多宿主提供强韧、一致的事件基础设施，引入端到端全链路 EventBus。

## 候选方案

### 方案 A：纯前端 TypeScript EventBus
- **描述**：仅在前端维护一个全局 EventEmitter / EventBus，组件间解耦。
- **落选原因**：无法解决 Rust 后端生命周期事件向多宿主广播的根本诉求，后端到前端仍旧是点对点胶水代码，留存大量架构债务。

### 方案 B：纯后端 Rust Core EventBus
- **描述**：仅在 `pony-agent-core` 内部提供广播总线，前端维持现状。
- **落选原因**：前端组件与各类 Store 仍需要大量散落的 `safeListen`，端到端事件协议不对齐。

### 方案 C：端到端全链路 EventBus（选中方案）
- **描述**：
  1. **Rust Core 核心层**：提供宿主无关的 `EventBus`，基于 `tokio::sync::broadcast`，支持有界容量（默认 1024）和强类型事件枚举 `AgentEvent`；
  2. **宿主适配层**：Tauri/Web 宿主作为统一订阅者，将 `AgentEvent` 统一通过 IPC/SSE 透传；
  3. **前端消费层**：统一的 TypeScript `EventBus`，支持强类型事件订阅、单次订阅（once）与安全的注销（unsubscribe）。
- **优势**：消灭网状依赖，解耦核心与宿主，天然支持未来多宿主扩展。

## 决策
在 `pony-agent-core` 建立 `AgentEventBus`，在前端 `src/lib/event-bus.ts` 建立统一强类型 `EventBus`，确立全链路事件标准。

## 影响
1. `crates/pony-agent-core/src/agent/event_bus.rs` 成为核心事件广播基础设施；
2. 前端跨组件、跨 Store 通信统一迁移至 `src/lib/event-bus.ts`；
3. 后续多宿主（Web/CLI）仅需挂载对应的 EventBus Subscriber。
