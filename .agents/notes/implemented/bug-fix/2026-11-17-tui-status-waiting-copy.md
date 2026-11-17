# Agent Note: 等待首字节的状态提示只说「在等」

Status: implemented

## Problem

用户看到 TUI 底部状态行在等首 token 时显示：

```
思考中 45s · 仍在等待上游首字节…（已静默 40s）
```

这串文案有两层问题：

1. **工程细节外泄给用户**：「上游 / 首字节 / 已静默 Ns」是排障术语（属于运行日志与 trace），对用户没有任何可行动信息；用户只需要知道「它在等」。
2. **事实错位**：上游尚未产出任何增量时写「思考中」，是对模型状态的虚构——此时并没有在思考，只是请求已发出、尚无首包。

来源是 backlog:B220 引入的上游静默心跳（ADR 2026-11-16-tui-upstream-stall-heartbeat）：`UpstreamPhase`（RequestSent/FirstByte/Delta）+ 5s/15s 两档阈值 + `StallHint` 拼进状态行。心跳的**事件驱动**部分是对的（用真实 `calling_model` / `turn:delta` 起点，不伪造静默），错的是把它翻译成了工程措辞并随行附注。

桌面端同源问题：`resolveAgentRuntimeStatus` 在 `phase="calling_model"` 且 `hasStreamingContent === false`（请求已发出、没有任何增量）时返回「思考中...」，同样把「等待」说成了「思考」。

## Decision

1. **TUI 状态行只由「状态词 + 耗时」两段构成**，不追加任何附注（`compose_running_status_text` 删除 `stall_hint` 参数与 3/4 参数双版本，回到单一签名，`ui()` 渲染点直接调用）。
2. **等待首字节 = 等待态文案**：`status_word_base` 在 `state.upstream_idle.awaiting_first_byte()` 为真时直接返回 `("等待响应中…", StatusTone::Muted)`。等待时长由既有 `elapsed` 承担，不再另造静默秒数。
3. **状态机从「三阶段 + 阈值」收敛为布尔窗口**：`status_dwell.rs` 的 `UpstreamIdle` 只保留 `awaiting_first_byte: bool` 与 `note_request_sent()` / `note_delta()` / `reset()` / `awaiting_first_byte()`；删除 `UpstreamPhase`、`StallHint`、`stall_hint()`、`UPSTREAM_STALL_HINT_MS`、`UPSTREAM_STALL_ALERT_MS`。事件接线不变（`calling_model` 点亮、任一增量熄灭、工具执行/轮次收口/换会话复位）。
4. **桌面端对齐同一口径**：`src/lib/runtime/status.ts` 的 `calling_model && hasStreamingContent === false` 分支返回 `label: "等待响应中…"`（`mode` 沿用 `thinking`，同一条脉冲动画）；`hasStreamingContent` 未知（`undefined`）的既有调用方语义不变（仍「正在回复...」），不扩大契约面。
5. **可机器验收**：
   - TUI：`b231_*` 四例（等待词、状态行无工程细节白名单、首增量退窗、仅真实事件驱动）；
   - 桌面端：`runtime-status.test.ts` / `runtime-status.spec.ts` 断言「等待响应中…」且不含工程细节词；
   - 冻结验收 `wave59-f1-waiting-animation.spec.ts` 的 F1-2 只把允许文案集合扩为 `等待响应中|思考中|准备就绪`（F1-1「唯一指示」契约与节点计数断言原封不动）。

## Alternatives considered

- **A. 保留静默心跳，只把措辞换成「等待中」**：落选。「已静默 Ns」与既有 `elapsed` 是同一量的两个数字，同屏重复且互相打架；静默阈值（5s/15s）也不再影响任何用户可见行为，留着只会腐化成第二套真源。工程侧仍可从运行日志与 trace 取静默事实。
- **B. 桌面端新增 `waiting` mode（新图标/新动画）**：落选。`AgentRuntimeMode` 是跨组件契约（`isHighPriorityMode`、`data-mode` 动画节奏、多处测试夹具），为一句文案新增模式属于放大契约面；等待与思考使用同一条脉冲动画在视觉上无差别，只改文案收益/成本比更好。
- **C. 桌面端把 `hasStreamingContent === undefined` 也算等待**：落选。该入参缺省是既有调用方的合法用法（多处测试与夹具），把「未知」当「无增量」会静默改变它们的输出；只对 store 实际传入的显式 `false` 生效。
- **D. 干脆不显示等待态（回到「生成中」）**：落选。长静默期用户需要知道「在等而不是死了」；`等待响应中…` 用最少字符给出这一事实，且与「思考中」「正在回复」三态互不冒充。
- **E. 把等待态并进 B198 的 followup 阶段通道**：落选。阶段通道回答「兜底链路卡在哪一步」（流程维度），等待窗口回答「本跳还没首包」（单跳维度）；两者正交，合并会让重试阶段的精确计数被布尔窗口吞掉。

## Consequences

- 状态行示例变成 `等待响应中…  45s`；首增量到达后立刻切回 `思考中…` / `正在回复…` + 耗时。
- 删除 5s/15s 两档阈值与三阶段枚举后，「静默多久才提示」这类参数不再存在——不存在调参面，也不存在「阈值改了但文案没改」的漂移。
- 端到端仍保留「无事件不伪造等待态」不变量：`b231_waiting_window_is_event_driven` 钉住默认态、工具执行期与 reset 后都必须退出窗口。
- 不新增对外接口、网络 IO、线程或后台任务；`docs/observability.yaml` 四维归属仍为结构性 N-A（TUI/桌面端无常驻监听地址），只更新行为收口点与用例名。
- 与 ADR 2026-11-16-tui-upstream-stall-heartbeat 的关系：该条**部分被取代**——事件驱动（真实 `calling_model` / `turn:delta` 起点、不伪造静默）保留；阈值化提示文案与三阶段枚举撤销。旧条正文链回本条。
