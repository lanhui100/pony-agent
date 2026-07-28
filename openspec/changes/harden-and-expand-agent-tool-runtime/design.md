## Context

Pony Agent 当前把 17 个 builtin 定义集中在 `tools.rs`，再通过名称映射派生 13 个模型可见工具。`ToolDefinition` 本体只有名称、描述和输入 schema；kind、exposure、permission、canonical identity 分散在函数表中，provider 还通过工具数组长度猜测是否为 builtin。runtime 顶层调用会经过 capability mediation，但 `workspace_batch` 和 `workspace_gather_context` 的子调用直接进入 `ToolRouter::execute_internal`，没有逐步经过相同的权限、hook 和 telemetry 管线。

现有 graph 已具备普通 turn 收口后的 `wait_user`，但它并不保存 mid-turn tool call、请求身份或回答状态；真实 `Ask` 不能直接复用该状态。Tauri 是首个 host adapter，但 core 仍必须保持可被 CLI、HTTP-SSE 或测试 harness 复用。

## Goals / Non-Goals

**Goals:**

- 建立单一工具描述符与注册表真相源。
- 让顶层调用和 composite/skill 子调用共享同一 dispatcher 治理链。
- 将权限声明、权限决策、宿主中介和执行结果分层。
- 把 `Plan`、`Ask` 修正为控制工具，而不是执行原语别名。
- 为 process、Web、Search/Glob 建立可测试的安全、sandbox/连接绑定和资源预算。
- 补齐依赖新内核的 `view_image`、MCP resource surface 和 deferred tool elevation。
- 保持产品工具名与历史 trace 的兼容读取。

**Non-Goals:**

- 本 change 不实现 LSP、任务/Goal、worktree、子代理、workflow 或 marketplace。
- 不在 core 内引入 Tauri 类型或桌面 UI 回调。
- 不以 command denylist、process group 或 Job Object 代替 sandbox/approval；它们只可作为纵深防御或 containment。
- 不把 MCP resource 的协议名称或 capability 元数据视为真实远端读取、可信内容或无副作用保证。
- 不一次删除所有旧 `workspace_*` 调用；迁移期保留有界内部别名。

## Decisions

### 1. 用单一 `ToolDescriptor` 取代名称派生表

每个注册项包含：

- `ToolIdentity { model_name, canonical_name, primitive_name, source }`
- input/output schema
- `ToolKind` 与 `ToolExposure`
- definition-time `ToolPermissionDeclaration`
- `ToolExecutionPolicy { concurrency, cancellation, timeout, result_budget }`
- display/search metadata
- handler/runtime 引用

provider、capability registry、planner、trace 和前端 contract view 都从同一 registry snapshot 投影。禁止再按 `tools.len()` 或名称 switch 猜测 builtin 身份。

替代方案是继续为 `ToolDefinition` 增加独立映射函数；它改动小，但无法阻止新增工具继续漏改某张表，故不采用。

### 2. Dispatcher 是唯一执行入口，且以最终调用事实授权

新增 `ToolDispatcher`，处理以下固定顺序：

1. 按不可伪造 `InvocationOrigin` 解析 identity/alias 并检查 exposure；model origin 仅能调用当前 turn provider view 中的 direct 或已提升 descriptor，internal descriptor 仅能由有 parent lineage 和 allowed-child 集合的 `ChildDispatch` 调用。
2. 对原始输入做 schema 和工具级校验。
3. 执行可改写的 pre-dispatch hook；hook 不得改写 descriptor identity 或调用来源。
4. 对 hook 产生的最终输入重新做 schema、规范化、工具级校验和权限/sandbox 决策。
5. 把 `descriptor_snapshot_id + final_args_digest + policy_digest + workspace/session/run/call` 绑定到许可、审批或 pending control request；此后不得再改写安全相关输入。
6. 原子预留 cancellation、deadline、并发、calls、bytes 和 output budget。
7. handler 执行，返回领域结果；handler 不能伪造 permission evidence、control outcome 或 telemetry。
8. 由 dispatcher 归一化结果、执行 post hook、结算预算并产生 telemetry；每一个 top-level 与 child invocation 各有一套 lifecycle record，runtime 不得重复发射同一层的 hook。

handler 只实现领域行为。组合 handler 获得受限 `ChildDispatch` 接口，不能持有 `ToolRouter` 或调用其他 handler 私有方法。child context 携带 `parent_call_id`、depth、共享原子预算账本、剩余 deadline 和 cancellation token；默认最大深度 4，禁止环路和自递归。副作用 child 在并发启动前必须完成解析、最终参数鉴权和预算预留；任何 child 进入 pending approval/interaction 时，不再启动未开始的 sibling。

替代方案是在 `workspace_batch` 内手工补 hooks/permission；这会复制 runtime 管线并继续遗漏外部 capability，因此不采用。

### 3. 迁移期先 fail closed

在 dispatcher 完整接管 composite 前，`workspace_batch` 只允许明确的 read-only primitive；`Run/Write/Edit/Web/MCP` 子调用返回结构化 `unsupported_composite_child`。这是临时安全门，不是最终组合模型。

### 4. 权限声明与决策分离

descriptor 只声明 typed scope set、风险类别、是否需要 host mediation 和审批策略。未知 scope fail closed，child scope 只能单调上收。每次 invocation 由 `ToolPolicyEvaluator` 基于最终规范化参数生成 `allow / deny / approval_required / waiting_host` 决策，并记录 `decision_source`。

composite 在计划阶段计算 scope 并集和最强审批要求；执行阶段仍对每个 child 重新决策，不能用父级摘要替代子步骤鉴权。builtin capability 注册直接使用 descriptor 权限声明，不再统一写成 `workspace` 和 `requires_approval=false`。

### 5. 控制请求与执行结果正交，`Ask` 使用持久化暂停

`ToolOutcome` 分为有限 `execution_status`（例如 ok/error/cancelled）与正交 `control_outcome`。`waiting_user`、`approval_required` 与 `waiting_host` 是 control outcome，不是 `ToolResult.status` 或 provider tool result。dispatcher 遇到 control outcome 时，runtime 在任何 provider follow-up 或 child 调度前原子持久化并暂停 run。

`Ask` 与 approval 共用 `PendingControlRequest`，但以 `request_kind` 区分。它包含 request id、session/run/turn/call id、descriptor snapshot id、final args digest、policy digest、问题/选项或审批事实、expiry、one-time nonce 与状态版本。answer/approve/cancel 均使用 compare-and-swap 一次性消费；重复、过期、跨 session、descriptor/source 变更或参数摘要不匹配全部 fail closed。恢复后，runtime 仅以原 `tool_call_id` 注入一个唯一终态 tool result；Ask 回答绝不等同 approval。

无交互 host 必须生成 `interaction_unavailable` 的终态执行失败，不得伪造成功或静默丢弃。pending request 持久化到 session/graph/checkpoint，明确保存原始 assistant tool-call transcript，避免 reload 清除未闭合 roundtrip。该方案不在 core 内持有 Tauri channel。

### 6. `Plan` 是状态控制，不执行任意 calls

`Plan` 采用 `create / replace / merge / complete_step` 操作更新 session-owned 的结构化 plan；plan 与 step 都有稳定 id 和 revision，mutation 带 expected revision 并在冲突、重复 complete 或非法转换时显式失败。计划驱动执行由 planner/graph 在后续决策中显式选择工具，`Plan` handler 本身不接受通用 `calls` 数组。

旧本地多路径聚合改为内部 `workspace_batch` 或新的 `GatherContext` composite，不再借用 `Plan` 名称。

### 7. Process 使用 session-scoped manager 与显式 sandbox 能力

将 `Run` 拆为产品入口和内部 process primitives：`process_start`、`process_poll`、`process_write_stdin`、`process_kill`。`ProcessManager` 由 runtime/session 持有，生成高熵、绑定 session/run/owner 的 opaque process handle，持续并发排空 stdout/stderr 并使用固定 ring buffer。poll/stdin/kill 对 handle owner 做鉴权，重启后旧 handle 失效。

`SandboxBackend` 独立于 `ProcessBackend`：前者负责 workspace 文件、网络、环境与句柄继承约束，后者负责生命周期与 containment。无人值守 `Run` 只在真实 sandbox 可用时启用；无 sandbox backend 的平台 fail closed。明确的 unsandboxed mode 只能逐次由 host 审批，并在结果和 trace 标为高风险，不能静默降级。Windows Job Object 必须禁止 breakaway；Unix process group 仅声明为 best-effort containment，不能声称能阻止 setsid/double-fork。输出返回截断标记和丢弃字节数；子进程使用最小环境，不能继承 provider key、session secret 或 ambient proxy。`Run` 可在短命令场景组合 start+wait，但仍走 child dispatcher。

### 8. Web 安全策略先于请求

使用标准 URL parser，仅允许 `http/https`。`WebAccessPolicy` 在初始 URL 和每次 redirect 上验证：禁止 credentials、localhost、回环、私网、链路本地、unspecified、multicast、reserved、IPv4-mapped IPv6 和受限端口；所有 A/AAAA 都必须通过策略。默认关闭 reqwest 自动重定向和 ambient proxy，由工具显式执行最多 5 跳。

任意 URL `WebFetch` 只有在 injected resolver/pinned connector 能把连接固定到已验证地址、保留原 authority/SNI 并校验实际 peer IP 时才可启用；每一 redirect 均重新解析、校验和 pin。该能力完成前产品入口 fail closed，不保留“预解析后交给默认 HTTP client”的弱化模式。受信代理必须显式注入、拥有等价目标校验责任且留下决策证据；否则禁止。正文采用 streaming 读取，默认 2 MiB 解压后 hard limit，并同时限制压缩前字节、压缩比、headers、redirect、总 deadline 和慢速响应；超限、二进制和不支持类型返回结构化错误/元数据。

### 9. Search/Glob 采用成熟语义

使用 `ignore` 遍历并尊重 `.gitignore`，`regex` 处理正则，`globset` 处理路径模式。结果确定性排序；扫描文件数、字节数、命中数和时间都有预算。任何预算截断必须返回 `truncated=true` 和原因，不能以完整成功伪装。

### 10. Deferred 工具提升只作用于当前 turn

ToolSearch 在 registry 中搜索 `Deferred` descriptor，并以 `select` 参数返回稳定候选或选择 descriptor reference。选择只接受当前 registry snapshot 中、权限允许且版本匹配的候选。runtime 把完整 schema 加入同一 user turn 的下一 provider hop `TurnToolView`，并在 trace 记录 elevation 与 prefix mutation reason；provider follow-up 必须实际读取更新后的 view。下一 user turn 默认重新计算；未提升工具可被 host/child 按 origin 授权调用，但模型调用必须拒绝。

### 11. 首批新增能力保持 host-agnostic

- `view_image` 读取 workspace 内图片并返回规范化 image artifact；是否可发送给模型由 provider modality 决定。
- MCP resource 拆为 list resources、list resource templates、read resource 三个只读控制入口，并通过 `McpTransport` port 真实调用绑定 source 的 transport；capability snapshot 仅是发现元数据，不得把 arguments 回显为资源内容。`ResourceTemplate` 与 MCP prompt template 是独立类型。transport response 按不可信内容处理，受 byte/item/MIME/depth、provenance 和脱敏规则约束。
- Tauri 只负责图片编码/展示和用户交互适配，不持有 core 工具真相源。

## Risks / Trade-offs

- [迁移期间新旧执行路径并存] -> 用 registry feature flag 和双读对照 telemetry，按工具逐个切换；同一 invocation 只能选择一条执行路径。
- [真实 Ask 改变 turn 生命周期] -> 先补 `PendingControlRequest`、原 tool-call transcript、CAS/reload/无交互 host 测试，再接 UI；不使用普通 completed-turn wait_user 作为替代。
- [进程树管理存在平台差异] -> 抽象 `SandboxBackend` 与 `ProcessBackend`；未实现真实 sandbox 的平台禁用无人值守 Run，process group 只作为 best-effort containment。
- [SSRF 校验与代理/DNS 行为复杂] -> 在 pinned connector 与 peer-IP 校验完成前禁用任意 URL；策略不得以默认 HTTP client 的 DNS 或 ambient proxy 静默放宽。
- [新 descriptor 扩大一次性改动] -> 先以适配器包装现有 handler，保持工具输出不变，再删除旧映射表。
- [Search 依赖增加编译体积] -> 优先使用 Rust 生态成熟小库；若基准显示明显回退，再评估 `rg` backend，但结构化合同保持不变。
- [动态 tool elevation 影响 prompt cache] -> builtin stable prefix 不变，elevated schemas 进入 turn volatile tool segment，并记录 prefix mutation reason。

## Migration Plan

1. 补 characterization tests，只冻结产品名称、无争议 schema/输出和兼容读取；把 Ask/Plan 旧映射写成迁移删除测试。
2. 定义 `ToolOutcome`、`PendingControlRequest`、`TurnToolView`、`SandboxBackend` 与 `McpTransport` 契约及其 fake harness。
3. 引入 descriptor/registry snapshot，以适配器包装现有 `ToolRouter`，provider 改读 registry；旧映射表保留只读对照。
4. 引入 dispatcher 和 child-dispatch；先限制旧 batch 为 read-only，再迁移 GatherContext/skills/composite。
5. 切换 builtin capability 权限来源，完成最终参数鉴权、子步骤权限、hooks 与 telemetry 证据。
6. 实现真实 Plan/Ask、TurnToolView elevation 和 MCP transport，再完成 graph wait/resume 与 host UI。
7. 先落 sandbox support matrix 与 process backend，再迁移 Run；先落 pinned connector，再启用任意 URL WebFetch；Search/Glob 可独立加固。
8. 实现 view_image，完成全量验证后删除长度猜测、冲突映射和旧执行旁路；同步 canonical specs、任务系统与架构文档。

回滚按阶段 feature flag 切回前一 registry snapshot；涉及安全修复的旧 composite 写/执行旁路不得重新启用。持久化 schema 如有新增必须保持向后兼容，未知字段由旧版本忽略。

## Verification Strategy

- descriptor/registry property tests：identity 唯一、alias 无环、exposure 与 provider view 一致。
- dispatcher matrix：builtin/MCP/skill/composite、model/child/host origin、allow/deny/approval/waiting、hook rewrite 后重新鉴权、cancel/timeout/atomic budget。
- graph integration：Ask/approval persist、reload、answer/approve/cancel/expire、CAS replay rejection、原 tool call id follow-up。
- process：sandbox support matrix、跨 workspace/网络/环境隔离、跨 session handle 拒绝、大输出无死锁、stdin、多轮 poll、timeout、session close、平台 containment canary。
- Web：hermetic resolver/connector 下的 literal/private DNS、多 A/AAAA、DNS rebinding、peer-IP、redirect-to-private、oversize/compression/binary/timeout/proxy policy。
- Search：真实 regex、glob、ignore、确定排序、预算截断。
- 前端：waiting/approval/running/truncated/elevated 状态渲染。
- Gate：format、clippy/check、定向/全量 Rust 测试、前端 unit/build、OpenSpec validate、独立安全与代码审核。

## Open Questions

- Windows ProcessBackend 最终采用 Job Object 直接实现，还是引入跨平台 process-group crate；在 process phase 开始前用 spike 和测试决定。
- `view_image` 的 artifact 是否直接承载 base64，或只保留受控文件引用并由 host/provider adapter 按需编码；优先选择后者以控制 trace 体积。
- 首版 host approval 是否复用 Ask 的 wait/resume envelope，还是保留独立 approval request 类型；二者应共享生命周期基础设施但保持语义类型分离。
