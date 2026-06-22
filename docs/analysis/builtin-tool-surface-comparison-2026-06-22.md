# 内置工具面三方对比：Pony Agent vs Codex vs Claude Code

> 更新时间：2026-06-22
> 目的：通过完整对比 Pony Agent 当前工具实现与 Codex/Claude Code 的工具面，识别差距、排定优先级，供未来逐步决策实施顺序。

## 数据来源

- **Pony Agent**: `crates/pony-agent-core/src/agent/tools.rs` 中 `builtin_tools()` 注册的 13 个模型可见工具 + 4 个内部原语
- **Codex**: `codex-openai/codex-rs/core/src/tools/` 下 36+ 个工具（含 feature-gated），注册于 `spec_plan.rs`
- **Claude Code 2.1.88**: `claude-code-sourcemap/restored-src/src/tools/` 下 42+ 个工具目录（含 feature-gated）

## 1. 当前对齐状态

### 三方共有的核心工具（Pony Agent 已覆盖）

| Pony Agent | Codex 对应 | Claude Code 对应 |
|-----------|-----------|-----------------|
| `Run` | `exec_command`, `shell_command` | `Bash`, `PowerShell` |
| `Read` | 通过 `apply_patch` 间接 | `Read` |
| `Write` | 通过 `apply_patch` 间接 | `Write` |
| `Edit` | `apply_patch` | `Edit` |
| `Search` | shell `grep` | `Grep` |
| `Glob` | shell glob | `Glob` |
| `List` | shell `ls` | Glob + Read 组合 |
| `WebFetch` | — | `WebFetch` |
| `WebSearch` | `web_search` (hosted) | `WebSearch` |
| `ToolSearch` | `tool_search` | `ToolSearch` |
| `MCPResource` | `read_mcp_resource` | `ReadMcpResourceTool` |
| `Ask` | `request_user_input` | `AskUserQuestion` |
| `Plan` | `update_plan` | `EnterPlanMode`/`ExitPlanMode` |

### Codex 独有（Pony Agent 未覆盖）

| 工具 | 实现位置 | 说明 |
|------|---------|------|
| `write_stdin` | `codex-rs/core/src/tools/handlers/unified_exec/write_stdin.rs` | 向运行中进程写入 stdin |
| `view_image` | `codex-rs/core/src/tools/handlers/view_image.rs` | 查看工作区内图片（需多模态模型） |
| `get_goal` / `create_goal` / `update_goal` | `codex-rs/core/src/tools/handlers/goal/` | 线程级目标跟踪 |
| `request_permissions` | `codex-rs/core/src/tools/handlers/request_permissions.rs` | 主动请求文件/网络/执行权限 |
| `list_available_plugins_to_install` / `request_plugin_install` | `codex-rs/core/src/tools/handlers/` | 插件扩展管理 |
| `image_generation` | `codex-rs/core/src/tools/hosted_spec.rs` | 文生图（hosted API） |
| `spawn_agents_on_csv` / `report_agent_job_result` | `codex-rs/core/src/tools/handlers/agent_jobs/` | CSV 批量子代理任务 |

### Claude Code 独有（Pony Agent 未覆盖）

| 工具 | 实现位置 | 说明 |
|------|---------|------|
| `LSP` | `src/tools/LSPTool/LSPTool.js` | 语言服务器协议 — 跳转定义、查找引用、hover 信息、symbol 搜索 |
| `Config` | `src/tools/ConfigTool/ConfigTool.ts` | 运行时读取/设置配置 |
| `Skill` | `src/tools/SkillTool/SkillTool.js` | 执行 `.claude/commands/` 或 `.claude/skills/` 中定义的技能 |
| `NotebookEdit` | `src/tools/NotebookEditTool/NotebookEditTool.js` | Jupyter notebook 单元格编辑 |
| `EnterWorktree` / `ExitWorktree` | `src/tools/EnterWorktreeTool/` , `src/tools/ExitWorktreeTool/` | git worktree 隔离环境 |
| `StructuredOutput` | `src/tools/SyntheticOutputTool/SyntheticOutputTool.ts` | 非交互模式返回结构化 JSON |
| `TodoWrite` | `src/tools/TodoWriteTool/TodoWriteTool.js` | 会话内任务列表管理 |
| `SendUserMessage` (Brief) | `src/tools/BriefTool/BriefTool.ts` | 代理主动向用户发送消息 |
| `EnterPlanMode` / `ExitPlanMode` | `src/tools/EnterPlanModeTool/` , `src/tools/ExitPlanModeTool/` | 计划模式切换 |
| `CronCreate` / `CronDelete` / `CronList` | `src/tools/ScheduleCronTool/` | 定时任务调度（feature-gated） |
| `RemoteTrigger` | `src/tools/RemoteTriggerTool/` | 远程 Claude Code 代理触发 |
| `REPL` | `src/tools/REPLTool/REPLTool.js` | REPL 模式包装器（内部工具） |
| `TeamCreate` / `TeamDelete` | `src/tools/TeamCreateTool/` , `src/tools/TeamDeleteTool/` | 团队/swarm 管理 |
| `Sleep` | `src/tools/SleepTool/SleepTool.js` | 等待/定时唤醒 |

### 三方都实现的功能（Pony Agent 缺失子代理系统）

| 能力 | Codex | Claude Code | Pony Agent |
|------|-------|-------------|-----------|
| 子代理创建 | `spawn_agent` | `Agent` (原 `Task`) | ❌ |
| 子代理通信 | `send_message` | `SendMessage` | ❌ |
| 等待子代理 | `wait_agent` | Agent 内建等待 | ❌ |
| 关闭子代理 | `close_agent` | `TaskStop` | ❌ |
| 子代理列表 | `list_agents` | — | ❌ |
| 任务列表管理 | — | `TaskCreate` / `TaskGet` / `TaskList` / `TaskUpdate` | ❌ |

## 2. 差距分级

### P0 — 建议首批补齐

| 缺失 | 理由 | 参考实现 |
|------|------|---------|
| **子代理系统** (`spawn_agent`, `send_message`, `wait_agent`, `close_agent`, `list_agents`) | Codex 和 Claude Code 都有，是处理复杂多步骤任务的核心能力 | Codex: `codex-rs/core/src/tools/handlers/multi_agents_v2/`; Claude: `src/tools/AgentTool/` |
| **图片查看** (`view_image`) | 多模态模型需要读取 UI 截图、图表、PDF 等 | Codex: `codex-rs/core/src/tools/handlers/view_image.rs` |
| **权限请求** (`request_permissions`) | 安全模型增强，让 LLM 主动申请额外文件/网络/执行权限 | Codex: `codex-rs/core/src/tools/handlers/request_permissions.rs` |

### P1 — 第二波补齐

| 缺失 | 理由 | 参考实现 |
|------|------|---------|
| **写入 stdin** (`write_stdin`) | 与运行中进程交互（交互式程序、需要持续输入的脚本） | Codex: `codex-rs/core/src/tools/handlers/unified_exec/write_stdin.rs` |
| **配置管理** (`Config`) | 运行时切换模型、主题、行为配置，无需重启 | Claude: `src/tools/ConfigTool/ConfigTool.ts` |
| **代码智能** (`LSP`) | go-to-definition、查找引用、hover 信息、symbol 搜索 | Claude: `src/tools/LSPTool/LSPTool.js` |
| **任务列表管理** (`TodoWrite` / 任务 CRUD) | 会话内的结构化任务管理，类似 `todowrite` | Claude: `src/tools/TodoWriteTool/`, `src/tools/TaskCreateTool/` 等 |
| **主动消息** (`SendUserMessage`) | 代理在非交互模式下主动向用户推送状态 | Claude: `src/tools/BriefTool/BriefTool.ts` |

### P2 — 第三波补齐

| 缺失 | 参考实现 |
|------|---------|
| **技能执行** (`Skill`) | Claude: `src/tools/SkillTool/SkillTool.js` |
| **工作树隔离** (`EnterWorktree` / `ExitWorktree`) | Claude: `src/tools/EnterWorktreeTool/`, `src/tools/ExitWorktreeTool/` |
| **结构化输出** (`StructuredOutput`) | Claude: `src/tools/SyntheticOutputTool/SyntheticOutputTool.ts` |
| **Notebook 编辑** (`NotebookEdit`) | Claude: `src/tools/NotebookEditTool/` |
| **团队/Swarm** (`TeamCreate` / `TeamDelete`) | Claude: `src/tools/TeamCreateTool/`, `src/tools/TeamDeleteTool/` |
| **定时任务** (`CronCreate` / `CronDelete` / `CronList`) | Claude: `src/tools/ScheduleCronTool/` |
| **Goal 跟踪** (`get_goal` / `create_goal` / `update_goal`) | Codex: `codex-rs/core/src/tools/handlers/goal/` |
| **插件管理** (`list_available_plugins_to_install`, `request_plugin_install`) | Codex: `codex-rs/core/src/tools/handlers/list_available_plugins_to_install.rs` 等 |
| **Sleep** | Claude: `src/tools/SleepTool/SleepTool.js` |

## 3. 实施建议路线

```
Phase 1 (P0) ──────────────── 建议优先决策
├── spawn_agent         — 启动子代理
├── send_message        — 向子代理发指令
├── wait_agent          — 等待子代理完成
├── close_agent         — 关闭子代理
├── list_agents         — 列出活跃子代理
├── view_image          — 多模态图片查看
└── request_permissions — 权限请求

Phase 2 (P1) ──────────────── 核心体验增强
├── write_stdin         — 写入进程 stdin
├── Config              — 运行时配置管理
├── LSP                 — 代码智能
├── TodoWrite           — 会话任务列表
└── SendUserMessage     — 主动消息

Phase 3 (P2) ──────────────── 专业场景扩展
├── Skill               — 技能执行
├── EnterWorktree/ExitWorktree — worktree 隔离
├── StructuredOutput    — 非交互结构化输出
├── NotebookEdit        — Jupyter 编辑
├── TeamCreate/Delete   — 团队管理
├── Cron 系列            — 定时任务
├── Goal 系列            — 目标跟踪
├── 插件管理             — 扩展安装
└── Sleep               — 等待工具
```

## 4. 交叉参考

- [Pony Agent 工具实现源码](C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/tools.rs)
- [第一波工具面 OpenSpec](C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/first-wave-tool-surface/spec.md)
- [第二波工具面 OpenSpec](C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/second-wave-tool-surface/spec.md)
- [第三波工具面 OpenSpec](C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/third-wave-default-tool-alignment/spec.md)
- [工具权限模型 OpenSpec](C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/tool-permission-contract/spec.md)
- [工具可观测性 OpenSpec](C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/tool-observability-contract/spec.md)
- [Codex 工具实现目录](C:/Users/HUAWEI/Documents/pony-agent/codex-openai/codex-rs/core/src/tools/handlers/)
- [Claude Code 工具实现目录](C:/Users/HUAWEI/Documents/pony-agent/claude-code-sourcemap/restored-src/src/tools/)
- [Codex / Hermes / Claude Code 对比说明（外部参考）](C:/Users/HUAWEI/Documents/pony-agent/docs/codex-hermes-claude-code-对比说明.md)
