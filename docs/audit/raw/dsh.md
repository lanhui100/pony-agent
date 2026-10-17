# B2 审计：DSH（DeepSeek Harness）agent 核心能力 22 轴

- 审计对象：`/home/dm/deepseek-harness`（TypeScript monorepo，pnpm workspace，全部包在 `packages/<group>/<pkg>/`，另有 `apps/`、`python/`、`native/`、`vendor/`、`benchmarks/`）
- 包规模：58 个 group、约 300 个 `@deepseek-ai/dsh-*` 包；架构为「全插件 Cordis」——一切能力都是可装卸的 Cordis 插件（`vendor/cordis`），组合由 `packages/bundle/*/cordis.patch.yml` 声明
- 证据口径：所有 `path:line` 相对 `/home/dm/deepseek-harness`，已用 `sed -n Np` 逐条回放校验
- 审计方式：只读；`ls` + `grep -rn` 定点取证，未通读实现

| # | 轴 | 状态 | 实现摘要（≤40字） | 证据 path:line | 备注/缺什么 |
|---|---|---|---|---|---|
| A01 | 主循环/回合引擎 | HAS | turn/step 双层驱动，流式块累积，并行/互斥工具池，AbortController 可取消 | `packages/core/agent-loop/src/agent.ts:296`（turn）、`:398`（step）、`:175`（cancel）、`packages/core/agent-loop/src/tool-calls.ts:90`（parallel 池）、`packages/core/agent-loop/src/assistant-stream.ts:122`（中断保留已产出块） | 中断由 cause 分类（disposed/aborted）；回合恢复靠持久日志重放而非内存快照 |
| A02 | 工具系统框架 | HAS | ToolRuntime 注册表 + JSON-Schema 支持子集校验 + presentation，超大结果 spill | `packages/core/tools/src/index.ts:1063`（register）、`packages/core/tools/src/json-schema.ts:385`、`packages/core/tools/src/schema.ts:478`（validateArgs）、`packages/spill/spill-policy/src/index.ts:34`（maxInlineTokens） | 无 tool_search 类工具检索（`tool_search/searchTools/discoverTools` 零命中）；动态工具只来自 MCP 连接与扩展注册；另有 `mode:'ptc'` 把 schema 折叠成 run_code SDK |
| A03 | 文件与编辑工具 | HAS | read/write/edit/str_replace_editor，hunk diff，编辑前必须读过（版本 CAS） | `packages/fs/tool-fs/src/read.ts:78`、`write.ts:73`、`edit.ts:85`、`packages/fs/tool-str-replace-editor/src/index.ts:430`、`packages/fs/tool-fs/src/diff.ts:35`、`packages/fs/fs-observation-policy/src/index.ts:82`（FS_NOT_OBSERVED） | 无 apply_patch / 多文件批量 patch（`apply_patch/multi_edit` 仅 UI 显示层引用）；write 不传版本则无条件覆盖；行号由 read 渲染层生成 |
| A04 | 代码检索工具 | HAS | glob/grep 走包内 ripgrep 二进制，默认尊重 ignore，可按需 --no-ignore | `packages/fs/tool-fs-search/src/grep.ts:285`、`glob.ts:307`、`search-core.ts:178`（`@vscode/ripgrep` rgPath）、`glob.ts:94`（--no-ignore 显式开启） | 无语义/embedding 检索；符号级导航由 A19 lsp 工具承担；raw 输出有字节上限（`search-core.ts:33`） |
| A05 | Shell 执行 | HAS | bash/pwsh 前台+后台 job，超时 SIGTERM→SIGKILL 升级，持久 PTY 终端 | `packages/shell/tool-bash/src/index.ts:374`、`:108`（run_in_background）、`packages/shell/bash-local/src/index.ts:102`（默认 120s + graceMs）、`packages/terminal/tool-terminal/src/index.ts:164`（terminal_open） | 后台输出经有界 ring 增量拉取（`packages/jobs/jobs-local/src/ring.ts:55`）；无交互式 stdin 注入（除 PTY 通道） |
| A06 | 权限与沙箱 | PARTIAL | 三档文件效果模式 + bwrap/Landlock/Seatbelt/windows-acl，审批 ask/never 全程审计 | `packages/sandbox/sandbox/src/index.ts:30`（模式枚举）、`packages/sandbox/sandbox-local/src/index.ts:70`（bwrap 探测/降级）、`packages/interaction/user-approval/src/index.ts:67`（ApprovalPolicy）、`packages/interaction/user-approval/src/types.ts:44`（approval/asked） | **缺网络出口管控**：`packages/sandbox/sandbox/README.md:167` 明确「seam 不表达 network/process/syscall」；**缺规则式 allow/deny 名单**（interaction/guard 内 `allowlist/denylist` 零命中）；`auto` 审批档由 experimental/auto-review 提供且默认关闭 |
| A07 | Hooks 生命周期 | HAS | Claude Code / Codex hooks.json 桥，命令钩子可阻断工具与提示、注入上下文、强制续 turn | `packages/hooks/hooks-claude-code/src/index.ts`、`packages/hooks/hooks-codex/src/index.ts`、`packages/hooks/hook-protocol/src/events.ts:76`（hook/invoked 入会话日志）、`hook-protocol/src/matcher.ts` | 只支持「命令钩子」子集；每次调用与结果都落 session log 可回放；原生插件走 Cordis 事件而非 hooks 协议 |
| A08 | 子代理与团队协作 | HAS | subagent 派生（spawn 全新 / fork 继承已完成 turn），控制工具，workflow 脚本编排 | `packages/subagent/tool-subagent/src/index.ts:380`、`packages/subagent/tool-subagent-control/src/index.ts:29`（send_message）、`:75`（interrupt_agent）、`packages/subagent/subagent-fork-in-process/src/index.ts:23`、`packages/workflow/tool-workflow/src/index.ts:275`、`packages/experimental/agent-team/src/task-graph.ts:36`（blockedBy DAG） | 子代理后端可换（in-process / ACP / SDK / Codex / Claude Code）；并发委派由 workflow 的 parallel/pipeline 提供；agent-team 属 experimental |
| A09 | 上下文管理与压缩 | HAS | token 阈值触发压缩摘要，工具结果超长裁剪，大输出落盘 spill，图片 offload | `packages/compaction/compaction-basic/src/config.ts:191`（thresholdTokens）、`packages/compaction/compaction-tool-result-pruner/src/index.ts:83`（pruneContent）、`packages/spill/spill-local/src/index.ts:149`、`packages/compaction/compaction/src/checkpoint.ts` | 压缩是「保留尾部 + 单条摘要」，被遮蔽内容留在日志里可确定重放；spill 通知按 token 计价守上限（`spill-policy/src/index.ts:113`） |
| A10 | 记忆与项目指令 | PARTIAL | 分层 AGENTS.md/CLAUDE.md 链（user-global + 根到 cwd），字节预算内截断 | `packages/context/agent-instructions/src/config.ts:12`（候选文件名）、`files.ts:317`（user-global + 逐级发现）、`render.ts:90`（user-global scope） | **缺跨会话记忆与笔记**：全库 `memory`/`note` 工具名零命中；也不持续 watch 文件（靠 fs 操作时机与 resume 对账）。检索词：`name: 'memory`、`name: 'note`，目录 `packages/**` |
| A11 | 会话持久化与恢复 | HAS | append-only 会话日志 + JSONL 落盘 + checkpoint 崩溃恢复 + resume/FTS 搜索 | `packages/session/session-persistence-jsonl/src/index.ts:252`、`packages/session/session-checkpoint-policy/README.md:7`（fail-closed checkpoint）、`packages/core/agent-loop/src/index.ts:253`（resume）、`packages/session-query/session-query-sqlite/src/index.ts:2`（FTS5） | session 格式带 v0→v4 迁移包；**无 rewind/undo 命令**（`'rewind'` 仅 UI 上下文枚举命中，无实现） |
| A12 | 计划模式 | HAS | `/plan` 进入，`exit_plan_mode` 呈现计划待人工批准，状态跨 resume/fork | `packages/plan/plan-mode/src/index.ts:67`（EXIT_PLAN_MODE）、`:233`（/plan 命令）、`packages/interaction/tool-ask-user`（review 提问通道） | 计划模式不锁工具（README 明说限制靠 sandbox/审批）；与 todo 无强绑定 |
| A13 | MCP 协议集成 | HAS | MCP stdio 与 streamable-http 双传输，tools/resources 注册，自动重连退避 | `packages/mcp/mcp-client/src/index.ts:54`（stdio）、`:82`（streamable-http）、`connection.ts:28`（reconnect policy）、`packages/mcp/mcp-resources/src/index.ts` | MCP prompts 模板不支持（README 明示）；OAuth 走通用 credentials/authorization 而非 MCP 专属流程；工具名 `mcp__server__tool` |
| A14 | Skill/扩展机制 | HAS | 多源 skill 注册表 + SKILL.md frontmatter 发现 + 目录/正文渐进加载，插件与斜杠命令 | `packages/skill/skill-filesystem/src/index.ts:729`（SKILL.md）、`packages/skill/tool-skill/src/index.ts:82`（skill 工具）、`packages/interaction/commands/src/index.ts:285`（命令注册）、`packages/extensions/tool-cordis/src/index.ts:23`（cordis_inspect_list） | 扩展面很宽：bundle patch 层、`boot/plugin-manager` 安装、preset 组合；skill 正文按需读取即渐进披露 |
| A15 | 模型路由与 Provider | PARTIAL | 多 provider 适配、reasoning effort 目录、指数退避+抖动重试、缓存 token 记账 | `packages/llm/llm/src/types.ts:369`（reasoning efforts）、`packages/llm/llm-retry/src/index.ts:61`（exponential+jitter）、`packages/llm/llm-deepseek-*`、`llm/llm-pi-ai` | **缺跨 provider/跨模型失败降级**：`failover` 全库零命中，重试只作用于同一 route；prompt cache 仅被动记账（cacheRead/cacheWrite），无显式 cache 断点标注 |
| A16 | 成本与用量核算 | PARTIAL | token/费用重放式测量 + cache 读写统计 + 上下文压力投影供 UI/压缩复用 | `packages/llm/token-meter/src/index.ts:101`（TokenMeter）、`projection.ts:14`（uncachedInputTokens/cacheRead/Write）、`packages/session/session-stats/src/projection.ts` | **无成本上限与熔断**：`budget/costLimit/maxCost` 无成本语义命中，预算只体现为压缩阈值（软降级）。检索词：`budget`、`costLimit`、`maxCost`，目录 `packages/llm`、`packages/session` |
| A17 | 可观测性与诊断 | HAS | OTLP 双通道（普通事件 + 会话日志，含脱敏），不变量注册表，启动诊断落盘 | `packages/telemetry/otel/src/index.ts:22`（createEventReporter）、`:30`（createSessionLogReporter）、`packages/runtime-diagnostics/invariants/src/index.ts:94`（InvariantRegistry）、`apps/cli/src/startup-diagnostics.ts:1` | 无 span/trace_id 语义（OTLP logs 而非 traces）；关联靠会话事件的 turn/step/callId 字段；`runtime-diagnostics` 是包级不变量自检，不是运行中「诊断命令」 |
| A18 | 后台任务与调度 | HAS | 后台 job 有界 ring + job_output/list/kill 工具，cron/日/周提醒，workflow 引擎 | `packages/jobs/tool-jobs/src/index.ts:311`（job_output）、`:352`、`:372`、`packages/jobs/jobs-local/src/ring.ts:55`、`packages/schedule/schedule/src/domain.ts:663`（cron 记录）、`packages/workflow/workflow/src/index.ts:157`（WorkflowEngine） | 调度语义是「向原会话投递跟进消息」，不是通用任务队列；`tool-ralph` 提供前台 Ralph 循环 |
| A19 | 代码智能与 LSP | PARTIAL | LSP stdio 客户端 + 四只读操作：定义/引用/实现/hover | `packages/lsp/tool-lsp/src/index.ts:110`（lsp 工具）、`:112`（四操作说明）、`packages/lsp/lsp-stdio/src/connection.ts` | **缺诊断注入、格式化、rename、workspace symbol**：`diagnostic` 在 packages/lsp 内只出现在「loader/stderr 诊断」注释里；也不联动类型检查 |
| A20 | 富媒体与外部能力 | HAS | 图像附件+read_image，web_search/web_fetch 多 provider，browser-use/computer-use 插槽 | `packages/attachment/attachment/src/types.ts:8`（ImageMediaType）、`packages/fs/tool-fs/src/read-image.ts:210`、`packages/web/tool-web/src/fetch.ts:457`、`packages/browser-use/browser-use/src/index.ts:15`、`packages/computer-use/computer-use/src/index.ts:15` | **无 PDF/Office 文本入模**（`document/office-to-pdf` 只为客户端预览做转换）；浏览器/桌面实际 provider 在 `experimental/`（playwright、chrome-devtools、stagehand、cua-driver） |
| A21 | 安全防御 | HAS | 凭据只存引用不入配置、审批审计事件、webhook HMAC 校验、外部内容 untrusted 提示 | `packages/credentials/credentials/src/index.ts:3`、`packages/interaction/user-approval/src/types.ts:44`（asked/decided 成对审计）、`packages/webhook/webhook-github/src/handler.ts:92`（x-hub-signature-256）、`packages/web/tool-web/src/trust.ts:7` | 提示注入防御是「提示语 + 签名验证 + sandbox」组合，**无独立注入检测器**；脱敏只在 UI/遥测边界（`telemetry/otel/src/session-log.ts:15`），工具输出原文仍进会话日志 |
| A22 | 交付与集成面 | HAS | SDK JSON-RPC 服务端、ACP 服务端、headless 单次模式、webhook 入口、性能基准 | `packages/sdk/server/src/server.ts:75`（HarnessSdkJsonRpcServer）、`packages/acp/acp/src/index.ts:379`（agent.initialize/session.new）、`packages/bundle/headless/src/index.ts:47`（stdout 事件流）、`packages/api/gateway` | 有 API/ACP/CLI 三面；**无 agent 质量评测集**，`benchmarks/` 仅性能门禁（会话打开、长会话、终端 IO 等） |

## 覆盖盲区

**取证方式**：`ls packages/**`（58 group 全枚举）+ `grep -rn <关键词> --include=*.ts` 定点取证 + 关键 `sed -n Np` 回放校验行号。

**逐轴检索关键词与目录**（ABSENT/PARTIAL 判定的「没找到」凭据）：
- 无 tool_search：`tool_search|searchTools|discoverTools`，目录 `packages/**` → 0 命中
- 无跨会话记忆：`name: 'memory|name: 'note`、`memory`（`packages/**`，排除 `lib/`、`/tests/`）→ 仅测试夹具里的文件名，无工具/服务实现
- 无 rewind/undo：`'rewind'|/undo`，目录 `packages/**` → 仅 `client/ui-chat/src/client/model/conversation-context.ts:6` 的类型枚举
- 无模型降级：`failover|fallbackModel|nextProvider`，目录 `packages/llm`、`packages/core/agent-default-model` → 0 命中
- 无成本熔断：`budget|costLimit|maxCost|costLimit`，目录 `packages/llm`、`packages/session` → 仅 token 预算/压缩阈值，无金钱语义
- 无规则 allow/deny：`allowlist|denylist`，目录 `packages/interaction`、`packages/guard`、`packages/credentials` → 0 命中（core/tools 的 allowlist 指 schema 字段白名单）
- 无网络出口管控：`network|egress|--share-net|socket`，目录 `packages/sandbox/*` → 仅 README 声明「out of scope」
- 无 LSP 诊断/格式化：`diagnostic|publishDiagnostics|format|rename`，目录 `packages/lsp` → 仅 loader/stderr 诊断注释
- 无 apply_patch/multi_edit：`apply_patch|applyPatch|multi_edit|multiEdit`，目录 `packages/**` → 仅 `client/ui-chat` 渲染层引用工具名

**明确未覆盖/未验证范围**：
- `packages/client/**`（Web GUI 浏览器半边，约 80 个 `ui-*` 包）、`packages/host/**`、`packages/boot/**` 只做了名字级确认，未审实现质量
- `native/`（node-addon-system，landlock-run 等原生件）、`python/`（Python SDK/runtime）、`apps/desktop`、`apps/web`、`website/`、`vendor/` 未审
- `packages/experimental/**` 只抽取了 agent-team、auto-review、browser/computer-use provider 四处，其余原型未评估
- `benchmarks/` 只看了目录名与 root `package.json` 脚本，未读基准实现
- 「实现是否真接线」的判定基于包 README + 关键源码行 + bundle patch 组合的交叉印证；未对每个包做端到端运行验证（本次为静态只读审计，未执行任何构建/测试）

## 该库独有能力（反向差距候选）

1. **PTC 呈现模式：把工具 schema 折叠成 `run_code` 内嵌 SDK（TS/Python）**。同一套工具既能原生直发 schema，也能让模型写代码调用，显著压缩工具 schema 的 token 占用。证据：`packages/core/tools/src/ptc.ts:30`（`RUN_CODE_NAME='run_code'`）、`packages/core/tools/src/ptc.ts:333`（`createRunCodeTool`）、`packages/ptc-runtime/`。
2. **全插件 Cordis 架构 + 运行时 API 自省**。任何能力（含 agent loop、fs、sandbox）都是可装卸插件，组合由 `bundle/*/cordis.patch.yml` 分层覆盖；模型可用 `cordis_inspect_list`/`cordis_inspect_query` 查运行时 API 目录。证据：`packages/extensions/tool-cordis/src/index.ts:23`、`:42`、`packages/bundle/`、`vendor/cordis`。
3. **确定论式会话日志：一切可重放**。压缩、token 计量、会话查询、快照测试全部基于 append-only 日志重放，附带 v0→v4 格式迁移链与 fail-closed checkpoint。证据：`packages/session/session-format-v0-to-v1`…`v3-to-v4`、`packages/llm/token-meter/src/index.ts:101`、`packages/session/session-checkpoint-policy/README.md:7`。
4. **同时兼容 Claude Code 与 Codex 的 hooks 线协议**，并把每次钩子调用与结果写入会话日志（可审计、可回放）。证据：`packages/hooks/hook-protocol/src/events.ts:76`、`packages/hooks/hooks-claude-code/`、`packages/hooks/hooks-codex/`。
5. **结构化沙箱 + 升级审批链**：bwrap → Landlock → Seatbelt → windows-acl 逐级探测降级，无法强制时 fail-closed 抛 `SANDBOX_UNAVAILABLE`，被拒后可申请「严格更宽一档」交人工批准。证据：`packages/sandbox/sandbox-local/src/index.ts:70`、`packages/sandbox/sandbox/src/escalation.ts`、`packages/sandbox/sandbox/src/diagnostics.ts`。
6. **按 token 计价的大输出 spill**：工具结果超阈值即落私有目录并给出「按 token 计价不超上限」的引用通知，模型按需回读。证据：`packages/spill/spill-policy/src/index.ts:113`、`packages/spill/spill-local/src/index.ts:149`。
7. **多形态子代理后端 + 脚本化 DAG 编排**：in-process / ACP / SDK / Codex / Claude Code 五种 child 后端共用一套委派契约，fork 以「父已完成 turn 前缀」为 seed；workflow 提供 `agent()/parallel()/pipeline()/phase()/log()` 与 run_code 沙箱执行。证据：`packages/subagent/subagent-fork-in-process/src/index.ts:23`、`packages/subagent/subagent/README.md`、`packages/workflow/workflow/src/index.ts:157`、`packages/workflow/workflow-ptc/`。
8. **同会话持久 goal + 自动续跑**：goal 作为会话内唯一目标跨 resume/fork 存活，`goal-round-driver` 可选地把工作推进若干轮。证据：`packages/goal/goal-round-driver/src/index.ts:18`、`packages/goal/README.md`。
