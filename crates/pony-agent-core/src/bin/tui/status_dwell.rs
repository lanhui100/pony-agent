//! 状态行可见性状态机：**min-dwell**（防一闪而过）+ 轮内**工具活动聚合**。
//!
//! 问题：状态行此前是「当前状态快照」——`ui()` 每帧直接读 `status_word(state)`，
//! 任何 `UiEvent` 都会在下一次绘制把它换掉。工具调用事件（`turn:tool` started →
//! completed）之间往往只隔几十毫秒，于是折叠态下用户只看到「生成中」，
//! 具体在跑什么工具闪一下就没了。
//!
//! 本模块把「显示什么」从「当前快照」改成「受最小驻留时间约束的可见文本」：
//! - `StatusDwell`：可见文本至少要存活 [`STATUS_DWELL_MS`] 才允许被普通候选替换；
//!   终态（失败 / 待确认 / 正在停止…）走 `Immediate`，永远立即抢占。
//! - `ToolActivity`：轮内工具调用计数 + 当前/最近工具文案，让「生成中」也能带上
//!   「刚做完什么工具」的上下文（折叠态下用户唯一的信息来源）。
//!
//! 纯逻辑：时间由调用方注入（`now_ms`），单测可直接喂假时间；渲染层只读可见文本。

use std::sync::OnceLock;
use std::time::Instant;

/// 普通状态词的最小驻留时长：短于一帧的工具态（如 30ms 的 read_file）也能被看见。
pub const STATUS_DWELL_MS: u64 = 1200;

/// 候选的抢占级别。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StatusUrgency {
    /// 过程态（思考 / 生成 / 执行工具 / 连接）：受 min-dwell 约束。
    Normal,
    /// 终态与用户动作反馈（失败 / 完成 / 待确认 / 正在停止…）：立即显示。
    Immediate,
}

/// 轮次生命周期：状态行**渲染门**与**文案合成**的同一真源（backlog:B218）。
///
/// 存在的理由：此前「是否进行中」在渲染门、抢占级别、文案合成三处各写一份 `matches!`，
/// 任一处漏改就会出现「门开了但文案为空」「终态残留进行中」这类分家缺陷。
/// 现在只有 `turn_lifecycle`（`tui.rs`）一处判定，其余消费方一律复用它。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TurnLifecycle {
    /// 轮次进行中：必须渲染非空状态行（含工具执行期与折叠态）。
    Running,
    /// 终态（就绪 / 完成 / 失败 / 待确认）：由 `status_notice`、消息区与 Ask 面板承载。
    Terminal,
}

/// 状态词的语义色（渲染层据此选 `theme` 色，模块本身不依赖 ratatui）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StatusTone {
    Muted,
    Faint,
    Warn,
    Success,
    Danger,
}

/// 一帧的状态行候选：文案 + 抢占级别 + 语义色。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StatusCandidate {
    pub text: String,
    pub urgency: StatusUrgency,
    pub tone: StatusTone,
}

impl StatusCandidate {
    pub fn normal(text: impl Into<String>, tone: StatusTone) -> Self {
        Self {
            text: text.into(),
            urgency: StatusUrgency::Normal,
            tone,
        }
    }

    pub fn immediate(text: impl Into<String>, tone: StatusTone) -> Self {
        Self {
            text: text.into(),
            urgency: StatusUrgency::Immediate,
            tone,
        }
    }
}

/// 当前正在显示的状态词。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VisibleStatus {
    pub text: String,
    pub tone: StatusTone,
    /// 该文本开始显示的单调毫秒。
    pub started_ms: u64,
    /// 必须驻留的时长（`Immediate` 为 0）。
    pub min_dwell_ms: u64,
}

/// min-dwell 状态机。
#[derive(Default, Clone, Debug)]
pub struct StatusDwell {
    visible: Option<VisibleStatus>,
    pending: Option<StatusCandidate>,
}

impl StatusDwell {
    pub fn new() -> Self {
        Self::default()
    }

    /// 喂入本帧候选。`now_ms` 必须单调递增（毫秒）。
    ///
    /// 判定顺序（首条命中即返回）：
    /// 1. 尚无可见文本 ⇒ 立即显示；
    /// 2. 与可见文本相同 ⇒ 只同步语义色，**不重置**驻留计时（秒数刷新不会拖住切换）；
    /// 3. `Immediate` ⇒ 立即抢占；
    /// 4. 可见文本已驻留满 `min_dwell` ⇒ 切换；
    /// 5. 其余 ⇒ 挂起为 pending（可被更新的候选覆盖，不排队）。
    pub fn observe(&mut self, now_ms: u64, candidate: StatusCandidate) {
        let Some(visible) = self.visible.as_mut() else {
            self.show(candidate, now_ms);
            return;
        };
        if visible.text == candidate.text {
            visible.tone = candidate.tone;
            if self.pending.as_ref().is_some_and(|p| p.text == candidate.text) {
                self.pending = None;
            }
            return;
        }
        let dwell_elapsed = now_ms.saturating_sub(visible.started_ms) >= visible.min_dwell_ms;
        if candidate.urgency == StatusUrgency::Immediate || dwell_elapsed {
            self.show(candidate, now_ms);
            return;
        }
        self.pending = Some(candidate);
    }

    fn show(&mut self, candidate: StatusCandidate, now_ms: u64) {
        let min_dwell_ms = match candidate.urgency {
            StatusUrgency::Immediate => 0,
            StatusUrgency::Normal => STATUS_DWELL_MS,
        };
        self.pending = None;
        self.visible = Some(VisibleStatus {
            text: candidate.text,
            tone: candidate.tone,
            started_ms: now_ms,
            min_dwell_ms,
        });
    }

    pub fn visible_text(&self) -> Option<&str> {
        self.visible.as_ref().map(|visible| visible.text.as_str())
    }

    pub fn visible_tone(&self) -> Option<StatusTone> {
        self.visible.as_ref().map(|visible| visible.tone)
    }

    /// 是否存在被压制、待切换的候选。
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// 下一次需要唤醒重绘的时刻（`None` ⇒ 无待决切换）。
    pub fn next_wake_ms(&self) -> Option<u64> {
        let visible = self.visible.as_ref()?;
        self.pending.as_ref()?;
        Some(visible.started_ms.saturating_add(visible.min_dwell_ms))
    }

    pub fn reset(&mut self) {
        self.visible = None;
        self.pending = None;
    }
}

/// 轮内工具活动：调用计数 + 当前运行中 / 最近完成的工具文案。
///
/// 文案来自 `format_tool_display` 的既有产物（`工具名 · 参数摘要`），本模块只做聚合，
/// 不重新解析工具参数。
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct ToolActivity {
    call_count: u64,
    running: Option<String>,
    last: Option<String>,
}

impl ToolActivity {
    /// 记录一次工具调用（`StreamToolCall` 的唯一写入点）。
    pub fn record_call(&mut self, label: &str) {
        let label = label.trim();
        if label.is_empty() {
            return;
        }
        self.call_count = self.call_count.saturating_add(1);
        self.running = Some(label.to_string());
    }

    /// 工具执行结束（`StreamToolResult`）：当前工具收口为「最近完成」。
    pub fn complete(&mut self, label: &str) {
        let label = label.trim();
        if !label.is_empty() {
            self.last = Some(label.to_string());
        }
        self.running = None;
    }

    /// 轮次边界 / 会话切换：清空当前与最近工具，同时复位计数。
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn call_count(&self) -> u64 {
        self.call_count
    }

    pub fn running(&self) -> Option<&str> {
        self.running.as_deref()
    }

    pub fn last(&self) -> Option<&str> {
        self.last.as_deref()
    }

    /// 执行期文案：`执行 · {当前工具}`；无运行中工具 ⇒ `None`。
    pub fn running_label(&self) -> Option<String> {
        self.running
            .as_deref()
            .map(|label| format!("执行 · {label}"))
    }

    /// 生成 / 思考期的聚合文案；本轮回合尚无工具 ⇒ `None`。
    ///
    /// - 1 次：`1 个工具 · {最近工具}`
    /// - ≥2 次：`{n} 个工具 · 最近 {最近工具}`
    /// - 最近工具未知（极少见）：`{n} 个工具`
    pub fn summary(&self) -> Option<String> {
        let label = self.running.as_deref().or(self.last.as_deref());
        let count = self
            .call_count
            .max(if label.is_some() { 1 } else { 0 });
        if count == 0 {
            return None;
        }
        Some(match label {
            Some(label) if count <= 1 => format!("{count} 个工具 · {label}"),
            Some(label) => format!("{count} 个工具 · 最近 {label}"),
            None => format!("{count} 个工具"),
        })
    }
}

/// 上游等待窗口：模型调用请求已发出、但还没有收到任何增量。
///
/// 状态行据此把「思考中」换成用户可读的「等待响应中…」——用户只需要「在等」这一事实，
/// 不需要「上游 / 首字节 / 已静默 Ns」这类工程细节（那些属于运行日志与 trace）。
///
/// 只由真实事件推进（请求已发出 / 增量到达 / 轮次收口），不读时钟、不另造静默计时：
/// 等待时长由状态行既有的 elapsed 承担。`Default` = 不在等待窗口内。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UpstreamIdle {
    awaiting_first_byte: bool,
}

impl UpstreamIdle {
    /// 模型调用请求已发出（`turn:trace` 的 `calling_model`）⇒ 进入等待首字节窗口。
    pub fn note_request_sent(&mut self) {
        self.awaiting_first_byte = true;
    }

    /// 任一增量到达（正文或思考）⇒ 上游已开始产出，退出等待窗口（幂等）。
    pub fn note_delta(&mut self) {
        self.awaiting_first_byte = false;
    }

    /// 轮次边界 / 终态 / 工具执行期：退出等待窗口。
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// 是否处于「已请求、尚无任何增量」的等待窗口。
    pub fn awaiting_first_byte(&self) -> bool {
        self.awaiting_first_byte
    }
}

/// B198：followup 阶段 ⇒ 状态行文案（固定短语，不拼接 URL / elapsed / provider 细节）。
///
/// 与 [`UpstreamIdle`] 正交：`UpstreamIdle` 只回答「是否在等首字节」（渲染层据此切等待态），
/// 本函数回答「重试/中断链路卡在哪个阶段」（流程维度）——两者都只给用户可读短语，不拼接工程细节。
///
/// 为什么文案在这里而不是 provider 层：provider 只上报**事实**（阶段枚举），
/// 用户可见措辞归展示层（同一事实一个家，避免库层被 UI 文案污染）。
pub fn followup_phase_text(phase: &pony_agent_core::agent::retry::ProviderPhase) -> String {
    use pony_agent_core::agent::retry::ProviderPhase;
    match phase {
        ProviderPhase::StreamFallback => "连接不稳定，重试中…".to_string(),
        ProviderPhase::SyncAttempt { attempt } => format!("连接不稳定，重试中（第 {attempt} 次）…"),
        ProviderPhase::LocalFallback => "本轮未能完成，请稍后重试。".to_string(),
    }
}

/// B216：轮末过程摘要的终态口径。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettledTurnOutcome {
    /// 正常完成（显示耗时）。
    Completed,
    /// 用户停止（只给终态，不显示耗时）。
    Cancelled,
    /// 失败（只给终态，不显示耗时）。
    Failed,
}

/// B216：轮末「过程摘要」一行（**持久信息**，独立于进行中的状态行）。
///
/// 为什么需要它：状态行随轮次终态停止渲染，16.9 分钟的一轮结束后界面上不留任何
/// 「用了多久 / 跑了多少工具」——历史只落在 `session_turn_traces` 里，用户无从回看。
/// 对齐 deepseek harness `TurnProcessNodeView`：轮次 `closed` 后渲染 settled duration；
/// `stopped` / `error` 只给终态摘要、**不显示耗时**（此时耗时无解释力，还容易被读成正常完成）。
///
/// 纯函数：不改状态、不读时钟，耗时由调用方注入。
pub fn settled_turn_summary(
    outcome: SettledTurnOutcome,
    elapsed_ms: Option<u64>,
    tool_calls: u64,
) -> String {
    let head = match outcome {
        SettledTurnOutcome::Completed => match elapsed_ms {
            // 走 `super::`（tui 模块内相对路径）而非 `crate::`：本文件会被测试以
            // `#[path] mod tui;` 形式内联，`crate::` 会解析到测试 crate 根而找不到该函数。
            Some(ms) => format!("本轮完成 · 用时 {}", super::format_elapsed_seconds(ms)),
            None => "本轮完成".to_string(),
        },
        SettledTurnOutcome::Cancelled => "本轮已停止".to_string(),
        SettledTurnOutcome::Failed => "本轮失败".to_string(),
    };
    if tool_calls == 0 {
        head
    } else {
        format!("{head} · 工具 {tool_calls} 次")
    }
}

/// 进程内单调毫秒时钟（TUI 唯一时间源）。
///
/// 只有渲染/事件层调用；纯逻辑模块（`StatusDwell`）一律由调用方注入时间。
pub fn now_ms() -> u64 {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    let origin = ORIGIN.get_or_init(Instant::now);
    origin.elapsed().as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normal(text: &str) -> StatusCandidate {
        StatusCandidate::normal(text, StatusTone::Muted)
    }

    #[test]
    fn first_candidate_is_visible_immediately() {
        let mut dwell = StatusDwell::new();
        assert_eq!(dwell.visible_text(), None);
        dwell.observe(1_000, normal("生成中"));
        assert_eq!(dwell.visible_text(), Some("生成中"));
        assert_eq!(dwell.next_wake_ms(), None, "无 pending 时不该有唤醒");
    }

    #[test]
    fn short_tool_state_survives_min_dwell() {
        let mut dwell = StatusDwell::new();
        dwell.observe(0, normal("生成中"));
        // 30ms 后工具开始（read_file 这种毫秒级工具），100ms 后工具结束。
        dwell.observe(30, normal("执行 · read_file"));
        assert_eq!(
            dwell.visible_text(),
            Some("生成中"),
            "tool 态刚到，尚未满 min-dwell，不得抢掉刚显示的生成中"
        );
        assert!(dwell.has_pending());
        assert_eq!(dwell.next_wake_ms(), Some(STATUS_DWELL_MS));

        dwell.observe(100, normal("生成中 · 1 个工具 · read_file"));
        assert_eq!(
            dwell.visible_text(),
            Some("生成中"),
            "切换前仍显示旧文本（工具态至少可见 min-dwell）"
        );

        // 驻留满 1200ms ⇒ 切到最新候选（中间被覆盖的候选不排队重放）。
        dwell.observe(STATUS_DWELL_MS, normal("执行 · read_file"));
        assert_eq!(dwell.visible_text(), Some("执行 · read_file"));
        assert_eq!(dwell.next_wake_ms(), None);
    }

    #[test]
    fn pending_is_overwritten_by_newer_candidate() {
        let mut dwell = StatusDwell::new();
        dwell.observe(0, normal("生成中"));
        for (i, tool) in ["read_file", "grep", "shell"].iter().enumerate() {
            let now = 10 + i as u64 * 10;
            dwell.observe(now, normal(&format!("执行 · {tool}")));
        }
        assert_eq!(dwell.visible_text(), Some("生成中"));
        dwell.observe(STATUS_DWELL_MS, normal("执行 · shell"));
        assert_eq!(
            dwell.visible_text(),
            Some("执行 · shell"),
            "只显示最新候选，不为被覆盖的 read_file/grep 补帧"
        );
    }

    #[test]
    fn immediate_candidate_preempts_always() {
        let mut dwell = StatusDwell::new();
        dwell.observe(0, normal("生成中"));
        dwell.observe(50, StatusCandidate::immediate("失败", StatusTone::Danger));
        assert_eq!(dwell.visible_text(), Some("失败"));
        assert_eq!(dwell.visible_tone(), Some(StatusTone::Danger));
        assert_eq!(dwell.next_wake_ms(), None);
    }

    #[test]
    fn immediate_visible_is_replaceable_at_once() {
        let mut dwell = StatusDwell::new();
        dwell.observe(0, StatusCandidate::immediate("完成", StatusTone::Success));
        dwell.observe(10, normal("生成中"));
        assert_eq!(
            dwell.visible_text(),
            Some("生成中"),
            "min_dwell = 0 的终态不得把后续过程态挡在门外"
        );
    }

    #[test]
    fn same_text_does_not_restart_timer() {
        let mut dwell = StatusDwell::new();
        dwell.observe(0, normal("生成中"));
        // 每一帧都会重复喂同一候选（秒数由渲染层拼接，不参与文本）。
        for tick in (0..STATUS_DWELL_MS).step_by(100) {
            dwell.observe(tick, normal("生成中"));
        }
        dwell.observe(STATUS_DWELL_MS, normal("执行 · shell"));
        assert_eq!(
            dwell.visible_text(),
            Some("执行 · shell"),
            "同文本重复 observe 不得把驻留计时往后推"
        );
    }

    #[test]
    fn tone_updates_without_timer_restart() {
        let mut dwell = StatusDwell::new();
        dwell.observe(0, normal("生成中"));
        dwell.observe(10, StatusCandidate::normal("生成中", StatusTone::Warn));
        assert_eq!(dwell.visible_tone(), Some(StatusTone::Warn));
        dwell.observe(STATUS_DWELL_MS, normal("执行 · shell"));
        assert_eq!(dwell.visible_text(), Some("执行 · shell"));
    }

    #[test]
    fn tool_activity_counts_and_summarizes() {
        let mut activity = ToolActivity::default();
        assert_eq!(activity.summary(), None, "本轮无工具时不得伪造工具信息");

        activity.record_call("read_file · src/main.rs");
        assert_eq!(activity.call_count(), 1);
        assert_eq!(
            activity.summary().as_deref(),
            Some("1 个工具 · read_file · src/main.rs")
        );
        assert_eq!(
            activity.running_label().as_deref(),
            Some("执行 · read_file · src/main.rs")
        );

        activity.complete("read_file · src/main.rs");
        activity.record_call("shell · git status --short");
        activity.complete("shell · git status --short");
        assert_eq!(activity.call_count(), 2);
        assert_eq!(
            activity.summary().as_deref(),
            Some("2 个工具 · 最近 shell · git status --short")
        );
        assert_eq!(activity.running_label(), None);

        activity.reset();
        assert_eq!(activity.call_count(), 0);
        assert_eq!(activity.summary(), None);
    }

    #[test]
    fn dwell_reset_clears_visible_and_pending() {
        let mut dwell = StatusDwell::new();
        dwell.observe(0, normal("生成中"));
        dwell.observe(10, normal("执行 · shell"));
        assert!(dwell.has_pending());
        dwell.reset();
        assert_eq!(dwell.visible_text(), None);
        assert!(!dwell.has_pending());
        assert_eq!(dwell.next_wake_ms(), None);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // B231：上游等待窗口（只回答「是否在等首字节」，不产生任何工程细节文案）
    // ─────────────────────────────────────────────────────────────────────────

    /// 窗口生命周期：无事件 ⇒ 不在等待；请求发出 ⇒ 等待；任一增量 ⇒ 退出；复位 ⇒ 退出。
    #[test]
    fn upstream_idle_tracks_first_byte_window() {
        let mut idle = UpstreamIdle::default();
        assert!(!idle.awaiting_first_byte(), "Default = 还没有任何上游事件");

        idle.note_request_sent();
        assert!(
            idle.awaiting_first_byte(),
            "请求已发出、尚无增量 ⇒ 必须处于等待首字节窗口"
        );

        idle.note_delta();
        assert!(
            !idle.awaiting_first_byte(),
            "任一增量到达 ⇒ 上游已开始产出，退出等待窗口"
        );
        idle.note_delta();
        assert!(
            !idle.awaiting_first_byte(),
            "后续增量保持退出状态（幂等）"
        );
    }

    /// 复位：轮次边界 / 终态 / 工具执行期后不得残留等待态（否则下一轮开局就挂「等待响应中」）。
    #[test]
    fn upstream_idle_reset_clears_waiting_window() {
        let mut idle = UpstreamIdle::default();
        idle.note_request_sent();
        assert!(idle.awaiting_first_byte(), "前提：复位前处于等待窗口");

        idle.reset();
        assert!(!idle.awaiting_first_byte(), "复位后不得残留等待窗口");
    }

    /// B198：followup 三阶段各有**可区分**的固定文案，且不泄漏内部细节
    ///（URL / elapsed 一旦进状态行，用户读到的就是噪音）。
    #[test]
    fn followup_phase_text_is_stable_and_leak_free() {
        use pony_agent_core::agent::retry::ProviderPhase;

        let stream = followup_phase_text(&ProviderPhase::StreamFallback);
        let sync = followup_phase_text(&ProviderPhase::SyncAttempt { attempt: 3 });
        let local = followup_phase_text(&ProviderPhase::LocalFallback);

        assert_eq!(stream, "连接不稳定，重试中…");
        assert!(sync.contains('3'), "同步兜底的尝试序号必须真实透出：{sync}");
        assert_ne!(stream, local, "降级收尾必须与过程中断可区分");
        for text in [&stream, &sync, &local] {
            assert!(!text.contains("http"), "状态行不得出现 URL：{text}");
            assert!(
                !text.contains("elapsed"),
                "状态行不得出现内部耗时字段：{text}"
            );
        }
    }
}

/// B216：轮末过程摘要的口径（完成显示耗时；停止/失败只给终态）。
#[cfg(test)]
mod settled_turn_summary_tests {
    use super::*;

    #[test]
    fn completed_shows_duration_and_tool_count() {
        let line = settled_turn_summary(SettledTurnOutcome::Completed, Some(1_013_009), 35);
        assert!(line.contains("16分53秒"), "完成轮必须给出真实耗时，实际 = {line:?}");
        assert!(line.contains("35"), "完成轮必须给出工具次数，实际 = {line:?}");
    }

    #[test]
    fn cancelled_and_failed_hide_duration() {
        for outcome in [SettledTurnOutcome::Cancelled, SettledTurnOutcome::Failed] {
            let line = settled_turn_summary(outcome, Some(1_013_009), 3);
            assert!(
                !line.contains("用时") && !line.contains("16分"),
                "非完成轮不得显示耗时（会被读成正常完成），实际 = {line:?}"
            );
            assert!(line.contains('3'), "非完成轮仍要保留已执行工具数，实际 = {line:?}");
        }
    }

    #[test]
    fn zero_tools_omits_tool_segment() {
        let line = settled_turn_summary(SettledTurnOutcome::Completed, Some(2_000), 0);
        assert!(!line.contains("工具"), "没有工具调用时不得出现空计数段，实际 = {line:?}");
    }

    #[test]
    fn missing_elapsed_still_summarises_completion() {
        let line = settled_turn_summary(SettledTurnOutcome::Completed, None, 1);
        assert!(line.contains("本轮完成"), "缺锚点也要给出终态，实际 = {line:?}");
        assert!(!line.contains("用时"), "无锚点不得伪造耗时，实际 = {line:?}");
    }
}
