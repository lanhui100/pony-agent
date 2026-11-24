# Agent Note: ACP P2 只读清单面——桌面端 `acp_list_agents` 与前端只读投影

Status: implemented

## Problem

ACP 外部 agent 的能力在核心层已就绪：`acp::config`（清单读写与校验）、`acp::registry`（活跃委派只读面）、
`acp::session`（会话驱动）、CLI 侧 `ponycli acp --list` 已能列出清单。但**桌面端没有任何入口**：
前端既无 `AcpAgentSpec` 类型，也没有读取清单的 Tauri 命令，UI 无法呈现「可选外部 agent」，
B163 登记的 P2 可选面因此只完成了核心层一半。

同时存在两个容易做错的边界：

1. **清单读取是阻塞文件 IO**（`<HOME>/.pony/acp-agents.json`），直接在 async 命令体里读会占住 runtime 线程；
2. **清单语义是 fail-closed 的**：文件缺省 ⇒ 内置默认两条；文件存在但解析/校验失败 ⇒ 必须报错，
   不能用空列表冒充「没有 agent」（否则 UI 会把配置损坏显示成「未安装任何 agent」）。

## Decision

1. **新增只读命令 `acp_list_agents`**（`src-tauri/src/lib.rs`），返回
   `Vec<AcpAgentSpec>`；语义逐条对齐 `ponycli acp --list`：
   - 清单文件不存在 ⇒ `load_agents()` 返回内置默认（`opencode` / `antigravity`）；
   - 解析/校验失败 ⇒ `Err("<code>: <message>")`，错误码取 `AcpConfigError::code()`（与 CLI 同一真源）。
2. **阻塞 IO 走 `BlockingHelper::spawn`**：`load_agents()` 同步读文件 + 校验，落在 blocking 线程池，
   与 `list_sessions` 等既有命令同一口径，不阻塞 async runtime。
3. **前端只投影 UI 消费字段**（`src/types/acp.ts`）：`id/label/command/args/env/defaultCwd/
   capabilitiesHint/inheritProxyEnv` 逐字段对齐（Rust 侧 `serde(rename_all = "camelCase")` 已保证线缆形状）；
   `mcpServers` 的 stdio/http/sse 三变体属于**清单写面**，只读面不消费，故用 `unknown[] | null` 占位并注明真源，
   **不**在前端造第二份结构真源。
4. **API 封装只做透传**（`src/lib/runtime/acp-api.ts`）：`listAcpAgents()` 不吞错、不兜底空数组，
   由调用方决定错误态呈现。
5. **观测归属**：`docs/observability.yaml` 追加本命令的归属说明——无新增出网/跨进程出口、无新增 span/metric，
   metrics/logs 维持结构性 N-A。
6. **范围切分**：本 PR 只做只读清单面。B163 的另外两个命令（`acp_delegate_start` / `acp_delegate_cancel`）
   依赖 turn 生命周期接线（`turn_task_registry` + 事件出口 + 取消注册）与前端交互态，属不同主题，
   拆到后续 PR；B163 保持 `todo` 并在账本注明拆分。

## Alternatives considered

- **三命令一起落地（list + start + cancel）**：start/cancel 需要把 ACP 委派接进 turn 任务注册表与事件出口，
  改动面横跨 `src-tauri` 与前端交互态，和「只读清单」不是同一主题；一个 PR 里混两类变更会削弱可审性，
  也让 fail-closed 语义（只读面）与长任务生命周期（控制面）互相拖累验证。
- **前端直接读配置文件（localStorage / fs 插件）**：绕过 Rust 边界，且清单路径/校验规则会变成两份实现；
  与「持久化与大模型交互必须在 Rust 侧」的架构约束冲突。
- **命令直接返回 `builtin_agent_specs()` 常量**：实现最简单，但会把用户清单（含自定义 command/env）丢掉，
  与 `ponycli acp --list` 语义不一致，属于静默降级。
- **失败时返回空列表 + 前端提示**：会让「配置损坏」与「未安装 agent」在 UI 上不可区分，
  违背 fail-closed 的既有口径。
