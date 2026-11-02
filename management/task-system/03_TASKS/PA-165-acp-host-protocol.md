# PA-165: ACP（Agent Client Protocol）宿主接入形态

## 状态
- **状态**: Backlog（已登记，待排期）
- **优先级**: P2（**强依赖 PA-164 完成后启动**）
- **Complexity**: C
- **创建日期**: 2026-11-02
- **宿主形态矩阵定位**: 四种交付形态中的第 4 种，当前**完全缺失**。

## 需求背景（物理事实）
- 现状所有宿主均以 **Tauri IPC 专有通道**接入：`src-tauri/src/tauri_adapter.rs:17` 的 `TauriTurnEventSink` + `src-tauri/src/lib.rs`（1495 行）的命令注册表。没有与 IDE / 第三方客户端通用的协议面。
- `docs/analysis/deepseek-harness-architecture-comparison-2026-08-14.md:14,43` 已把 ACP 列为 Pony 与 DSH 的**结构性差距**（DSH「多宿主平铺：web/headless/ACP/SDK 平级」vs Pony「桌面优先，SSE/CLI 规划中」）。
- 可复用底座：core 已有 `HostControlPlane` + `TurnEventSink` seam + `agent/sse_adapter.rs`；**协议层是唯一缺口**，不是运行时缺口。

## 目标
实现 ACP 服务端（JSON-RPC 2.0 over stdio），使任何 ACP 兼容客户端（Zed 等 IDE、未来的 Web/桌面壳）可用**同一协议**驱动 Pony core，实现「UI 与内核解耦」。

## 交付契约
- **C1 握手与能力声明**：`initialize` 返回 `protocolVersion` + `agentCapabilities`（loadSession / promptCapabilities / mcpCapabilities），未知能力**不静默降级**。
- **C2 会话生命周期**：`session/new`（cwd + mcpServers）、`session/load`（历史回放，映射既有 checkpoint/session store）、`session/cancel`；会话 ID 与 core `session_id` **一号一映射**、不复用。
- **C3 提示与事件映射**：`session/prompt` 入口；core `TurnEventSink` 事件 → ACP `session/update`（`agent_message_chunk` / `tool_call` / `tool_call_update` / `plan`），**双向字段可对拍**（同一 turn 的 ACP 帧与 core 事件序列一一对应）。
- **C4 宿主反向调用**：权限即审批 —— `session/request_permission` 由客户端裁决（本卡把 ACP 客户端作为审批 UI），映射到 core 的 `ask_control` / `tool_authorization_mode` 通道，**不新造权限语义**。
- **C5 fail-closed**：未知 method → JSON-RPC `-32601`；非法 state 转移 → 明确 error code 且不破坏会话；任何解析失败**不得**回退为「默认放行」或静默丢弃。
- **C6 传输边界**：仅 stdio（单客户端）；TCP/HTTP 传输**非目标**（若未来需要，另立卡并须先过鉴权设计）。

## 非目标
- 不做 WebSocket / HTTP 远程传输。
- 不做客户端侧实现（只做服务端）。
- 不改 TUI 与 Tauri 现有行为。

## 验收标准（物理收据）
1. `tests/acceptance/l2at-acp.sh`：真实子进程 + 手写 JSON-RPC 帧，覆盖 initialize / session/new / prompt / cancel / 权限反向调用 / 未知 method，落盘原始 stdout 帧与 Exit Code。
2. 一致性对拍：同一 prompt 下 core 事件序列 vs ACP 帧序列的映射**逐条**断言（不得只测连通性）。
3. `RUSTFLAGS="-D warnings" cargo check --workspace` EXIT 0；全量 `--lib` 无新增红相。

## 当前进展
已登记，未动工。

## 下一步动作
1. **前置**：PA-164 完成，事件契约与退出码冻结（否则 ACP 事件映射会二次返工）。
2. 起草 ACP 版本锁定（跟随上游 spec 版本，需记录兼容窗口）与错误码表。
3. 先做「initialize + session/new + prompt 单向事件」最小闭环，再做权限反向调用。

## 当前卡点
- **协议版本漂移风险**：ACP 仍在演进，须先锁定一个上游 commit/版本并写进 ADR，否则实现与客户端对不齐。
- 无外部 ACP 客户端可做联调时，需以**手写帧回放**作为验收真值（不能以「我实现的客户端能连通」自证）。

## 断点续跑提示
复用点全在 core 侧（`HostControlPlane`、`TurnEventSink`、`sse_adapter`），不要碰 `src-tauri/src/lib.rs` 的命令注册表——ACP 是**并行宿主**，不是替换。

## 工作量画像
- C1+C6 骨架（stdio loop / 帧编解码 / 错误码）：1.5 人日
- C2 会话生命周期与回放：1.5 人日
- C3 事件映射 + 对拍验收：2 人日
- C4 权限反向调用：1.5 人日
- C5 fail-closed 加固 + 对抗审查：1 人日
- **合计约 7~8 人日**

## OpenSpec Change
待建。

## Canonical Spec
待建：建议 `openspec/specs/acp-host-protocol/spec.md`。

## Spec 状态
未起草。
