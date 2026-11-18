# Agent Note: Windows CI 可移植性与新波次后的冻结锚点同步

Status: implemented

工单：B230（Windows CI 红）、B246（冻结锚点/快照同步）。影响面见文末清单。

## Problem

2026-11-18 的 Windows `verify` 任务（32k 行日志）暴露 25 个失败套件，可归为四类：

1. **Windows 路径形态**：`std::fs::canonicalize` 产出 `\\?\C:\...` verbatim 前缀；
   - `subagent_runtime::workspace_root_matches` 与注册表里的可读路径（`normalize_workspace_root`
     已剥前缀）比对恒 miss ⇒ 子代理 root 反查失败；
   - ACP fake agent 是 POSIX shell 脚本，Git Bash 无法打开 verbatim 路径 ⇒ capture 文件静默缺失、
     `terminal/create` 里内嵌的 marker 路径被 shell 吃掉反斜杠 ⇒ JSON 非法 escape ⇒ 超时。
2. **XDG 契约在 Windows 失效**：`agents_file_path()` 走 `dirs::config_dir()`（Windows =
   KnownFolder/RoamingAppData，忽略 XDG），而冻结契约明示 ACP 清单只经
   `$XDG_CONFIG_HOME/pony-agent/acp-agents.json` 发现；测试的隔离根在 Windows 上不生效 ⇒
   全部 ACP 委派用例报 `acp_config_agent_not_found`。
3. **冻结锚点未随新波次同步**（平台无关，Linux 同样红，此前只有 Windows 跑全量测试）：
   工具面 38→39 / 45→46（`acp_delegate`）、内部注册表 45→46、`/title` 入 TUI 命令表、
   `list_agents` 增 `wait_ms` 导致 schema 逐字节 hash 变化、`acp_delegate` schema 变化、
   B217 终态三分后失败轮投影 kind 从 `Interrupted` 改为 `Error`。
4. **提示词模板被 JSON 转义污染**：`TEAM_POLICY_TEMPLATE` 的换行是字面量 `\n`（单行渲染），
   且有一处 `spawn_teammate` 裸名未用 `{{tool:...}}`；快照/结构锚点仍是旧文案。

## Decision

- `subagent_runtime::workspace_root_matches`：比较前统一 `normalize_win_prefix` 去 verbatim，
  再走 `roots_match`（平台大小写语义同源）。
- 测试路径统一去 verbatim：新增 `tests/support/paths.rs::deverbatim`，ACP 集成测试的临时根
  在 canonicalize 后即归一，Rust IO 与 shell 参数共用同一可读路径。
- `acp::config::agents_file_path()`：显式优先读**已有** `XDG_CONFIG_HOME`（不新增 env 变量），
  Windows 上也遵守冻结契约；未设置时回落 `dirs::config_dir()`，产品默认行为不变。
- 冻结锚点按真实契约同步（每条注明来源/工单）：
  39/46 工具面与内部注册表、`/title`、`list_agents`/`acp_delegate` schema hash、
  B217 终态三分的 `Error`、团队策略模板改回真实换行 + 全占位符 + 快照重锚。
- `wave2_b070` 的「无 XDG 兜底」用例在 Windows 无等价环境变量 ⇒ 平台 gating（带理由）。

## Alternatives considered

- **A. 把 ACP shell 夹具整体改写为 Rust fake agent。** 落选：19 个套件、数十种模式，
  正确但工作量与回归面远超本卡；去 verbatim 即可让现有夹具在 Git Bash 上跑通。
- **B. Windows 上把 ACP 集成套件全部 `#[ignore]`。** 落选：会丢掉 Windows 上 ACP 协议/委派
  覆盖；先修可修的路径与配置面，只对确无平台等价物的用例做窄 gating。
- **C. 让 `agents_file_path` 新增专用 env（如 PONY_AGENT_ACP_CONFIG）。** 落选：契约明示
  「不新增 env 变量」；`XDG_CONFIG_HOME` 已是文档口径。
- **D. 只改测试不改生产（workspace_root_matches 侧的 verbatim）。** 落选：生产调用方同样可能
  传 canonical 路径，属真实缺陷而非测试缺陷。

## Verification

- 本地 `cargo-lane check -p pony-agent-core --tests` exit 0；
- 修复套件逐个复跑：wave61 / tool_surface_order / tool_surface_singularity / prompt_tool_consistency /
  system_prompt_structure / slash_commands / wave42_skill_body / adr0123 / wave2_b070 全绿；
- 全量 `--lib` 1620 passed / 0 failed（B234/B242 同批）；
- Windows 侧以 GitHub Actions `verify` 收据为准（本机无 Windows）。
