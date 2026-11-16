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

/// 上游静默**提示档**阈值：超过 5s 没有任何 delta ⇒ 状态行追加一句「仍在等待…」。
///
/// 现场依据（backlog:B220）：长静默期界面无任何变化，用户无法区分「上游在慢」与「会话死了」。
pub const UPSTREAM_STALL_HINT_MS: u64 = 5_000;

/// 上游静默**告警档**阈值：超过 15s ⇒ 文案升级为「上游已静默」。
pub const UPSTREAM_STALL_ALERT_MS: u64 = 15_000;

/// 上游等待阶段（真实事件驱动，不伪造）。
///
/// - `RequestSent`：已发出模型调用请求（后端 `turn:trace` 的 `phase=calling_model`），尚未见任何增量；
/// - `FirstByte`：已见到本次调用的**首个**增量（首个 delta）；
/// - `Delta`：已在读流（首个之后的增量）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum UpstreamPhase {
    /// 已发起请求，等待首字节。
    #[default]
    RequestSent,
    /// 首字节已到（首增量之后进入 `Delta`）。
    FirstByte,
    /// 读流中。
    Delta,
}

impl UpstreamPhase {
    /// 名词式阶段文案（告警档用）。
    pub fn label(self) -> &'static str {
        match self {
            UpstreamPhase::RequestSent => "等待首字节",
            UpstreamPhase::FirstByte => "等待上游响应",
            UpstreamPhase::Delta => "读流中",
        }
    }

    /// 进行式阶段文案（提示档用，接在「仍在…」之后）。
    ///
    /// 三个阶段必须**两两可区分**：同一静默时长下若 `RequestSent` 与 `FirstByte` 文案相同，
    /// 用户就看不出卡在「请求还没被受理」还是「首个增量还没来」。
    pub fn waiting_label(self) -> &'static str {
        match self {
            UpstreamPhase::RequestSent => "仍在等待上游首字节",
            UpstreamPhase::FirstByte => "已收到首字节，仍在等待上游更多响应",
            UpstreamPhase::Delta => "仍在接收增量",
        }
    }
}

/// 上游静默提示（纯函数产物，渲染层只读文本与档位）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StallHint {
    /// 产生该提示的阶段。
    pub phase: UpstreamPhase,
    /// 该阶段的静默时长（毫秒）。
    pub silent_ms: u64,
    text: String,
}

impl StallHint {
    /// 是否已进入告警档（`silent_ms >= UPSTREAM_STALL_ALERT_MS`）。
    pub fn is_alert(&self) -> bool {
        self.silent_ms >= UPSTREAM_STALL_ALERT_MS
    }

    /// 可渲染文案（非空；含阶段与静默秒数）。
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// 纯函数：阶段 + 静默时长 ⇒ 提示文案。
///
/// 三档，边界是**闭区间语义**（`silent_ms` 恰为阈值即进入该档）：
/// - `< 5s` ⇒ `None`（正常抖动，不打扰）；
/// - `5s..<15s` ⇒ 提示档：`仍在等待上游首字节…（已静默 8s）`；
/// - `>=15s` ⇒ 告警档：`上游已静默 16s · 等待首字节`（含「上游已静默」+ 阶段，与提示档可区分）。
pub fn stall_hint(phase: UpstreamPhase, silent_ms: u64) -> Option<String> {
    if silent_ms < UPSTREAM_STALL_HINT_MS {
        return None;
    }
    let secs = silent_ms / 1_000;
    Some(if silent_ms < UPSTREAM_STALL_ALERT_MS {
        format!("{}…（已静默 {}s）", phase.waiting_label(), secs)
    } else {
        format!("上游已静默 {}s · {}", secs, phase.label())
    })
}

/// 上游静默跟踪器：**只**由真实事件推进（请求已发出 / 增量到达 / 轮次收口）。
///
/// 时间由调用方注入（`now_ms`），本类型不读时钟 ⇒ 单测可喂假时间。
/// `Default` = 「没有任何活动事件」：此时 `hint` 恒为 `None`（不凭空报告上游静默）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UpstreamIdle {
    /// 是否处于「已发起请求 / 正在读流、尚未收口」的等待窗口。
    active: bool,
    phase: UpstreamPhase,
    last_activity_ms: u64,
}

impl UpstreamIdle {
    /// 模型调用请求已发出（`turn:trace` 的 `calling_model`）⇒ 进入等待首字节阶段。
    pub fn note_request_sent(&mut self, now_ms: u64) {
        self.active = true;
        self.phase = UpstreamPhase::RequestSent;
        self.last_activity_ms = now_ms;
    }

    /// 增量到达（正文或思考）⇒ 首次进入 `FirstByte`，其后进入 `Delta`，并刷新活动时刻。
    pub fn note_delta(&mut self, now_ms: u64) {
        self.phase = match (self.active, self.phase) {
            // 首增量（含「没见到 calling_model 就先来增量」的路径）⇒ 首字节已到。
            (false, _) | (true, UpstreamPhase::RequestSent) => UpstreamPhase::FirstByte,
            (true, _) => UpstreamPhase::Delta,
        };
        self.active = true;
        self.last_activity_ms = now_ms;
    }

    /// 轮次边界 / 终态：退出等待窗口（`hint` 立即变 `None`）。
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// 当前阶段（无活动事件时为默认阶段 `RequestSent`，但 `hint` 仍为 `None`）。
    pub fn phase(&self) -> UpstreamPhase {
        self.phase
    }

    /// 是否处于等待窗口。
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// 距上次真实活动的静默时长；不在等待窗口内 ⇒ `0`。
    pub fn silent_ms(&self, now_ms: u64) -> u64 {
        if !self.active {
            return 0;
        }
        now_ms.saturating_sub(self.last_activity_ms)
    }

    /// 本帧的静默提示（`None` ⇒ 不追加任何文案）。
    pub fn hint(&self, now_ms: u64) -> Option<StallHint> {
        if !self.active {
            return None;
        }
        let silent_ms = self.silent_ms(now_ms);
        stall_hint(self.phase, silent_ms).map(|text| StallHint {
            phase: self.phase,
            silent_ms,
            text,
        })
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
    // B220：上游静默提示（阈值边界 / 阶段切换 / 复位）
    // ─────────────────────────────────────────────────────────────────────────

    /// 阈值边界：4999 无提示、5000 进提示档、14999 仍是提示档、15000 进告警档。
    #[test]
    fn stall_hint_thresholds_are_closed_intervals() {
        assert_eq!(
            stall_hint(UpstreamPhase::RequestSent, 4_999),
            None,
            "5s 以内是正常抖动，不得提示"
        );

        let hint_tier = stall_hint(UpstreamPhase::RequestSent, 5_000)
            .expect("恰为 5s 必须进入提示档（边界闭区间）");
        assert!(
            hint_tier.contains("已静默 5s"),
            "提示档文案必须含静默秒数，实际 = {hint_tier:?}"
        );
        assert!(
            hint_tier.contains("上游首字节"),
            "提示档文案必须点名阶段，实际 = {hint_tier:?}"
        );
        assert!(
            !hint_tier.contains("上游已静默"),
            "提示档不得使用告警档措辞，实际 = {hint_tier:?}"
        );

        let still_hint_tier = stall_hint(UpstreamPhase::Delta, 14_999)
            .expect("14999ms 仍在提示档");
        assert!(
            !still_hint_tier.contains("上游已静默"),
            "14999ms 不得提前进入告警档，实际 = {still_hint_tier:?}"
        );

        let alert = stall_hint(UpstreamPhase::Delta, 15_000)
            .expect("恰为 15s 必须进入告警档（边界闭区间）");
        assert!(
            alert.contains("上游已静默 15s"),
            "告警档文案必须含「上游已静默」+ 秒数，实际 = {alert:?}"
        );
        assert!(
            alert.contains("读流中"),
            "告警档文案必须点名阶段，实际 = {alert:?}"
        );
        assert_ne!(
            alert, still_hint_tier,
            "5s 档与 15s 档文案必须可区分"
        );
    }

    /// 阶段切换：请求发出 ⇒ RequestSent；首增量 ⇒ FirstByte；其后 ⇒ Delta。
    #[test]
    fn upstream_idle_phase_transitions() {
        let mut idle = UpstreamIdle::default();
        assert!(!idle.is_active(), "Default = 无任何活动事件");
        assert_eq!(idle.silent_ms(10_000), 0, "无活动事件时静默时长恒为 0");

        idle.note_request_sent(1_000);
        assert_eq!(idle.phase(), UpstreamPhase::RequestSent);
        assert_eq!(idle.silent_ms(9_000), 8_000);
        let first = idle.hint(9_000).expect("静默 8s 必须提示");
        assert!(!first.is_alert(), "8s 仍是提示档");
        assert!(first.text().contains("仍在等待上游首字节"));

        idle.note_delta(9_500);
        assert_eq!(idle.phase(), UpstreamPhase::FirstByte, "首个 delta ⇒ 首字节已到");
        idle.note_delta(10_000);
        assert_eq!(idle.phase(), UpstreamPhase::Delta, "首增量之后 ⇒ 读流中");
        assert_eq!(idle.silent_ms(10_000), 0, "刚到的增量刷新活动时刻");
        assert_eq!(idle.hint(10_500), None, "刚有活动 ⇒ 不提示");

        let streaming = idle.hint(30_000).expect("读流期静默 20s 必须告警");
        assert!(streaming.is_alert());
        assert!(streaming.text().contains("上游已静默"));
        assert!(streaming.text().contains("读流中"));
    }

    /// 复位：`reset()` 后既无提示也不残留阶段（轮次边界不得把上一轮静默带进下一轮）。
    #[test]
    fn upstream_idle_reset_clears_hint() {
        let mut idle = UpstreamIdle::default();
        idle.note_request_sent(0);
        assert!(idle.hint(20_000).is_some(), "前提：静默 20s 有告警");

        idle.reset();
        assert!(!idle.is_active());
        assert_eq!(idle.silent_ms(20_000), 0);
        assert_eq!(idle.hint(20_000), None, "复位后不得报告任何上游静默");
    }
}
