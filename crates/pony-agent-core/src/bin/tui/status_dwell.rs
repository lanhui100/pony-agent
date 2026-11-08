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
}
