# 0063 TUI Ask 覆盖层与取消反馈的用户可见边界

Status: implemented

## 背景

TUI 的 Ask 面板虽位于正文底部，但布局先从消息区扣除面板高度，随后通用浮层逻辑又清空整个消息区。结果是问题出现时历史上下文消失，与「提问时保留决策上下文」的交互目的冲突。取消链路则已经区分停止请求、终态事件与持久化哨兵，需要避免为修复呈现而引入第二条取消协议。

现状证据：`crates/pony-agent-core/src/bin/tui.rs:4822-4835`（Ask 占位）、`:4992-5016`（整区清理与 Ask 绘制）、`:6886-6905`（恢复视图优先反显结构化参数）、`:1724-1757`（停止终态投影）、`:3091-3096`（取消事件适配）、`:8230-8283`（轮次线程未注册 sink）。`HostControlPlane` 已提供成对注册/注销并同时绑定 sink 与监督 emitter（`crates/pony-agent-core/src/agent/control_plane/mod.rs:2037-2080`）。

## 候选方案

### 方案 A：保留内联占位，仅取消整区清屏

历史重新可见，但 Ask 出现/消失仍改变消息区高度与滚动窗口，造成正文跳动；它只是修复清屏，不满足稳定的覆盖层交互，故不采纳。

### 方案 B：Ask 覆盖绘制，取消复用既有事件投影（采纳）

Ask 不再参与消息区高度计算，只在消息区下部覆盖绘制且下缘贴 Composer 上缘；面板与消息区允许相交，未被面板遮挡的历史保持原布局。工具摘要采用 `description` 非空优先，交互 Ask 不把结构化 arguments/result 反显为摘要，普通工具维持兼容。取消继续走 `StopSequenceState → stop_turn → turn:cancelled → apply_stop_terminal`，但 TUI 必须为每轮注册既有 `TuiTurnSink` 才能收到该链事件，并在调用结束后成对注销 sink/emitter；用户原因呈现既有「已停止（用户中断）」，不把协议 payload 变成聊天消息。改动闭包最小，事件与 CAS 公共契约不变。

### 方案 C：新增取消消息/事件并统一清洗所有内部 payload

能提供另一套呈现通道，但会扩大事件、持久化与消费者兼容面，并混淆实时状态与历史消息；本缺陷无需该架构变更，故不采纳。

## 决策

1. Ask 是覆盖层而非布局项：显示时历史消息区仍按其原有几何渲染，面板仅遮住其下部有限行；不得整区清除历史，也不得遮挡 header、Composer 和指标行。
2. 保留既有宽终端≤12、窄终端≤8 行的面板边界及极小终端降级；不承诺在极小终端同时展示全部历史与全部选项。
3. 流式双击 Esc 仍只在第二次命中时发一次停止请求；「正在停止…」只在受理后出现，终态只由取消/完成/失败事件驱动。
4. 每个初始 turn 与 goal 续轮复用现有 `register_turn_event_sink` 注册 `Arc<TuiTurnSink>`，该次 `start_turn_stream` 返回后调用 `unregister_turn_event_sink`；sink 与监督 emitter 成对回收。不得增加事件、线程或 core 接口。
5. 用户主动停止的终态沿用「已停止（用户中断）」状态提示，不新增取消聊天消息；已有正文与持久化历史保持原义。原始 `text`/`error`、工具参数、协议 JSON 与 Rust Debug 不作为取消提示或用户消息显示。
6. 恢复视图的工具摘要以非空 `description` 优先；若该字段只是工具名占位，则从结构化参数中只提取已知的人类可读 `description` 字段，绝不反显整段 JSON。交互 Ask 的结构化 arguments/result 不作为消息摘要，复用现有交互工具名称分类，不增加通用 sanitizer。普通工具无 description 时维持既有 arguments→result 兼容回退。
7. Ask 自身 Esc 成功取消仍调用既有 `cancel_ask(request_id, version)` 并关闭面板，不构造答案、不新增系统消息、不调用 `stop_turn`；CAS 失败不得伪装成功。Ask 已 suspended，其 Esc 与普通流式双击停止保持不同产品语义。
8. 不新增接口空桩、事件字段、持久化格式、配置、依赖、网络调用或锁；新增资源义务仅为上述既有 sink/emitter 的成对回收；可观测性四维归属不变。
9. TUI 启动后把既有结构化 JSON 日志的缺省输出从 stdout 改道已重定向的 stderr（`otel::log::set_stderr_output(true)`）；全屏界面不得被内部协议 JSON 覆盖。日志格式、字段与落盘取证面不变，仅修正错误输出目标。

## 影响

- 实现范围限 `crates/pony-agent-core/src/bin/tui.rs` 的 Ask 布局、工具摘要呈现及既有 sink 注册/注销；`docs/observability.yaml` 仅追加本次无归属变化的说明。
- 验收以历史与问题同帧、切换 Ask 不改变消息几何、J1 定向抑制结构化摘要、取消状态无内部 payload、重复/陈旧事件幂等为核心；契约见 `.dev-team/contract-matrix-tui-overlay-cancel.md`。取消终态另须当前构建的真实 PTY 证据，不能仅凭旧日志推定 sink 已生效。
- 本 ADR 不取代 ADR 0042/0043 的 Composer 与消息层级决定，仅修正 Ask 覆盖方式和取消呈现边界；wave-60 的停止生命周期与持久化哨兵决定继续有效。
