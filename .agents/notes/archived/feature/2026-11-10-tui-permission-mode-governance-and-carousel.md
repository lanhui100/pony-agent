# Agent Note: TUI 运行态三态权限治理与状态行时分轮播

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0119-tui-permission-mode-governance-and-carousel.md` 之归档快照，仅供溯源。

日期：2026-11-10
实施说明：[`.agents/notes/implemented/feature/2026-11-10-tui-permission-mode-governance.md`](../../.agents/notes/implemented/feature/2026-11-10-tui-permission-mode-governance.md)。
取代：[ADR 0078](0078-tui-header-workspace-and-metrics-layering.md) 决策 6 的「底面行右槽只承载模型身份」口径（改为模型身份 ⇄ 权限模式 3s 轮播；底面行左槽优先级/几何/底色契约不变）。
关联：与内核态 `PermissionMode`（`crates/pony-agent-core/src/agent/headless.rs` 与 `governed_executor.rs`）及桌面端权限通道完全对齐。

## 背景

终端交互（TUI）此前缺乏运行态即时查看与切换工具执行权限的接缝，也缺乏在 `/settings` 中持久化默认权限的能力。
为遵循 DeepSeek Harness 的权限模型分层设计（Fail-Closed 原则），TUI 需要具备：
1. 在 `/settings` 中配置并持久化 `default_permission_mode`（只读 / 工作区写入 / 完全访问）；
2. 支持 `/permission` 斜杠命令与 `Ctrl+P` 快捷键，在会话中即时切换权限模式并同步注入内核；
3. 在输入框右下角状态行将模型信息与当前权限模式进行平滑时分轮播（3 秒交替）。

## 候选方案与权衡 (Alternatives considered)

- **A（采纳）对齐内核 PermissionMode + 独立首选项持久化 + 精确轮播时钟**：
  复用内核 `PermissionMode` 三态，通过 `tui-preferences.json` 持久化默认权限；运行态通过 `Ctrl+P` 与 `/permission` 命令即时轮转或直接切档，下发底层 `TurnInput.tool_authorization_mode`；状态行右槽以 3 秒时钟片与模型身份平滑轮播。
- **B（落选）在 TUI 内部维护独立权限策略求值**：
  导致 TUI 与桌面端、ACP 宿主的权限裁决逻辑分叉，破坏高内聚治理，增加安全失控风险。
- **C（落选）在 status_line 将模型和权限同时水平并排**：
  移动端/窄终端下排版容易挤爆，与已有指标/排队提示产生冲突；时分复用（分时轮播）在保留全部信息的同时不占用横向空间。

## 决策

**相位与断言纪律（2026-11-11 补充，随 Windows 全量首跑收敛）**：

- **相位是显式状态，不是墙钟巧合**：`status_carousel_phase(state)` 以
  `state.status_carousel_started_at: Option<Instant>` 为唯一相位源（`None` 时退化用墙钟毫秒取模）。
  断言必须钉住该字段（`Some(Instant::now() - 偏移)`，偏移/周期取
  `STATUS_CAROUSEL_PERIOD_MS / STATUS_CAROUSEL_MODEL_HOLD_MS`），否则相位随机、断言偶发红。
- **两相位断言口径**：模型相位 = `model`（有思考强度时 `model · <档位>`）；权限相位 =
  `权限 · 只读 / 权限 · 工作区 / 权限 · 完全访问`（对应 `PermissionMode` 三态）。
  比较粒度分两种：**ASCII 期望按整行 `trim` 后逐字相等**（空白差异也会红）；**CJK 期望按字形序列
  相等**（宽字符右侧填充格为空格，故有意不区分 `权限·工作区` 与 `权限 · 工作区` 这类排版差异，
  只捕获字形/措辞与分隔符存在性变化）——两种口径都在断言里写明，不混用。
- **执行面**：Linux 侧此前只跑 `--lib` 单测（集成测试只在 windows job），故相位不确定性与平台差异
  只在 windows-latest 暴露；PTY 子集自本批起由 ubuntu job 的独立步骤执行（见 ADR 0121 决策 4）。

1. **权限契约统一**：直接复用 `pony_agent_core::agent::headless::PermissionMode`，通过 `TurnInput.tool_authorization_mode` 透传至执行内核。
2. **偏好持久化**：`TuiPreferences` 扩展 `default_permission_mode`（默认 `WorkspaceWrite`），支持向前兼容反序列化。
3. **命令与快捷键**：`AVAILABLE_COMMANDS` 注册 `/permission`，全局拦截 `Ctrl+P`，即时轮转权限模式并持久化至配置文件。
4. **状态行轮播**：定义 `STATUS_CAROUSEL_MODEL_HOLD_MS = 3_000` 与 `STATUS_CAROUSEL_PERM_HOLD_MS = 3_000`，状态机在事件循环内精准调度微秒级唤醒，实现平滑轮播。

