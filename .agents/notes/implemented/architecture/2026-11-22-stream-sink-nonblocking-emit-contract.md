# Agent Note: 流式读循环不引入独立发送线程——sink 侧非阻塞投递契约收口（B136）

Status: implemented

## Problem

B136 登记的是「慢 sink 会串进 SSE 读循环：读线程与 sink 投递未解耦（独立发送线程/队列）」。
登记时的语义假设是：`on_delta` 里内联调用 `TurnEventSink::emit`，一旦 sink 变慢（有界 channel 满、
阻塞写），读线程会停止读 socket，触发 TCP 背压，token 率被硬顶。

`crates/pony-agent-core/src/agent/runtime/stream_support.rs`（939-944 行）已有一条 F3 修订 v2 的裁决：
`TurnEventSink::emit` 必须是**非阻塞投递**；`emit` 仍内联在读循环里，但若将来出现阻塞式 sink，
**必须在 sink 实现侧解决**，不得在聚合器里加等待。也就是说：读循环该不该改，取决于现状是否真的存在
阻塞式 sink —— 这是本次要核实的事实，而不是可以继续按假设排期的施工项。

现状核查（逐 sink）：

| sink | 投递方式 | 是否可能阻塞读线程 |
|---|---|---|
| `TauriTurnEventSink`（`src-tauri/src/tauri_adapter.rs:75-79`） | `app.emit(name, payload)`，失败仅打印 | 否（同步序列化 + fire-and-forget 投递，不等待前端消费） |
| `TuiTurnSink`（`crates/pony-agent-core/src/bin/tui.rs:3556, 3726+`） | `std::sync::mpsc::Sender<UiEvent>::send` | 否（`std::sync::mpsc::channel` 为**无界**通道，send 不阻塞） |
| `CliTurnSink` / `TerminalSink`（`crates/pony-agent-core/src/bin/cli.rs:382, 533`） | 序列化后写 stdout（text/jsonl/sse） | 否（stdout 写是交付形态本身；丢弃会破坏 SSE/JSONL 协议，不属于「慢消费者」问题） |
| `AcpTextSink`（`crates/pony-agent-core/src/bin/cli/acp.rs:538`） | 写到 ACP 输出通道 | 否（同上，协议输出不可丢） |
| `RecordingSink`（`crates/pony-agent-core/src/bin/non_tauri_harness.rs:66`） | 内存收集 | 否 |

结论：当前**不存在**阻塞式 sink，F3 契约已被满足；B136 描述的故障模式尚未成立。

## Decision

1. **不引入独立发送线程或队列**：读循环与聚合器保持现状（`CoalescedBatch` 非阻塞时间窗聚合，
   `stream_support.rs` 已有的 I2/I3 判据不变）。
2. **维持并显式化 F3 契约**：`emit` 必须非阻塞；新增 sink 时其实现侧负责降级策略
   （有界 channel 用 `try_send` + 丢弃/合并，或改用无界通道），并补一条「慢 sink 不拖慢读线程」
   的回归测试。
3. **触发条件立法**：仅当出现以下任一情形才重开本决策 —— (a) 新增有界 channel 的 sink；
   (b) 实测到 `emit` 在读线程内的耗时进入 p99 热点；(c) 前端消费能力成为已测得的瓶颈。
   触发后的处置点在 **sink 实现侧**，不是读循环。
4. **账本口径**：B136 由 `todo` 转 `annotate`（证据收口，不产生代码变更），
   保留标题中的原始现象描述作为历史记录。

## Alternatives considered

- **独立发送线程 + 有界队列（B136 原始提议）**：能彻底解耦读线程与消费侧，但引入事件乱序与
  背压丢弃风险 —— SSE/ACP/JSONL 的协议语义要求按序、不丢终态帧（`stream_support.rs` 的 I2 判据），
  而当前没有阻塞 sink，属于为不存在的故障付架构代价。
- **读循环内 `try_send` 满即丢弃**：改动最小，但会和 I2「禁止丢最终帧」直接冲突，
  且把丢弃策略藏进聚合器，违反 F3 的「问题在 sink 侧解决」分工。
- **无界队列缓冲 + 后台线程**：消除排队阻塞，但把背压变成内存增长，在长流式回合下风险更高，
  且当前无实测数据支持需要缓冲。
- **直接关闭 B136 不做记录**：会让后续维护者重新捡起同一假设并再评估一次；落一条
  带触发条件的决策记录成本更低。
