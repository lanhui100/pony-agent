# Agent Note: 工具面动态来源的缓存突变原因（ToolSurfaceDynamic）

Status: implemented

## Problem

`PrefixMutationReason` 此前有八种原因（会话摘要/目标/长期记忆/技能清单/图片提示/截断提示/历史边界/原生 transcript 边界），**唯独没有工具维度**。

但 `tools` 数组属于 provider 前缀缓存的一部分：内建面是编译期常量，唯一跨 turn 可变来源是外部 MCP/插件通道——它在会话中途连接、重载或断开时改变 `tools` 数组，使整个前缀缓存失效。此时 trace 上只会看到「这轮缓存没命中」，无法归因到工具面变化；而工具面变化恰恰是**外部行为触发**的（用户接了一个 MCP），最需要可归因。

`testing/2026-11-13-tool-surface-order-contract.md`（6a）已把「顺序漂移」堵住，本条补的是**「来源动态」的归因**，即 B144。

## Decision

1. **新增 `PrefixMutationReason::ToolSurfaceDynamic`**（wire 串 `tool_surface_dynamic`），语义：本轮工具面含跨 turn 可变来源，tools 数组可能与本轮前缀的其它部分不同源。
2. **判定下沉到工具面模块**：`agent::mcp_config::tool_surface_is_dynamic()` = 「已安装 MCP/插件通道且该通道确有工具」。不用「与前一轮比较哈希」的判定，理由见备选。
3. **发射点在 `turn_prep::prepare_turn`**（工具面解析之后、`build_context_observation` 之前），通过 `ProviderRequestObservation::note_dynamic_tool_surface(bool)` 记账——**幂等**（同一轮只留一条），且**只改观测、不改请求内容**。
4. **前端契约同步**：`src/types/runtime.ts` 的 `PrefixMutationReason` 联合加 `"tool_surface_dynamic"`；`npm run contract:check` 通过（该门禁只查「Rust 有、TS 无」）。
5. **测试**：`provider::tests::tool_surface_dynamic_reason_wire_string_and_idempotent_note` —— 锁 wire 串（与 TS 联合成员同名）、锁「静态面不产生原因」、锁「重复记账幂等」。

## Alternatives considered

- **按「与前一轮的工具面哈希比较」判定**：语义最准（只有真正变化才记），但需要新增一份**每会话上一轮工具面哈希**的进程级注册表（会话数 × 生命周期管理 + LRU + 并发），而它要解决的问题仅是「给缓存失效加一个归因标签」。当前判定（外部通道存在即可能变化）零状态、零并发风险，且**不会漏报**（外部工具在时工具面本就可能变），代价只是**可能多报**（外部通道存在但本轮没变也记一条）。
- **把原因写进 `BuildContextObservation` 而不是 `ProviderRequestObservation`**：两个结构都进 trace，但 `PrefixMutationReason` 的既有家就在 `ProviderRequestObservation`（另外八种原因都在这里），保持一致优于分散。
- **同时新增 `tool_surface_fingerprint: u64` 观测字段**：能让「是否真的变了」在 trace 上可算，但要动 serde 契约快照与前端类型、且与本条目标（归因标签）不是同一层次；留作后续增强（真实需要按轮 diff 时再加）。
- **完全不做**：`tools` 变化导致的缓存失效会继续不可归因——而它正是成本最高、最不容易被怀疑的一类（用户以为只是"接了个 MCP"）。

## Consequences

- trace 上首次可以区分「工具面含动态来源」与其它八种前缀突变；MCP 接入/断开后的缓存表现异常可被直接归因。
- 零状态、零请求内容变化：lib 全量 1535 passed；`contract:check` 通过（无 Rust/TS 字段漂移）。
- 已知过报：外部通道安装但该轮工具面实际未变时仍会记一条。这是「零状态」换来的代价，已在备选里写明；若将来需要精确 diff，再补每会话哈希。
- 残余：判定不覆盖「内建面因版本升级而变化」的情形（那属于编译期常量变化，会由 `tool_surface_order_contract` 的 TS1 冻结表在 CI 拦截，无需运行时归因）。
