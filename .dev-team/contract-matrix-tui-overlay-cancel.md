# TUI Ask 覆盖层与取消反馈最小验收契约（已冻结 2026-11-05）

> Protocol Agent 提案；Test Agent 仅据此编写验收，Executor 只读。公共事件、工具、CAS 与持久化契约不变。

## 0. 事实与最小闭包

- Ask 当前从消息区扣除 `inline_ask_height`（`crates/pony-agent-core/src/bin/tui.rs:4822-4835`），且通用浮层分支清空整个 `chat_rect`（同文件 `:4992-5016`）；这是历史不可见的直接原因。
- Ask 的高度与绘制入口已存在（同文件 `:5531-5545`、`:5616-5617`），无需新组件、类型或函数签名。
- 恢复视图确实优先把 `arguments_text` 反显为工具详情（同文件 `:6886-6905`）；结构化载荷泄漏的修复仅针对交互 Ask。live `turn:tool` 已优先采用面向用户的 `description`（同文件 `:3003-3025`），该路径与通用正常工具文本不变。
- 停止链路已有 `turn:cancelled → StopTerminal → apply_stop_terminal`（同文件 `:3091-3096`、`:8386-8398`），用户原因投影为「已停止（用户中断）」（同文件 `:1724-1757`）；本波固定该兼容行为，不改写 core 的取消哨兵或持久化历史。现有日志只能证明事件曾产生，不能证明当前运行二进制可见；取消终态必须经当前构建的真实 PTY 复核。
- 双击判定、500ms 窗口、幂等与锁边界已存在（同文件 `:7877-7914`；`crates/pony-agent-core/src/agent/stop_sequence.rs:54-83`），本波不重新设计。

## 1. 非目标与公共契约不变量

- 不改 `UiEvent`、`ChatMessage`、`MessageRole`、`AskUserState`、`HostControlPlane::{answer_ask,cancel_ask,register_turn_event_sink,unregister_turn_event_sink}`、`stop_turn`、`StopReason` 或事件字段/版本。
- 不改 Ask 提交答案 JSON、CAS `request_id + version`、500ms 轮询、答案恢复、权限审批、Ctrl+C、普通非流式 Esc、Composer 指标行及会话持久化。
- 不引入接口空桩，故 `dev-stub-lint` 与新增接口类型检查均**不适用**：实现只调整既有 `tui.rs` 内部布局/呈现，无新增跨模块 API。
- 不迁移历史消息，不删除/重编号字段，不新增错误码、配置、依赖、网络/外部 IO 或线程；仅新增现有 sink/emitter 的按 turn 注册与成对回收义务。

## 2. 黑盒验收矩阵

| ID | 场景与操作 | 必须满足的可见/状态断言 |
|---|---|---|
| O1 | 120×34；先有至少一条短历史（唯一 `HISTORY_SENTINEL`），再打开带唯一 `QUESTION_SENTINEL` 的 Ask | 同一帧同时可见历史哨兵与问题哨兵；历史不是先渲染后被整区 Clear；Ask 不得覆盖 header、Composer 或指标行。 |
| O2 | 同一状态、宽度、滚动位置，仅切换 Ask 显示 | `chat_rect` 与可见行窗口不得因 Ask 高度重算；Ask 与消息区允许相交，面板下缘贴 Composer 上缘。仅被面板覆盖的行允许隐藏，未覆盖历史逐格不变。 |
| O3 | 宽/窄终端及极小高度；长问题、长选项、CJK | 不 panic、不越出终端；保留既有宽≤12/窄≤8 行上限；空间不足优先保留问题与提交/取消提示，不承诺全部选项可见。 |
| O4 | 流式中同一 turn 在 500ms 内双击 Esc | 第一击只上膛；第二击恰好一次停止请求。受理后仅显示「正在停止…」；未收到终态不得显示已停止/已终止。 |
| O5 | 上述 turn 收到 `turn:cancelled`，`stopReason="user"` | 既有状态通道显示「已停止（用户中断）」（即用户可读的已终止语义），`is_streaming=false`、stopping 清理；已有正文保留；不得新增 `ChatMessage`。 |
| O6 | stopping 中重复 Esc；同一终态事件重复/陈旧 turn 终态 | 不重复派发停止请求，不重复追加状态/用户/系统消息；陈旧 turn 不得终止当前 turn。 |
| C9 | 普通初始 turn 与每个 goal 续轮；含 start_turn_stream 早退/返回 | 每轮调用前用现有 `register_turn_event_sink(turn_id, Arc<TuiTurnSink>)` 注册同一 UI 通道，使 stop_requested/cancelled 可达；该次调用结束后以 `unregister_turn_event_sink(turn_id)` 成对回收 sink 与 emitter。不得新增事件、线程、emitter 类型或 core 改动；注册项不得跨 turn 泄漏。 |
| C10 | 流式双击 Esc 后检查真实屏幕与原始 pane 流 | 结构化 JSON 日志不得覆盖消息区或成为聊天消息；TUI 启动后将既有 JSON 日志缺省 stdout 改道已重定向 stderr，终态仍显示「已停止（用户中断）」；不新增日志字段/出口。 |
| O7 | Ask 面板 Esc 成功取消 | 沿用既有 `cancel_ask(request_id, version)` 一次并关闭面板；不新增用户/系统消息，不调用 `answer_ask` 或 `stop_turn`，不注入取消答案 JSON。CAS 失败不得谎报成功。Ask 内 Esc 不继承普通流式双击停止语义。 |
| O8 | 取消事件携带内部 `text`/`error`/JSON/Debug 形态哨兵 | 用户状态只来自受控停止原因投影；原始事件、工具参数、协议 JSON、Rust Debug 不得进入用户消息或取消状态提示。正常模型正文中合法出现的 `{`/`}` 不在禁用范围。 |
| J1 | resume 的 trace 同时含 `description`、`arguments_text`、`result_text`，注入唯一 JSON 哨兵 | 非空 `description` 优先于 arguments/result；若 `description` 只是工具名占位，只提取结构化参数中的已知人类可读 `description` 字段，不得反显整段 JSON；交互 Ask（复用现有名称分类：`ask_user`/`ask`/`builtin:ask`/`builtin:ask_user`，大小写不敏感）的结构化 arguments/result 不得成为消息摘要。普通工具 description 为空时保留既有 arguments→result 回退，不引入通用 sanitizer。 |

## 3. 候选方案裁决依据

- **A：仅删除整区 Clear，仍从消息区扣 Ask 高度**：历史可恢复，但开关面板会重排/跳动，不满足覆盖层定义，否决。
- **B：Ask 覆盖层 + Ask 定向摘要 + 注册既有停止 sink（采纳）**：消息几何与 Ask 解耦；resume/live 共用 description 非空优先并仅对交互 Ask 抑制结构化摘要；每轮接通并按对回收现有 sink/emitter，终态只走现有事件通道；改动限 `tui.rs`，公共契约零变化。
- **C：新增取消事件/消息类型或通用 payload sanitizer**：扩大序列化与持久化爆炸半径，且会改变既有取消历史语义，否决。

## 4. 独立验收与物理收据

- Test Agent 写域：`crates/pony-agent-core/tests/tui_overlay_cancel_acceptance.rs`、`tests/acceptance/l2at-tui-overlay-cancel.sh`、`tests/acceptance/receipts/l2at-tui-overlay-cancel/`、`.dev-team/l2at-done-tui-overlay-cancel.json`。
- 红相至少 O1/O2/J1 必须在现实现上以断言失败；C9 需证明未注册时停止事件不可达且修复后可观察，O4–O8 中已满足项作为兼容回归，不得为制造红相而伪造失败。对抗注入使用确定性同 turn/异 turn/重复事件与含 JSON 哨兵 payload，禁止 sleep 概率竞态。
- 绿相由 Lead 统一运行该验收目标及相关既有 TUI 回归、`cargo check --manifest-path crates/pony-agent-core/Cargo.toml --all-targets`，逐项保留原生 Exit Code；以当前构建的真实 PTY 留 Ask 打开、Ask 取消、普通流式双击停止终态三类帧，断言无 panic、无内部 JSON 哨兵且停止状态可见。
- 本波触及 UI 交互：Executor 在 `docs/observability.yaml` 追加「四维归属不变 + 无新增出口/字段」说明；Lead 跑 `audit-diff.sh` 与 `check-declaration.sh`，WARN 不得静默当 PASS。
- NFR：`.dev-team/nfr-baseline-tui-overlay-cancel.json` 必须经 `node .dev-team/tools/validate-nfr-baseline.mjs <该文件>` Exit 0；不套用项目总 NFR Schema，也不声称新增日志/锁/资源机制。
