# PA-076 Phases 4–7 + Runtime Switch Review

## 审核对象

- [design.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/harden-and-expand-agent-tool-runtime/design.md)（Decisions 5–11）
- 阶段 4–7 + 切换产物：`plan_state.rs`、`ask_control.rs`、`process.rs`、`sandbox.rs`、`web_access.rs`、`search.rs`、`image_artifact.rs`、`mcp_resources.rs`、`tool_search_elevation.rs`、`governed_executor.rs`、`runtime/mod.rs`（默认执行器切换）、`tools.rs` 接线（run/web/search/glob）、`graph.rs`（Ask wait/resume）、`control_plane/ask_plan_commands.rs`、前端 `src/stores/ask.ts|plan.ts`、`AskPanel.vue`、`PlanPanel.vue`

## 审核阵容与结论

| 角色 | 结论 | 关键发现 | 处置 |
| --- | --- | --- | --- |
| `@security-reviewer` / `@code-reviewer`（C3） | **CONDITIONAL PASS** | Ask 未接进 runtime turn loop（P1-1）；切换静默引入 120 KiB 输出预算 + 10s deadline（P1-2）；ProcessManager 未做最小环境（P1-3）；WebFetch 无界读 body + 二进制按文本解码（P1-4）；保留"预校验后交默认 HTTP client 重解析"弱模式（P1-5） | P1-2/3/4 + P2-10 本轮修复；P1-1/5、P2-6/7/8/9 记录为剩余里程碑 |

无 P0。C3 确认模块级扎实：Plan revision CAS、Ask CAS/nonce/kind、进程 truncation 证据、web fail-closed resolver、search 诚实截断、MCP source-revision 绑定 + args 回显拒绝、ToolSearch source-revision 失效均健壮。

## 本轮已修复（代码变更）

### P1-2 切换静默引入输出预算/deadline 回归
- 修复：`DispatchBudgetConfig` 增加 `unbounded_output_and_deadline` 标志；`build_governed_executor` 置位，使 legacy-compatible 模式不应用 descriptor 的 120 KiB 输出上限与 10s deadline（复刻 legacy `ToolRouter` 无工具级字节/deadline 上限）。governed 路径仍保留取消与 child-call 预算。
- 测试：`dispatcher::unbounded_legacy_budget_config_does_not_cap_output_bytes_or_deadline`（unbounded 模式 200 KiB 输出成功；默认模式 `output_budget_exceeded`）。

### P1-3 ProcessManager 未做最小环境
- 修复：`start_inner` 在 `sandbox.environment_allowlist` 非空时 `env_clear()` + 保留必需变量（PATH/SystemRoot/ComSpec 等）+ 应用 allowlist，不继承 provider key/session secret/ambient proxy（design Decision 7）。legacy（非沙箱）路径保持继承父环境。
- 测试：`process::sandboxed_child_with_allowlist_does_not_inherit_parent_secrets`。

### P1-4 WebFetch 无界读 body + 二进制按文本解码
- 修复：`web_fetch_url` 读 body 前按 Content-Type 门禁（text/* 与白名单 application/* 才解码，二进制/图片/PDF/octet-stream 拒绝 `unsupported_content_type`）；改为 `read_bounded_body` 流式读取，`MAX_WEB_BODY_BYTES = 2 MiB` 硬上限 + `truncated`/`truncationReason` 诚实证据（design Decision 8）。
- 注：该路径当前经 `FailClosedResolver` 不可达，修复为真 resolver 接线做准备。

### P2-10 `time_now` 被归为 Execute 撞沙箱门
- 修复：`tool_kind_for_name("time_now")` → `Read`（纯时钟读取，非副作用 Execute）；`workspace_run_command` 保持 Execute。

### Flake 根除：`process::large_output_is_truncated_with_evidence`
- 根因：测试用 `cmd for /l` 逐行小输出（~34 字节/迭代），而 `poll` 每 25ms 读并清空缓冲——两次 poll 间 drain 推入从未超过 1024 cap，buffer 永不截断。属测试设计缺陷非实现 bug。
- 修复：`large_output_command` 改为单次 4 KiB burst，首次 push 即截断，与 poll 时序无关。三连跑稳定。

## 记录推迟项（下一步里程碑，已写入任务卡 Next Action）

| 发现 | 处置理由 | 承接点 |
| --- | --- | --- |
| P1-1 Ask 未接进 runtime turn loop（executor 用 `DispatchContext::default()`、host `ask_dispatcher` 是独立空实例、graph bind/resume 无生产调用方、`LegacyCompatiblePolicyEvaluator` 恒 Allow 使 Ask 永不 persist） | 真实 Ask 端到端需要：单一共享 `Arc<GovernedDispatcher>` 贯穿 runtime 执行器与 host 控制面 + session-scoped `DispatchContext` + Ask 专用 policy（WaitingHost）+ turn loop bind/resume。属既定 integrator note | 阶段 4 收尾（Ask session-context 线程接线） |
| P1-5 WebFetch 保留"预校验后交默认 HTTP client 重解析"弱模式 | 需 pinned connector（连接固定到已验证地址 + peer-IP 校验 + Host/SNI 保留）才能真正关闭；现被 `FailClosedResolver` 兜住不可达 | 任务 6.2/6.3 pinned connector |
| P2-6 阶段 7 工具（view_image/MCP/ToolSearch）未注册进 builtin registry | 库已就绪 + 测试绿，缺 handler 注册与 provider-modality 编码；注册需避 `image include_bytes` 默认 2 MiB 撑爆结果预算 | 任务 7.1–7.3 注册 |
| P2-7 WebFetch redirect 预算未实现（现 `Policy::none()` fail-closed，但 ≤5 跳显式重解析/重校验缺失） | 与 P1-5 同属 pinned connector 工作 | 任务 6.2/6.3 |
| P2-8 `DispatchContext::default()` 全 session-less（pending request 的 `session_id` 为 None，跨会话检查空转） | 与 P1-1 同源：需 session-scoped context | 阶段 4 收尾 |
| P2-9 Ask 问题/选项被丢成 digest（`persist_pending_request` 合成 prompt，`options: None`） | 需把模型 Ask 参数透传进 `PendingControlRequest.prompt/options` | 阶段 4 收尾 |
| P3（CGNAT/保留地址段未覆盖、glob 无时间预算、前端 CAS 失败不 reconcile、answer-consume 与 graph-binding 非原子、child 结构化 code 塌缩） | 低风险补强 | 后续迭代 |

## 测试证据

- core lib **674/674**（含 P1-2/3 回归测试）+ matrix **27/27** + tool_router_regression 13 + session_regression 5，全量单线程通过
- 前端 vitest 328 + build 通过
- 全工作区 `cargo:check:shared` 通过
- 既有环境性 flake（provider_regression 过期断言）仍记录于任务卡 Blockers

## 结论

阶段 4–7 + runtime 切换整体 **通过（有条件的）**：模块级扎实、切换保真（read/write/search 逐字节 + Run fail-closed 为设计）、有界 P1 已修复。剩余 P1-1（Ask 真实接线）与 P1-5（pinned connector）为明确的下一个里程碑，已写入任务卡 Next Action 与两条 integrator notes。
