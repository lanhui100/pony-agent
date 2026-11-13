# Agent Note: 上游重试次数上调到 10 次，重试进度与终态错误的口径收口（TUI 状态消息）

Status: implemented

## Problem

用户提出三条诉求（原话：「上游模型调用重试的次数是多少？我记得是参考 dsh 的 5 次，请提高到 10 次；
另外重试信息在消息流的状态消息中显示；错误消息也应该在状态消息显示，且不应该重复显示，已经有警示色
显示了，则不用加重显示了」）。对照代码实测，三条各自对应一个真实缺陷：

1. **次数口径与曲线装不下**：`UPSTREAM_DECISION_MAX_ATTEMPTS_DEFAULT = 5` 是**含首次**的尝试次数
   （`max_retries = attempts - 1`，`config::UpstreamPolicy::short_backoff_config`）⇒ 实际只有 4 次
   重试；而退避预算 `UPSTREAM_RETRY_BUDGET_MS_DEFAULT = 30_000` 相对曲线
   （500ms→×2→封顶 8s）只能装下 7 次尝试（9 次尝试的 sleep 合计需 47.5s）。**只改次数不改预算，
   第 7 次尝试就会被 `elapsed >= total_budget_ms` 提前耗尽**，次数上调形同虚设。
2. **重试过程对界面完全黑盒**：重试循环只写 `provider_log`（落盘）与线程局部账本
   `note_provider_attempt_*`（只供结算），不发任何 UI 事件；`UiEvent` 里没有任何重试变体。
   历史实现曾在 `StreamFailed` 分支伪造「重试中（1/5）」——那已经是**终态**，且分母是硬编码假值，
   后被删除（`tui.rs` 该分支留有考古注释）。前端 `lib/runtime/status.ts` 的「正在重试（x/y）」
   也因缺后端分母只能降级为「正在重试…」。
3. **终态错误重复且加重**：`UiEvent::StreamFailed` 臂同时写状态行 `status_text` 与消息区
   `MessageRole::System` 消息（同一句受控摘要）；而 `severity_style(Error)` 是
   `DANGER + BOLD` ⇒ 同一句错误用户看到两遍，其中一遍还被加粗。

## Decision

1. **次数与预算同步上调**（`agent/config.rs`）：
   - `UPSTREAM_DECISION_MAX_ATTEMPTS_DEFAULT: 5 → 10`（含首次 ⇒ **9 次重试**；上界同为 10 不变，
     用户仍可下调到 `[2,10]`）。
   - `UPSTREAM_RETRY_BUDGET_MS_DEFAULT: 30_000 → 60_000`（9 次退避 sleep 合计 47.5s，留 12.5s 余量）。
   - `UPSTREAM_AGGREGATE_DEADLINE_MS_MAX` **刻意保持 180s**：它约束的是「每次 attempt 都吃满 45s
     超时」的病态慢路径；而重试真正救得回来的快失败（429/5xx/连接重置，单次百毫秒级）9 次重试的
     sleep 仅 47.5s，180s 内可跑满 10 次尝试。即「快失败走满次数、慢超时由墙钟护栏收口」。
2. **重试进度改为真值事件**：`agent/retry.rs` 新增线程局部观察者
   （`RetryNotice` / `install_retry_observer` / `notify_provider_retry` + RAII 守卫），重试循环在
   **每次 `RetryDecision::Retry` 决定后、sleep 之前**派发一次通知（分子 = 已失败次数、分母 =
   本跳尝试总数，与 `provider_log` 同源），限流长退避切换处同样派发。选择线程局部而非 sink 的原因：
   一次 provider 调用（含其内部重试）与 `on_delta` 在**同一线程同步执行**，与本文件既有
   `ATTEMPT_WINDOW` 完全同一取舍；未安装观察者 ⇒ 静默 no-op（CLI / 子代理线程 / 测试零副作用）。
3. **两个宿主各自接线，界面只有一个状态位**：
   - TUI：`UiEvent::ProviderRetry { attempt, max_attempts }` + `TuiState::provider_retry`，在三个
     轮次线程入口用 `install_tui_retry_observer` 安装；`status_word_base` 在进度位非空时优先渲染
     `正在重试（n/N）…`（Warn 色）——**不**以 `is_streaming` 当门（goal 续轮在轮次线程内直接开轮，
     该派生标志未必为真），清空点钉死为「首个正文/思考增量 + 完成/失败/停止终态 + 新轮起点
     （`TurnStarted`）」，不做 TTL 猜测。
   - 终态错误：`RuntimeMode::Failed(summary)` 由状态行**原样呈现受控摘要**（Danger 色），
     消息区不再推重复的 System 消息；`severity_style(Error)` 去掉 `BOLD`（警示色即区分手段）。

## Alternatives considered

- **只把默认次数改成 10、预算与聚合上限都不动**：退避预算 30s 会在第 7 次尝试耗尽，等于改了配置
  却拿不到 10 次；这正是用户抱怨的「配了不生效」类缺陷，故必须同步抬预算。
- **把 `UPSTREAM_AGGREGATE_DEADLINE_MS_MAX` 一并抬到 510s（= 45s×10+60s）**：可让慢超时也跑满 10 次，
  但单次同步等待的最坏墙钟会从 3 分钟级涨到 8.5 分钟，违反 PA-174 F2 的立法本意（用户可接受的同步
  等待有界）。取舍：**保留护栏**，把「次数生效」限定在可救回的快失败场景。
- **新增一条 `turn:retry` 事件走 `TurnEventSink`**：语义更「正统」，但 sink 是 `&impl TurnEventSink`
  逐层传参，无法存进 `'static` 的 thread_local；要在 provider 深处拿到 sink 只能把 sink 穿进 10+ 层
  调用栈（或引入 unsafe 擦除指针），代价远大于收益。观察者方案只在宿主侧安装一次，库层零结构变更。
- **重试信息写进消息流（每条重试一行 System 消息）**：9 次重试会往消息流里堆 9 行噪音，且「重试中」
  是**当前状态**而非历史事实；状态行本来就是唯一瞬时状态位，放这里信息密度最高、且天然原地更新。
- **错误继续双写（状态行 + 消息区）但只去掉加粗**：仍违反「不重复显示」；且两处文案必须永远同步，
  是第二个真值源。故收口为**单一可见载体**（状态行）。
- **保留消息区 System 消息、撤掉状态行摘要**：错误会随下一轮 `status_text` 覆盖而不可回看；
  状态行是用户此刻正在看的位置，且与桌面端「状态消息」语义一致（桌面端错误早就是单源状态消息）。

## Consequences

- 默认行为变更（配置面可见）：`ponycli config effective` 的 `disk_policy` 缺省值变为
  `45000 / 10 / 60000`；`aggregate_budget_ms` 仍是 `min(450000+60000, 180000) = 180000`。
- 已同步更新的冻结断言：`tests/pa172_upstream_policy_config_contract.rs`（默认值 + 常量边界）、
  `src/agent/provider/tests.rs`（默认曲线跑满 10 次尝试、退避合计 47.5s）、
  `src/bin/tui/tests/transient_error_terminal_tests.rs`（Error 不再 BOLD）。
- 新增回归用例：`provider::tests::retry_loop_reports_real_progress_to_observer`（分子/分母真值）、
  `src/bin/tui/tests/provider_retry_status_tests.rs`（状态行优先级、进度位清空时机、观察者转投递、
  守卫摘除、终态摘要落在状态行）。
- 门禁：本卡改了 `docs/observability.yaml`（补归属说明），因此 `ponygo observe audit --level ob2`
  的 D1 判定要求**本次提交**携带 `[decl-change-approved]`（防同一次提交削弱规则集自证清白）；
  声明一致性 `check-declaration.sh` 本身为 PASS。提交信息同时须带 `[backlog:B154]`。
- 遗留（本次未做，非缺陷）：桌面端（`src-tauri`）尚未安装同一观察者，故桌面 App 的状态消息仍只显示
  「正在重试…」占位；接线点已就绪（`run_stream_task` 的 `spawn_blocking` 闭包内装一次即可），
  属独立前端改动（需新增 `turn:retry` 监听与 `retryMax` 透传）。
