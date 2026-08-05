# PA-076 Phase 7 Review — view_image / MCP Resources / ToolSearch Elevation + P2-6 Registration

## 审核对象

- [design.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/harden-and-expand-agent-tool-runtime/design.md)（Decision 10/11、Verification Strategy）
- task 7.1–7.4 + P2-6（阶段 7 工具注册）
- 阶段 7 产物：
  - `crates/pony-agent-core/src/agent/image_artifact.rs`（`view_workspace_image`、`ImageReadOptions` default `include_bytes=false`、`ViewImageHandler` :338-416、magic-byte MIME/尺寸/字节上限）
  - `crates/pony-agent-core/src/agent/mcp_resources.rs`（`McpResourceSurface`：list resources / list templates / read、source-bound `McpTransport`、untrusted content 边界、独立 `ResourceTemplate`）
  - `crates/pony-agent-core/src/agent/tool_search_elevation.rs`（`ToolSearchElevator`：Deferred 候选 → 当前 turn `TurnToolView` 提升、`source_revision` 校验、trace evidence）
  - `crates/pony-agent-core/src/agent/tools.rs`（`builtin_tools()` 新增 `plan_control`/`view_image` 定义 :3727-3835、`contract_priority`/`tool_kind_for_name`/`exposure`/aliases/permission facts/execution policy、`builtin_turn_tool_contract_views` 前端投影、MCP/ToolSearch 边界裁决注释 :3541-3591）
  - `crates/pony-agent-core/src/agent/governed_executor.rs`（注册循环 :136-200 接 `PlanControlHandler`/`ViewImageHandler`、MCP/ToolSearch 裁决注释 :169-178）
  - `crates/pony-agent-core/src/agent/plan_state.rs`（`PlanControlHandler` 本身）
  - 关联生产路径：`crates/pony-agent-core/src/agent/runtime/tool_exec.rs`、`capability_bridge.rs`、`runtime/mod.rs`（`apply_governed_turn_context`）

## 审核阵容与结论

| 角色 | 结论 | 关键发现 | 处置 |
| --- | --- | --- | --- |
| `@architect-code` | **CONDITIONAL PASS** | phase-7 MCP/ToolSearch 安全面（`McpResourceSurface`/`ToolSearchElevator`）未接入生产 capability registry 路径；生产 MCP resource read 把请求参数回显为 content，违反 Decision 11；governed 注册循环注释与代码不一致（MCP/ToolSearch 仍落入 router 兜底） | P1-1 / P2-1 修复 |
| `@security-reviewer` | **CONDITIONAL PASS** | Plan `session_id` 由模型参数自报、不从 dispatch context 注入，`CrossSession` 对自报 key 不可强制（T3 遗留）；view_image `maxBytes` 无上限钳制可致超大预分配 | P1-2 / P2-2 修复 |

无 P0。两条不变量（descriptor 顺序 = stable prefix 不重排；外部/未知工具名透传不回落到 builtin 产品名）经代码与测试双重确认守住；`plan_control`/`view_image` 已按「统一 registry/dispatcher 解析与执行」注册并可经生产 dispatch 路径到达；`include_bytes=false` 默认确实避免把文件字节撑进结果；view_image 路径防逃逸（canonicalize + 组件级 `starts_with`）对绝对/相对越界、符号链接解析后逃逸均 fail-closed。

## 正面确认（验收 #1/#8、不变量、决策记录）

1. **注册满足统一 registry/dispatcher（验收 #1/#8）**：`plan_control`/`view_image` 在 `builtin_tools()` 末尾追加（`tools.rs:3731-3732` 注释明确稳定前缀），经 `ToolRegistrySnapshot::builtin()` → descriptor（`builtin:plan_control`/`builtin:view_image`），在 `build_governed_executor` 注册循环（`governed_executor.rs:177-190`）绑定 `PlanControlHandler`/`ViewImageHandler`。生产路径：runtime 能力解析把产品名映射为 primitive 名 → `GovernedToolExecutor::execute` → `dispatcher.resolve_descriptor`（`dispatcher.rs:1118-1127`）按 alias 解析 → 命中注册 handler。`builtin_turn_tool_contract_views()`（前端/provider 投影）经测试断言包含 `Plan`/`ViewImage`（`tools.rs:3872-3911`、`provider/mod.rs:3860-3873` 断言追加在尾部、13-tool 前缀不变）。
2. **不变量 1（descriptor 顺序 = stable prefix）**：`from_builtin_definitions` 以 `product_first_index` 保首次出现槽位（`tools.rs:377-391`、446-457），`Plan`/`ViewImage` 追加于 14/15 号槽；contract view 断言 15 个产品名顺序（`tools.rs:3872-3891`）；provider 测试断言尾部追加（`provider/mod.rs:3870-3873`）。未重排既有前缀。
3. **不变量 2（外部/未知工具名透传）**：`model_visible_tool_name_opt`/`product_visible_tool_name` 对未知名返回 `None` → 原样透传（`tools.rs:4384-4421`），`tool_kind_for_name` 未知名回落 `External`。`plan_control`/`view_image` 的别名/产品名映射（`tools.rs:4268-4292`、4314-4324、4382-4403）均无捕获未知名路径。
4. **MCP/ToolSearch 留在 capability registry 的裁决避免双执行**：当前无真实双执行——runtime 在 `execute_registered_tool_call` 对 `mcp_resource_read`/`tool_search` 先拦截到 capability registry 路径（`tool_exec.rs:197-235`），不达 executor；且 legacy `ToolRouter` 对二者返回 `deferred_to_registry` fail-closed（`tools.rs:988-999`）。决策在 `tools.rs:3541-3546`、3566-3571 与 `governed_executor.rs:172-176` 均有文字记录。
5. **`include_bytes=false` 默认生效**：`ImageReadOptions::default()`（`image_artifact.rs:36-45`）与 handler 缺省（`image_artifact.rs:364-369`）均为 false；测试 `include_bytes_default_is_false_and_reference_only`/`handler_returns_reference_artifact_with_default_options` 验证 `bytes == null`。
6. **view_image 路径防逃逸**：`resolve_inside_workspace`（`image_artifact.rs:130-149`）canonicalize 后按组件级 `starts_with` 校验，符号链接解析后逃逸被拒；测试覆盖绝对/相对 `..` 越界（`image_artifact.rs:568-595`）。
7. **ToolSearch `source_revision` 失效可靠**：`elevate` 对当前 snapshot 再查 descriptor + 复核 `Deferred` + 比对 candidate revision + `view.elevate_from_registry` 的 snapshot 绑定（`tool_search_elevation.rs:107-156`、`tool_runtime.rs:208-216`）；测试覆盖 revision 过期、descriptor 被删、view 与 snapshot 不匹配。

---

## P1

### P1-1 phase-7 MCP/ToolSearch 安全面未接入生产执行路径；生产 MCP resource read 把请求参数回显为 content（design / security / 验收 #8）

**位置**：`mcp_resources.rs:124`（`McpResourceSurface` 全仓唯一消费方是其测试模块，生产无调用）；`tool_search_elevation.rs:41`（同）；`runtime/tool_exec.rs:285-298, 375`；`capability_bridge.rs:742-754`；`docs/architecture/tool-runtime-descriptor-registry.md:289`

**失败场景**：
- `McpResourceSurface` 声称的 untrusted-content 边界（`McpResourceLimits`：items/uri/name/description/content items/content bytes/content chars，`mcp_resources.rs:22-51`）、`reject_argument_echo`（`mcp_resources.rs:241-251`）、content uri 匹配校验（`mcp_resources.rs:432-436`）只在单元测试中生效。生产 MCP resource 路径 `execute_resource_registry_tool_call` → `resource_fetch_success_result(action, arguments)` 把**请求参数对象原样作为 resource content** 返回（`tool_exec.rs:298`、`375`），正是 design.md Decision 11 明文禁止的「不得把 arguments 回显为资源内容」，也正是 `McpResourceSurface::reject_argument_echo` 专门要拒绝的模式。
- `ToolSearchElevator` 的 Deferred 候选 → 当前 turn `TurnToolView` 提升、`source_revision` 校验、trace evidence 均未接入生产；生产 `execute_tool_search_registry_tool_call`（`tool_exec.rs:405-470`）用 `list_capabilities` + confidence 打分，是另一套简化实现，不满足 Decision 10 的 turn 级提升语义。
- 架构文档 `tool-runtime-descriptor-registry.md:289` 声称 `McpTransport` 「Wired — phase 7」，与实际（仅测试接线）不符，会误导后续接线决策。

**影响**：验收 #8「MCP resource list/template/read 和 deferred 工具 turn 级提升**可用**并有测试」中「可用」在生产未达成——当前生产是库 + 测试、无真实接线；且生产路径违反书面设计决策。由于生产尚无真实 `McpTransport`（无真实 fetch），今日实际泄露面有限（回显的是模型自己的参数），但 MCP source 注册后此路径即为资源读取主通道，设计上属于未满足项。

**建议处置**：
- 将 `McpResourceSurface`（source-bound transport + 边界 + echo 拒绝）接入 capability registry 的 `Resource` 分支，替换 `resource_fetch_success_result(action, arguments)` 的回显；或将 `execute_resource_registry_tool_call` 在无真实 transport 时改为 `MalformedResponse`/`SourceUnavailable` fail-closed，**不得回显参数**。
- 将 `ToolSearchElevator` 接入 `execute_tool_search_registry_tool_call`（或明确在 runtime 层使用它），使提升、revision 校验、trace 生效。
- 更正架构文档的「Wired」表述，标注两个模块为「库就绪、接线待办」，并挂接到对应的接线里程碑。

### P1-2 Plan `session_id` 由模型参数自报、不从 dispatch context 注入，`CrossSession` 对自报 key 不可强制（architect / security，T3 遗留确认）

**位置**：`plan_state.rs:432-453`（`PlanControlOperation` 各 op 的 `session_id` 来自 arguments）；`plan_state.rs:499-511`（handler 从 arguments 反序列化）；`tool_runtime.rs:36-39`（`PrimitiveToolHandlerRequest` 仅 `descriptor_id` + `arguments`，无 session 字段）；`governed_executor.rs:96-113`（dispatch 时 `DispatchContext.session_id` 不进入 handler request）；`governed_executor.rs:464`（测试把 `session_id` 显式放进模型参数）

**失败场景**：
- `PlanStore` 以「模型自报的 `session_id`」为隔离 key（`plan_state.rs:148-151`、204-206）。`CrossSession` 守卫（`plan_state.rs:385-402`）只按 key 不同隔离，**不校验 key 的真实性**——模型在 session A 的 turn 里传 `session_id: "session-B"` 即可在 session B 命名空间 create/replace/complete。
- `plan_id`/`step_id` 由 `AtomicU64` 顺序生成（`plan-1`、`step-1`…，`plan_state.rs:341-349`），可枚举，便于对共享 executor 上的其他 session 的 plan 做盲操作。
- 当前每个 runtime 一个 executor（`runtime/mod.rs:446-447`），单 session 场景风险主要为命名空间污染/混淆；但 Ask 接线既定方向是「单一共享 dispatcher」（见 PA-076 任务卡 Next Action #1），届时跨 session 风险升级为可实际读写其他 session 的 plan。

**影响**：design.md Decision 6「session-owned + CrossSession 守卫」的隔离保证对自报 client 不可强制；`session_id` 作为安全边界应从已鉴权的 dispatch context 注入，而不是信任模型参数。

**建议处置**：扩展 `PrimitiveToolHandlerRequest` 携带 dispatch-context `session_id`（或让 dispatcher 在调用 handler 前用 `context.session_id` 覆盖/校验模型参数），对自报 session 与上下文不一致 fail-closed；在「单一共享 dispatcher」里程碑落地前必须完成。

---

## P2

### P2-1 governed 注册循环注释与代码不一致：MCP/ToolSearch 未显式排除，落入 router 兜底（architect）

**位置**：`governed_executor.rs:172-197`；`tools.rs:988-999`

**失败场景**：注释声称对 `mcp_resource_read`/`tool_search`「Deliberately NOT registering governed handlers」，但代码对二者无排除分支，落入 `RouterPrimitiveHandler` 兜底。当前因 legacy `ToolRouter` 对二者返回 `deferred_to_registry`（fail-closed）而无实际双执行——但这是**碰巧**安全，不是结构性排除。若 legacy router 日后实现这两个 primitive（迁移期常见），governed executor 会无预警地出现第二执行路径，绕过 capability registry 的 mediation/hooks。

**建议处置**：在循环中显式 `if matches!(primitive, TOOL_MCP_RESOURCE_READ | TOOL_TOOL_SEARCH) { continue; }`，与注释/决策一致；并加测试断言这两个 descriptor 未注册 governed handler（当前无此断言）。

### P2-2 view_image 参数无上限钳制，`maxBytes` 巨大值可致超大预分配（security）

**位置**：`image_artifact.rs:380-384`（handler 从 arguments 读 `maxBytes` 无 clamp）；`image_artifact.rs:112`（`read_bounded(.., options.max_bytes as usize)`）；`image_artifact.rs:174`（`Vec::with_capacity(limit)`）

**失败场景**：模型传 `maxBytes: 18446744073709551615` → `Vec::with_capacity(usize::MAX)` → allocator `handle_alloc_error` → 进程 abort（可用性）。`maxBytes == 0` 已拒绝（`image_artifact.rs:82-84`），但无上限。仅模型可触发，但属未受控输入，且该 handler 未来可能被 child/host origin 复用。

**建议处置**：对 `maxBytes`（以及 `maxWidth`/`maxHeight`）增加上限钳制（如 `maxBytes ≤ 2 MiB` 或配置化上限），超限按 `invalid_argument` 拒绝而非执行分配。

### P2-3 ViewImageHandler 的 root 在 executor 构建时固定，不随 per-turn/per-session workspace 更新（correctness）

**位置**：`governed_executor.rs:151-153`（`workspace = workspace_root.unwrap_or(current_dir)`）；`governed_executor.rs:164`（`ViewImageHandler::new(workspace.clone())`）；`runtime/mod.rs:566-572`（`apply_governed_turn_context` 每 turn 设 `DispatchContext.workspace_root`，handler root 不变）

**失败场景**：runtime 以 `workspace_root=None` 构建（默认 cwd），后续按 session/turn 提供真实 workspace 时，`view_image` 仍用构建期 cwd 作为路径约束根——读到的不是目标 session 的 workspace 文件（错位或误拒）。dispatcher 的 workspace facts 与 handler 的 root 分离。

**建议处置**：让 handler 的根跟随 dispatch-context 的 `workspace_root`（每次调用取 context，或把根解析下沉到 dispatcher 的 workspace 事实），与 `view_workspace_image` 的根保持单一真相源。

### P2-4 include_bytes=true 序列化为 JSON 数字数组，体积膨胀且注释与行为不符（UX / 正确性）

**位置**：`image_artifact.rs:57`（`bytes: Option<Vec<u8>>`）；`dispatcher.rs:1548-1553`（`render_output` 用 `to_string_pretty`）；`governed_executor.rs:664`（注释声称「bytes are base64-encoded in the serialization」）

**失败场景**：serde_json 把 `Vec<u8>` 序列化为 JSON 整数数组（每字节 ~2–5 字符），2 MiB 载荷 → ~10 MB+ pretty JSON；`governed_executor.rs:664` 注释（base64）与实际不符。默认 `include_bytes=false` 规避了常规路径，但模型显式 `includeBytes=true` 时输出体积远超 `result_budget_bytes=2 MiB` 语义（且 governed 路径预算关闭，无人截断）。

**建议处置**：在 handler 内显式 base64 编码 `bytes` 再序列化（与注释/预算语义一致），或收紧 `maxBytes` 默认上限并文档化数字数组行为；同步修正测试注释。

---

## P3 与测试覆盖缺口

1. **测试缺口（建议补齐）**：
   - `image_artifact`：无符号链接逃逸测试（canonicalize 处理正确但未测）；无设备路径/UNC 路径测试；无 `maxBytes` 巨大值/超上限测试；`ViewImageHandler` 的 `maxWidth/maxHeight/maxBytes` 为 0 或负语义未测。
   - `mcp_resources`：`max_content_bytes` 的单条目例外分支（`contents.is_empty()` push，`mcp_resources.rs:400-408`）未测；`truncate_string` 长文本截断未测；`set_registry` 源替换流未测（仅 `source_revision_mismatch` 单测）；分页 `nextCursor` 未支持也未测（`mcp_resources.rs:262-284`，超出 `max_items` 仅置 `truncated`）；无超时测试（`McpTransport::execute` 为同步调用，无 deadline，`mcp_resources.rs:200`）。
   - `tool_search_elevation`：elevator 自身 registry snapshot 与传入 snapshot 分离（`elevate` 用 `self.search` 于自身 snapshot、descriptor 于传入 snapshot，`tool_search_elevation.rs:116-143`）无测试；提升后 `provider_contract_views` 的 turn volatile segment 顺序未验证。
   - `governed_executor`：无测试断言 `mcp_resource_read`/`tool_search` 未注册 governed handler（对应 P2-1）。
2. **`McpResourceSurface::parse_content_item` 要求 content uri 与请求 uri 精确相等**（`mcp_resources.rs:432-436`）：对返回规范化 uri 的合法 MCP server 可能误拒。fail-closed 可接受，但建议放宽为「模板/规范化等价」并记录。
3. **MCP 分页未支持**：list-resources/templates 超过 `max_items` 仅置 `truncated`，无法取下一页。bounded 语义可接受，但需在文档/契约中明确。
4. **`view_image` 不在 batch composite 只读子授权内**（`governed_executor.rs:43-52` `READ_ONLY_PRIMITIVES` 无 `view_image`）：`workspace_batch` 子调用不能调用 `view_image`。属既有门禁的保守边界，可接受，建议文档化。

## 测试证据（本次审核静态核对）

- 新增测试覆盖确认：`governed_executor` 新增 5 项（plan create/replace/merge/complete-step + stale/missing-op fail-closed + view_image 默认引用/含字节上限/缺 path 与未知格式 fail-closed，`governed_executor.rs:447-696`）；`image_artifact` 新增 include_bytes 默认、ViewImageHandler 5 项（`image_artifact.rs:700-800`）；`mcp_resources` 12 项覆盖 list/templates/read、截断证据、item 上限、未知 source、revision 不匹配、builtin 非资源源、参数回显拒绝、畸形响应、content uri 不匹配、transport 错误传播；`tool_search_elevation` 8 项覆盖仅 Deferred 可搜、查询排名、提升 + evidence、非 Deferred 拒绝、revision 过期、descriptor 被删、view snapshot 不匹配、多提升 trace。
- 缺口：见「P3 与测试覆盖缺口」第 1 条；`contract_view` 断言已更新（`tools.rs:3872-3911`、`provider/mod.rs:3860-3873`）。

## 结论

阶段 7（7.1–7.4 + P2-6）判定为 **CONDITIONAL PASS**。

- 无 P0。
- P1-1（phase-7 MCP/ToolSearch 安全面未接入生产路径 + 生产 MCP resource read 回显 arguments 违反 Decision 11）与 P1-2（Plan `session_id` 信任边界）需在收口前处理；其中 P1-1 同时是验收 #8「可用」未满足项，P1-2 是「单一共享 dispatcher」里程碑的前置条件。
- P2-1..P2-4 已记录处置建议；P2-1 建议随 P1-1 的接线工作一并显式排除 MCP/ToolSearch 的 governed 注册。
- 两条不变量、plan_control/view_image 的统一注册与生产可达性、include_bytes 默认引用语义、view_image 路径防逃逸、ToolSearch revision 失效均经代码与测试确认。
