# Stage 1: ask_user 工具 + 前端组件 验收契约矩阵（PA-114）

## 0. 调研结论（Lead 冻结，供 Test Agent / Executor 对齐）

- 后端 Ask **控制平面**已完整存在（PA-076 phase-4）：
  `GovernedDispatcher` 支持 host-mediated `WaitingHost` → `Interaction` `PendingControlRequest`
  （prompt/options 提取、CAS 消费、expiry），`ask_control.rs` 适配器、Tauri 命令
  （`ask_list_pending/answer/cancel/expire`、`graph_list_ask_waits/resume_ask`）、graph Ask wait
  绑定与 resume 注入、`turn:suspended` 流程均已接线。
- **缺口 A（后端工具本体）**：模型可见工具名是 `Ask`（primitive=`echo_input`，schema 仅
  `{text}`，描述为回显占位符）；全项目 `ask_user` 字面量 0 命中。无一等公民 `ask_user`
  定义，schema 无 `options/defaultAnswer/timeoutMs`。
- **缺口 B（前端组件）**：`AskPanel.vue`（宿主侧待确认面板）+ `stores/ask.ts` 已存在并挂载
  （HomeWorkspace.vue），但聊天/轨迹内无 ask 工具调用的专属卡片，工具调用走
  `WorkspaceTurnItem.vue` 通用行渲染（`event.kind === "tools"`，行 348-381）。

## 1. 后端验收契约（Rust，`crates/pony-agent-core/tests/ask_user_acceptance.rs`）

### B1 builtin 注册
- **B1-1**：`builtin_tools()` 包含 `name == "ask_user"` 定义；`input_schema` type=object，
  properties 含 `question`(string, required)、`options`(array< string >, optional)、
  `defaultAnswer`(string, optional)、`timeoutMs`(integer, optional)，additionalProperties=false。
- **B1-2**：`ToolRegistrySnapshot::builtin()` 构建成功；存在 descriptor：
  `descriptor_id == "builtin:ask_user"`、`primitive_name == "ask_user"`、`model_name == "Ask"`、
  `kind == interactive`、`exposure == modelVisible`、`permission_declaration.host_mediated == true`
  且 `requires_approval == false`。
- **B1-3**：模型面 `provider_contract_views()`（builtin 表面）含 name "Ask"（execution_primitive
  == "ask_user"），其 schema 含 `options`；`echo_input` 不再 model-visible（走入 Internal）。

### B2 映射注册
- **B2-1**：`"Ask"` 产品名解析到 `ask_user`（经 alias 或 canonical 映射，
  `registry.resolve("Ask")` 命中 `builtin:ask_user`）；`registry.resolve("ask_user")` 亦命中；
  `resolve("echo_input")` 仍解析 echo_input 自身（兼容不回归）。
- **B2-2**：`model_visible_tool_name("ask_user") == "Ask"`；display metadata 中文名 "提问"。

### B3 受管执行路径（governed dispatcher 端到端）
- **B3-1**：`LegacyCompatiblePolicyEvaluator::evaluate` 对 `builtin:ask_user` descriptor 返回
  `PermissionVerdict::WaitingHost`。
- **B3-2**：`dispatch_governed`（descriptor `builtin:ask_user`，
  arguments=`{"question":"继续？","options":["是","否"],"description":"确认"}`，
  `DispatchContext { session_id, run_id, turn_id, host_control_available: true }`）→
  返回 `ToolControlKind::WaitingHost`；持久化 `Interaction` PendingControlRequest：
  `prompt == "继续？"`、`options == ["是","否"]`、`descriptor_id == "builtin:ask_user"`、
  state == pending。
- **B3-3**：`list_pending_asks(&dispatcher)` 可见该请求；用
  `ControlRequestAuthorization::for_request(&pending, Some(json!("是")))` 调 `answer_ask` 成功；
  consumed request state == `Consumed`，`answer == json!("是")`。
- **B3-4**：兼容不回归：以 `Ask`（model_name）/`echo_input`（legacy）dispatch 仍得到
  `WaitingHost`（LegacyCompatiblePolicyEvaluator 旧判定保留）。

## 2. 前端验收契约（vitest，`tests/acceptance/stage-1-ask-user.spec.ts`）

### F1 组件与接线
- **F1-1**：`src/lib/runtime/ask-tools.ts` 导出 `isAskToolName(name: string): boolean`，
  对 `"ask_user" / "ask" / "Ask" / "builtin:ask" / "builtin:ask_user"` 返回 true。
- **F1-2**：`src/components/ask/AskUserToolCallCard.vue`：props `{ tool }`（结构化工具调用）；
  渲染 question（优先 tool 参数 `question`→`text`→`prompt`，回退匹配 pending ask 的 prompt）。
- **F1-3**：当 store 中存在匹配的 pending ask（按 `callId`/`runId`）时渲染：
  options 按钮（含 `data-testid="ask-user-option"`）、自由文本输入
  （`data-testid="ask-user-typed-input"`）、回答（`data-testid="ask-user-answer"`）与取消
  （`data-testid="ask-user-cancel"`）按钮；点击 option 调 `store.answer`；无匹配 pending 时
  渲染"等待用户回答…"待命态（`data-testid="ask-user-waiting"`）；busy 态禁用交互。
- **F1-4**：`WorkspaceTurnItem.vue` 的 tools 事件渲染中对 `isAskToolName(tool.toolName)` 的行
  渲染 `<AskUserToolCallCard>`；其余行保持原通用渲染。

## 3. 门禁与验收测试文件

- 红相文件（Test Agent 编写，Lead 红相锚定提交）：
  - `crates/pony-agent-core/tests/ask_user_acceptance.rs`（B1~B3）
  - `tests/acceptance/stage-1-ask-user.spec.ts`（F1）
- 绿相门禁：`cargo test --test ask_user_acceptance` 全绿；`npm test`（含新 spec）全绿；
  `npm run build`（vue-tsc + vite build）通过；`npm run cargo:check:shared` 通过。
- Executor 禁止修改红相测试文件；如需调整契约，须经 Lead 一票裁决后由 Test Agent 修改。

## 4. 兼容与边界

- `echo_input`（legacy 回显）保留：`resolve("echo_input")` 与 legacy ToolRouter 行为不变；
  仅产品面暴露权移交给 `ask_user`（模型面 winner）。
- ask 的宿主应答/恢复链路（AskPanel、graph_resume_ask、take_pending_ask_injection、turn:suspended）
  属于既有基础设施，本次不改动其语义；`ask`/`Ask` 旧名字仍可解析到新工具。

## 5. Lead 裁决补充（2026-10-07，红相冻结后）

- **F1-2 语义裁定（契约矩阵 §2 与红相 spec 冲突处，以红相为准）**：卡片问题文本优先级为
  **匹配 pending ask 的 prompt 优先**，其次 tool 参数 `question → text → prompt`，最终回退
  `description`。红相 spec（stage-1-ask-user.spec.ts F1-2）冻结此顺序，Executor 已按此实现且
  spec 全绿（10/10）。§2 原文"优先 tool 参数"表述废止。
- **旧契约测试升级授权**：tools.rs/governed_executor.rs/dispatcher_matrix.rs 中冻结旧行为
  （"Ask"→echo_input、echo_input ModelVisible、tools.len()==20）的 characterization 测试，
  由 Test Agent 按本矩阵 B1-1/B1-3/B2-1 升级为新契约；Executor 不得改动测试。
- **B 组既有失败**（list_files/read_file/path_info 的 path traversal 用例报
  `requires_authorization` vs `invalid_path`）在基线 a8e9fd9 即可复现，属 PA-080 行为，
  与 PA-114 无关，不在本次交付范围内。

## 6. Lead 裁决补充 2（2026-10-07，后端对抗评审后）

- **B1-1 schema 修订（死参数剔除）**：ask_user input_schema 删除 `defaultAnswer`、`timeoutMs`
  两个参数——模型可见却全局无人消费（dispatcher 仅提取 text/question/prompt + options，
  前端卡仅读 question/text/prompt/options），等待期由 `config.control_request_expiry_ms`
  统一管辖。修订后 schema：`question`(string, required) + `options`(array<string>, optional)
  + with_description 注入的 `description`(string, required)；additionalProperties=false。
  红相 B1-1 断言同步：`defaultAnswer`/`timeoutMs` 必须**不存在**于 properties。
- **保留字扩充**：`is_reserved_builtin_alias` 增小写 `"ask"`（防外部工具注册 `ask` 被前端
  `isAskToolName` 误标为 Ask 卡片）。
- **已知非对称（不采纳，记录）**：echo_input 的 `host_mediated` 声明仍为 false，
  WaitingHost 仅由 `LegacyCompatiblePolicyEvaluator` 装配保证（B3-4）。生产 runtime 必装
  该 evaluator，故当前不可达；强行对齐声明会破坏 dispatcher_matrix System-origin 语义与
  legacy echo 契约，留后续卡跟进。