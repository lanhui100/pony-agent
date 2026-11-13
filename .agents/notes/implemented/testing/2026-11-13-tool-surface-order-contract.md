# Agent Note: 模型工具面顺序契约化（tools 数组 = 缓存前缀的一部分）

Status: implemented

## Problem

provider 侧的前缀缓存按**请求体前缀**匹配，`tools` 数组在这份前缀里。工具面的顺序此前没有任何契约：

- `surface_tool_definitions()`（`mcp_config.rs`）返回「内建面 + 外部通道追加」，内建顺序来自 `builtin_tools()` 的声明顺序；
- 仓库里没有断言保护这个顺序——`wave37_tool_surface` 只查工具**是否存在**，不查是否同序；
- 外部工具（MCP/插件）若被插入内建序列中间，或某次重构调换了两个 `ToolDefinition` 的声明位置，请求仍会成功，只是每轮都按冷缓存计费。

这是「静默成本恶化」：没有错误日志、没有测试变红，只有账单和延迟变差。对照 DSH：其 `SystemPrompt.tools()` 把工具顺序做成**中央分配的显式契约**（`Config.toolOrder` + `<unlisted-tools>` 占位符 + 未知名加载期失败），并另设 `deferLoading` 让工具增删通过消息激活、不动主表。

## Decision

新增黑盒契约 `crates/pony-agent-core/tests/tool_surface_order_contract.rs`（TS1–TS3）：

- **TS1**：内建工具面顺序**逐项冻结**（45 个 primitive 名的完整序列）。新增/删除/调序必须显式改本表并说明理由——因为那就是一次缓存前缀变更。
- **TS2**：同一进程内两次取面逐项一致（进程级缓存幂等），且无重复名（重复声明会放大请求体并让工具选择歧义）。
- **TS3**：外部工具必须**追加在内建面之后**（`names[..builtin.len()] == builtin`）。这一条守住「连一次 MCP 不得重排整个 tools 数组」的边界。

**不做的部分（有意收窄）**：原计划同轮加 `PrefixMutationReason::ToolSurfaceDynamic`（工具面含动态来源时给出缓存突变原因）。该意图仍成立，但落地必须改 `provider/mod.rs` 的枚举——而该文件当前已被并行 lane 推过基线（9912 行 vs 基线 9768，见 backlog B123 文件规模债）。再加行会把既有违规进一步恶化，且门禁的棘轮规则不允许抬基线。故权益衡后：本轮只落**零风险、零行数增长**的顺序契约；突变原因登记为独立待办，等 provider 拆分完成后再做。

## Alternatives considered

- **同时实现 `ToolSurfaceDynamic` 原因（原计划 6a 的後半）**：见上——被 `provider/mod.rs` 的既有规模债阻塞；硬塞会恶化已登记债务，也违反「最小闭包」。
- **按 DSH 做 `toolOrder` 配置 + `<unlisted-tools>` 占位**：表达力更强（可由部署决定顺序），但本仓工具面是编译期常量、没有部署级装配概念，引入配置只会多一个「顺序可被外部改坏」的入口；先锚定现状，等真有装配需求再升级。
- **只做计数断言（工具总数不变）**：抓不到「同数量但调换顺序」，而调序正是最容易发生的漂移（重构两个 struct 的位置），必须逐项比对。
- **把顺序断言塞进 `wave37_tool_surface`**：那是别名/可见性契约文件，主题不同；新建文件让「顺序」这条不变式有独立真源与独立失败信息。
- **用 FNV 指纹代替逐项冻结**：指纹失败只给一个数字，无法定位是哪两个工具换了位；逐项比对直接报出下标与名字，排障成本更低。

## Consequences

- 工具面顺序现在有可执行契约：任何调序/插入都会在 CI 变红并指出具体位置，而不是变成「缓存命中率悄悄下降」。
- 成本：新增工具时必须同提交更新冻结表（预期摩擦，与工具面其它契约一致）。
- 残余 1：`PrefixMutationReason::ToolSurfaceDynamic` 未落地——外部工具在会话中途连接/断开时，trace 仍无法归因「本轮前缀因工具面变化而失效」。已登记 backlog，依赖 provider/mod.rs 拆分。
- 残余 2：provider 真正下发给 API 的顺序取决于 `provider_contract_views()`（registry 序），与本测试覆盖的 `surface_tool_definitions()` 顺序同源但非同一函数；若将来两者分叉，需要补一条「wire 顺序 == surface 顺序」的断言。
