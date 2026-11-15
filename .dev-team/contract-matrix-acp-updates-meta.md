# ACP SessionUpdate 14 变体 + `_meta` 契约矩阵（backlog B166 / B180 / task-19）

> 状态：契约冻结。Owner：Protocol Agent。真源：`/tmp/acp-probe/schema-v1.json`（官方 v1，SessionUpdate 14 变体）；`docs/architecture/acp-client-integration.md` §3.2.7；Lead 2026-11-15 裁决。
> 写域：`crates/pony-agent-core/src/acp/`、`.dev-team/`。

## 1. 签名清单（path:line）

| 位置 | 内容 |
|---|---|
| `acp/protocol.rs:408` | `pub enum SessionUpdate`（官方 v1 **14** 变体，判别键 `sessionUpdate`） |
| `acp/protocol.rs:472 / 474 / 476 / 478 / 480 / 482` | B166 新增 6 变体：`ConfigOptionUpdate` / `SessionInfoUpdate` / `UsageUpdate` / `Notice` / `CompactionUpdate` / `CompactionSummaryChunk`（newtype 载荷，字段 camelCase） |
| `acp/protocol.rs:486` | `#[serde(other)] Unknown`：未知/厂商私有 `sessionUpdate` 降级为本变体（不再整条反序列化失败） |
| `acp/protocol.rs:786` | `ConfigOptionUpdate { config_options: Vec<SessionConfigOption>, _meta? }` |
| `acp/protocol.rs:795` | `SessionConfigOption`（`type` 判别）：`select{id,name,description?,category?,currentValue,options,_meta?}` / `boolean{…currentValue:bool}`；`SessionConfigSelectOptions`（untagged：Grouped/Ungrouped）+ `SessionConfigSelectGroup` + `SessionConfigSelectOption` |
| `acp/protocol.rs:858` | `SessionInfoUpdate { title?, updatedAt?, _meta? }`（部分更新；null 清除语义由 executor 三态解析补） |
| `acp/protocol.rs:870 / 885` | `UsageUpdate { used: u64, size: u64, cost?: Cost, _meta? }` / `Cost { amount: f64, currency: String, _meta? }`（opencode 实测发送） |
| `acp/protocol.rs:895 / 906` | `Notice { severity, title, description?, _meta? }` / `NoticeSeverity`（info/warning/error + `Unknown(String)` 原样保留） |
| `acp/protocol.rs:940 / 955 / 992` | `CompactionUpdate { compactionId, status, summary?, error?, _meta? }` / `CompactionStatus`（in_progress/completed/failed/cancelled + `Unknown`）/ `CompactionSummaryChunk { compactionId, content, _meta? }` |
| `acp/protocol.rs:100` | `ClientCapabilities._meta`（`#[serde(rename="_meta", default, skip_serializing_if="Option::is_none")] pub meta: Option<Value>`）：与 agent 协商私有能力（`opencode/child-session-updates` / `session.compaction` / `elicitation.form`），宿主只透传不解读 |
| `acp/client.rs:386` | **additive** `connect_with_capabilities_and_meta(spec,cwd,capabilities, client_meta: Option<Value>)`：把 `_meta` 写进 `initialize.clientCapabilities._meta`；`connect_with_capabilities`(:372) 等价 `None`；既有 4 个入口签名不变（`AcpClientCapabilities` 保持 `Copy`，不改结构体字面量契约） |
| `acp/session.rs:750` | `emit_update` 新增 6 + Unknown 共 7 个 arm；**usage→turn 映射位置 = `session.rs:853` 的 `SessionUpdate::UsageUpdate` arm**（当前结构化日志，不进 UI；保留 used/size/cost 供 telemetry） |

## 2. 断言面

| ID | 断言 |
|---|---|
| U1 | **14 变体 round-trip**：逐个用官方样例 JSON 反序列化再 `to_value`，判别键与主字段逐字段等价（8 个既有变体映射不变；6 个新增：configOptions/currentValue、title/updatedAt、used/size/cost{amount,currency}、severity/title/description、compactionId/status/summary/error、compactionId/content）。 |
| U2 | **未知变体兜底**：`{"sessionUpdate":"vendor_new_update","foo":1}` ⇒ `SessionUpdate::Unknown`（不 Err、不 panic）；`session/update` 其余帧仍继续消费；warning/结构化日志可见。 |
| U3 | **`_meta` 透传**：`connect_with_capabilities_and_meta(..., Some({"opencode/child-session-updates":true}))` 的 initialize 报文含 `clientCapabilities._meta`（值逐字节保留）；`None` 时**不出现** `_meta` 键（既有 4 入口报文与 B180 前一致）；`ClientCapabilities` 反序列化容忍任意 `_meta` 内容。 |
| U4 | **usage→turn 映射位置**：`UsageUpdate` 只走 `emit_update` 的 `UsageUpdate` arm（`:853`，结构化日志），**不**产生 `turn:delta` / `turn:tool`；`used`/`size`/`cost` 正确可见；`cost` 缺省合法（required 仅 used/size）。 |
| U5 | **诊断不吞**：Unknown / notice / compaction / config / session_info 各 arm 只记 stderr 诊断，不 panic、不中断 prompt 归约；非法载荷仍记 warning（沿用 B162 的 `UpdateForwarder`）。 |
| U6 | **无回归**：8 个既有变体→`turn:delta`/`turn:tool` 映射与 B166 前逐字段一致；stopReason 归约/取消语义不变；`AcpClientCapabilities` 的 `Copy` 与既有构造字面量不变。 |

## 3. 边界与已知限制

- **patch 语义**：`CompactionUpdate.summary` / `error` / `_meta` 的「省略=不变、null=清除、值=替换」三态未在当前类型区分（`Option` 折叠省略与 null）；executor 落地时按需引入三态解析，不改变本契约的主字段/判别键。
- **notice/compaction 前置私有能力**：agent 仅在客户端 `_meta` 声明对应能力时才发；Pony 当前只透传 `_meta`，是否声明由调用方（CLI/TUI）决定。
- **未知变体载荷**：`#[serde(other)]` 天然丢弃未知载荷；如需保留原始 JSON 供诊断，由 `UpdateForwarder` 层另行记录（当前只记"未知变体"）。
- **测试字面量**：`ClientCapabilities` 新增 `meta` 字段属 additive；若测试有显式构造需补 `meta: None`（Test Agent 写域）。

## 4. 门禁收据

- `cargo check -p pony-agent-core --lib` → **Exit 0**（0 warning）；`npm run cargo:check:shared` → **Exit 0**（`Finished dev profile ... in 41.24s`）。
- 未改 `acp/transport.rs`、`acp/config.rs`、`acp/handlers.rs` 与 `agent/*`。
