# Agent 核心能力完善计划（Wave 25 起）

> 依据：[agent-core-gap-matrix.md](agent-core-gap-matrix.md)（22 轴三方审计 + 15 条反证核验）
> 目标：把 4 项 P0 能力断层与 8 项 P1 落后项按"先止损、再补芯、后扩展"的顺序分批补齐，每波独立可验收、可回滚。
> 台账：沿用本项目 `.dev-team/state.json` 的 wave 约定（当前 wave 24 已收口）。

## 进度（滚动更新）

| 波次 | 状态 | 提交 | 门禁凭证 |
|---|---|---|---|
| W25 假能力收口 | ✅ 完成 | `0d162da` 红相 / `3dab551` 实现 / `27255fd` 台账 | check Exit 0；wave25 4/4；orchestration 4/4；provider 52/52；contract_view 12/12；dispatcher 27/27；src-tauri Exit 0 |
| W26 大输出 spill | ✅ 完成 | `93aa1f9` 红相 / `543d531` 实现+台账 | check Exit 0；wave26 8/8；spill 单测 10/10；wave25 4/4；provider 52/52；contract_view 12/12（primitive 37/surface 21）；src-tauri Exit 0；L3 审查 3 major 已修 |
| W27 MCP 客户端 | ⏳ 下一波 | — | — |

## 一、总原则（不可协商）

1. **先停止说谎，再补齐能力**：假接线（桩工具对模型谎报成功、空壳 `/cost`、fail-closed 的资源工具）优先级高于新功能。
2. **契约先行**：凡改动模型可见工具面、会话/持久化格式、配置结构，先由协议阶段冻结（descriptor + 错误码 + 测试契约），再实施。
3. **写域正交**：每波 `write_scopes` 明确且互斥；跨波共享文件（`tools.rs`、`provider/mod.rs`）按波次串行，不做并行写。
4. **物理凭证**：每波收口必须有 `cargo check -p pony-agent-core --all-targets` Exit 0 + 目标测试 Exit 0 的真实输出；前端改动加 `npm run typecheck`。
5. **不夹带**：只动本波范围内的代码，禁止顺手重构无关模块。

## 二、波次总览

| 波次 | 主题 | 覆盖轴 | 优先级 | 规模 | 前置依赖 |
|---|---|---|---|---|---|
| W25 | 假能力收口：桩工具从模型可见面摘除 + 显式 unavailable | G-02 / A08 部分 | P0 | S | — |
| W26 | 大输出 spill：超限结果落盘 + 引用回读 | A09 | P0 | M | — |
| W27 | MCP 客户端：`initialize`/`tools/list` 握手 + 资源面接线 | A13 | P0 | M | — |
| W28 | 回合引擎并发化：多工具调用 + 流式边跑 | A01 | P0 | L | W25 |
| W29 | 真实子代理 + workflow DAG 执行器 | A08 | P0 | XL | W28 |
| W30 | 扩展与治理面：外部命令 hooks + 工具级 allow/deny + plan 只读门禁 | A07 / A06 / A12 | P1 | M | — |
| W31 | 代码智能：真实 LSP 客户端 + 符号导航 | A19 / A04 | P1 | M | — |
| W32 | 安全试错：文件级 checkpoint / rewind | A11 | P1 | M | — |
| W33 | 平台化收尾：OTel、成本熔断、cron、SDK/ACP、浏览器自动化 | A17 / A16 / A18 / A22 / A20 | P1/P2 | L | W25–W32 |

> 排期逻辑：W25 最小且立即降低"模型被假工具欺骗"的风险；W26/W27 是独立模块、无共享写域冲突，可紧接；W28 是 W29 的地基（并发执行器同时被多子代理复用）；P1 批次在 P0 稳定后按"用户可感知收益/成本比"排序。

## 三、逐波方案

### W25 假能力收口（P0 / S）

- **问题（基线实测修正后）**：`subagent`/`workflow`/`spawn_teammate`/`list_agents`/`send_message`/`interrupt_agent`/`drain_inbox`/`team_task_*` 共 10 个工具的 handler 返回 `status:"ok"` 假回执（`orchestration.rs:200/223/236` 合成字符串与进程内静态表），模型按名硬调即被骗；同时 `provider/mod.rs` 内联断言按"这些工具在模型面"写死，**基线本就 FAILED**（真实模型面为 21 个名字，不含它们）。
- **方案**：① 新增单一能力开关 `SUBAGENT_RUNTIME_WIRED = false`；② 10 个 handler 统一返回结构化 `capability_unavailable`（含 hint 指向本计划 W29），绝不返回假 ok；③ 修正陈旧内联断言为真实 21 名工具面；④ 保留 handler 供 W29 接管。
- **写入范围**：`crates/pony-agent-core/src/agent/tools.rs`、`agent/orchestration.rs`、`agent/provider/mod.rs`（内联断言）、`crates/pony-agent-core/tests/orchestration_acceptance_test.rs`；验收测试 `tests/wave25_capability_honesty_test.rs` 由独立 Test Agent 冻结，实施者只读。
- **验收**：模型可见面精确等于 21 个真实名字；10 个工具任何调用路径均返回 `capability_unavailable`；`cargo check --all-targets`、`wave25_capability_honesty_test`、`orchestration_acceptance_test`、`provider::tests` 全绿。
- **风险**：前端若有工具目录展示需同步标注；W29 落地时回滚开关并重锚测试。

### W26 大输出 spill（P0 / M）

- **问题**：超预算直接 `output_budget_exceeded` 硬失败（`dispatcher.rs:1099/1434`），大文件拒读让模型分段重读（`tools.rs:2700`），命令输出环形缓冲直接丢弃（`process.rs:1435`）。
- **方案**：新增 `agent/spill.rs`（写入会话私有目录、内容 hash 去重、返回 `content_ref` + 字节数 + 前 N 行摘要）；截断/超限路径改为「落盘 + 返回引用」；新增 `workspace_read_segment` 之外的回读入口（或复用分段读 + ref 解析）。
- **写入范围**：新增 `crates/pony-agent-core/src/agent/spill.rs`、`tools.rs`（结果截断点）、`dispatcher.rs`（预算超限分支）、`session/store.rs`（ref 生命周期）。
- **验收**：红相测试「大输出 → 返回 content_ref 且文件存在 → 按 ref 回读内容一致」；预算超限不再硬失败而是引用化。

### W27 MCP 客户端（P0 / M）

- **问题**：`StdioMcpTransport` 能跑 JSON-RPC，但无 `initialize`/`tools/list` 握手、method 名自造、无 SSE/HTTP、资源面未接线（`tool_runtime.rs:395/452`、`runtime/tool_exec.rs:327` fail-closed）。
- **方案**：① 按 MCP spec 实现 `initialize` 能力协商 + `tools/list` → 动态注册 `mcp__<server>__<tool>`；② 长连接会话（保持子进程，逐行 JSON-RPC，带 request id 与超时）；③ `HttpMcpTransport`（reqwest + streamable-http/SSE）；④ 把 `McpResourceSurface` 接进正常路径，去掉 fail-closed。
- **写入范围**：`agent/tool_runtime.rs`、`agent/mcp_resources.rs`、`agent/runtime/tool_exec.rs`、`agent/provider/mod.rs`（动态工具注入）、新增 `agent/mcp_client.rs`。
- **验收**：本地起一个最小 MCP server（测试内 spawn）→ 握手成功 → `tools/list` 得到的工具可被调用 → 资源读取返回内容；无 server 时降级为不可用但不崩。

### W28 并发工具调用（P0 / L）

- **问题**：provider 决策是单值 `tool_call: Option<ToolCall>`（`provider/mod.rs:225`），Anthropic 多调用直接 `Err`（`:2249`），OpenAI 静默丢首（`openai_sse.rs:105`）；无流中执行。
- **方案**：① 决策结构改 `Vec<ToolCall>`（或 `tool_calls` + 兼容访问器）；② `runtime/tool_exec.rs` 按 descriptor 的 `concurrent_safe`（`tools.rs:375`）分区并发（不安全工具串行保序）；③ `turn_stream.rs` 支持流式增量派发（收到完整 tool_use 即可入池）；④ 中断保留已完成结果。
- **写入范围**：`provider/mod.rs`、`provider/openai_sse.rs`、`provider/responses_api.rs`、`runtime/{turn_runner,tool_exec,turn_stream}.rs`、`runtime/types.rs`。
- **验收**：红相「3 个只读工具同一轮并行执行，墙钟 < 串行阈值且结果顺序稳定」+「含写工具时按串行分区执行」+「中断后已完成结果保留」。
- **附带必修（W25 遗留）**：`anthropic_tools_payload` / `openai_tools_payload` / `render_tool_definitions` 的 fallback 分支（`is_builtin_tool_definition_set == false`）不按 `ToolExposure` 过滤，是假能力/内部工具回流到模型面的通道——W28 重写工具装配时统一改为按 exposure 过滤，并补单测（该分支当前无公开入口，需在 crate 内以单元测试覆盖）。

### W29 真实子代理 + workflow（P0 / XL）

- **问题**：`subagent`/`workflow` 是桩；`child_dispatch` 只是工具内 fan-out；无独立上下文派生。
- **方案**：① 新增 `agent/subagent/` 子运行时：独立 `SessionStore` + 独立上下文 + 父子消息通道 + 预算继承（复用 `budget.rs` 台账与 `child_dispatch` lineage）；② `fork_context` 以父已完成回合为 seed；③ `workflow` 落地为脚本化 DAG（phase/parallel/pipeline 语义，参考 DSH `workflow` 包）；④ 打开 W25 的能力开关并回写测试。
- **写入范围**：新增 `agent/subagent/*`、`agent/workflow/*`、`orchestration.rs`（替换桩）、`tools.rs`（exposure 回滚）、`session/store.rs`（子会话持久化）。
- **验收**：红相「子代理拥有独立上下文且父上下文不被污染」+「并发 2 个子代理结果可聚合」+「workflow 并行阶段墙钟收益」。

### W30 扩展与治理面（P1 / M）

- **A07**：`hooks.rs` 增 `CommandHookExecutor`（子进程 + JSON stdin/out + 超时 + 可阻断），支持加载 `.claude/hooks.json` 同构配置；保留 Noop 作为默认关闭态。
- **A06**：`app_settings.rs` + `governed_executor.rs` 增工具级规则（工具名/参数正则 → allow/ask/deny），替换 `LegacyCompatiblePolicyEvaluator` 的全放行。
- **A12**：plan 相位闸门——Draft 态下写类工具在 `governed_executor` 直接拒；`todo` 关联 `plan_id`。
- **验收**：hook 阻断写工具 → 工具未执行且返回 blocked；deny 规则命中 → 拒绝并留审计；Draft 态写文件 → 拒绝。

### W31 真实 LSP（P1 / M）

- **方案**：`lsp.rs` 接 `lsp-types` + JSON-RPC over stdio（server 由配置提供），实现 definition/references/hover/diagnostics；诊断作为上下文附件注入；A04 符号导航复用同一通道。
- **验收**：对一个真实 Rust/TS 文件取定义位置准确；diagnostics 非空且随编辑刷新。

### W32 文件级 checkpoint/rewind（P1 / M）

- **方案**：写类工具执行前登记文件快照（内容 hash + 备份），`session` 增 `WorkspaceSnapshot` 类型并与 history node 关联；新增 rewind 命令回滚工作区到指定节点；`workspace_rollback_capable` 置真。
- **验收**：改写 3 个文件 → rewind → 内容逐字节恢复；无快照节点 rewind 返回明确错误。

### W33 平台化收尾（P1/P2 / L）

OTel/OTLP 导出（A17）、USD 计价与硬上限熔断（A16，`/cost` 修正为真实统计）、cron 调度（A18）、SDK/ACP 服务面（A22）、浏览器自动化工具族（A20）。每项独立子波，验收各自独立。

## 四、每波固定门禁

```bash
# 1. 基线编译（必须 Exit 0，无新增告警视为通过）
cargo check -p pony-agent-core --all-targets
# 2. 目标测试（红相→绿相）
cargo test -p pony-agent-core --test <wave_test_name>
# 3. 回归（触碰 provider/session/tools 时）
cargo test -p pony-agent-core
# 4. 前端改动时
npm run typecheck && npm test
```

- 红相锚定：测试先失败（真实断言，非空跑），提交后才允许实施。
- 独立审查：每波至少 1 路 L3（本轮起 P0 波次加 1 路安全视角）。
- 收口：更新 `.dev-team/state.json`（wave/branches/status/证据），工作区 `git status` 只留本波变更。

## 五、明确不做（本计划边界）

1. 不重写 legacy `ToolRouter`（`tools.rs` 12292 行）为治理 dispatcher——渐进迁移面保留，只做定点接线。
2. 不引入第二套会话存储格式（继续 SQLite）。
3. 不为对齐基线而照搬 Cordis/插件市场等重型架构；只取"用户可感知能力"。
4. 不做 P2 中收益不明的项（如语义/向量检索），除非有具体需求驱动。
