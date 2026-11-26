//! 后台作业（`job_start` / Run 到期 promote）的 TUI 可见面。
//!
//! 为什么需要这一层：`crates/pony-agent-core/src/agent/jobs.rs` 的注册表此前在 `src/bin/`
//! 下**引用数为 0** —— 作业只在聊天流里留一行工具回执，还在跑 / 跑完 / 失败都没有家，
//! 状态条也无法回答「现在有什么在跑」（见 ADR `2026-11-17-tui-task-visibility-bottom-drawer`）。
//!
//! 分层：本模块只做「注册表 → 视图态」投影（[`JobsViewState`]，可无 TTY 单测）与行渲染；
//! 刷新是**拉取式**（与 `team-panel-live-polling` 同构），不新增事件通道、不改作业生命周期。

use super::*;
use super::task_center::RowSelection;
use crate::agent::jobs::{job_list_scoped, JobListArgs, JobListItem};

/// 作业状态（wire 口径：`running` / `completed` / `failed` / `killed`）。
///
/// 未知值一律降级 [`JobStatus::Running`]？不 —— 降级成 Running 会把已结束的作业长期算作
/// 「还在跑」，与看板纪律（「绝不把状态吞成成功」）同源地反过来：**未知值降级为 Failed**，
/// 让用户去看一眼，而不是假装它活着。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Running,
    Completed,
    Failed,
    Killed,
}

impl JobStatus {
    pub fn from_wire(status: &str) -> Self {
        match status.trim().to_ascii_lowercase().as_str() {
            "running" => Self::Running,
            "completed" | "complete" | "done" | "exited" => Self::Completed,
            "killed" | "canceled" | "cancelled" | "stopped" => Self::Killed,
            _ => Self::Failed,
        }
    }

    /// 行首状态符号：与任务看板（`✔ ● ○ ✕`）同一套视觉语法。
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Running => "●",
            Self::Completed => "✔",
            Self::Failed => "✘",
            Self::Killed => "✕",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Killed => "killed",
        }
    }

    pub fn is_running(self) -> bool {
        matches!(self, Self::Running)
    }

    /// 「需要人看一眼」：失败算，被杀不算（用户自己叫停的是预期结果）。
    pub fn needs_attention(self) -> bool {
        matches!(self, Self::Failed)
    }
}

/// 一条作业的视图行（只留面向用户的字段：主文案、状态、起始时刻）。
///
/// 刻意**不含** `pid` / `kind`（`owned`/`promoted`）：它们是内部实现细节，用户看
/// 「这个后台任务在干什么、跑了多久、还在不在跑」即可（ADR
/// `2026-11-25-tui-drawer-ux-and-subagent-visibility`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobViewRow {
    pub id: String,
    pub command: String,
    /// 面向用户的中文描述（优先作为主文案）；`None` ⇒ 回退精简命令。
    pub description: Option<String>,
    pub status: JobStatus,
    /// 起始时刻（Unix 秒，与 `JobListItem.started_at` 同口径），用于展示运行时长。
    pub started_at: u64,
}

impl JobViewRow {
    fn from_item(item: JobListItem) -> Self {
        Self {
            id: item.job_id,
            command: item.command,
            description: item.description,
            status: JobStatus::from_wire(&item.status),
            started_at: item.started_at,
        }
    }

    /// 行主文案：优先中文描述，无描述时回退精简后的命令。
    pub fn primary_text(&self) -> String {
        match self.description.as_deref().map(str::trim) {
            Some(text) if !text.is_empty() => text.to_string(),
            _ => compact_command(&self.command),
        }
    }
}

/// 命令精简：单行化 + 长度截断（TUI 一行放不下完整 shell 命令，截断后由折行兜底）。
fn compact_command(command: &str) -> String {
    const MAX_CHARS: usize = 80;
    let single_line = command.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= MAX_CHARS {
        return single_line;
    }
    let mut out: String = single_line.chars().take(MAX_CHARS).collect();
    out.push('…');
    out
}

/// 运行时长（`started_at` 为 Unix 秒；`0` 或未来时刻 ⇒ 空串，不编造时长）。
///
/// 口径：`<60s` 显示 `Ns`；`<60m` 显示 `Nm Ss`；其余 `Nh Nm`。
pub(crate) fn format_elapsed(started_at: u64) -> String {
    if started_at == 0 {
        return String::new();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    format_elapsed_secs(now.saturating_sub(started_at))
}

/// 运行时长（`started_at_ms` 为 Unix 毫秒口径，供子代理运行记录使用）。
pub(crate) fn format_elapsed_ms(started_at_ms: u64) -> String {
    if started_at_ms == 0 {
        return String::new();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0);
    format_elapsed_secs(now.saturating_sub(started_at_ms) / 1000)
}

/// 秒差 → 人读时长（`0` ⇒ 空串：刚开始的任务不显示「0s」噪声）。
fn format_elapsed_secs(secs: u64) -> String {
    if secs == 0 {
        return String::new();
    }
    if secs < 60 {
        return format!("{secs}s");
    }
    let minutes = secs / 60;
    if minutes < 60 {
        return format!("{minutes}m{}s", secs % 60);
    }
    format!("{}h{}m", minutes / 60, minutes % 60)
}

/// 作业视图态：`None`（从未同步）与「同步了但为空」是两件事 —— 前者摘要段不出现，
/// 后者摘要段也不出现，但抽屉能给出「当前没有后台作业」的可引导空态。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JobsViewState {
    pub jobs: Vec<JobViewRow>,
}

impl JobsViewState {
    pub fn from_items(items: Vec<JobListItem>) -> Self {
        Self {
            jobs: items.into_iter().map(JobViewRow::from_item).collect(),
        }
    }

    /// `(running, needs_attention)` 计数，供状态条摘要与页签徽标共用。
    pub fn counts(&self) -> (usize, usize) {
        let running = self.jobs.iter().filter(|job| job.status.is_running()).count();
        let attention = self
            .jobs
            .iter()
            .filter(|job| job.status.needs_attention())
            .count();
        (running, attention)
    }
}

/// 读作业注册表 → `TuiState::jobs_state`（唯一同步点）。
///
/// **只装当前会话归属**的作业（契约 B012）：注册表是进程级的，`job_start` / Run 到期 promote
/// 的作业带 `owner = 发起会话`；不分会话过滤会把别的会话（含 teammate 子会话）的后台任务
/// 混进本条会话的状态栏与抽屉。过滤在注册表层完成（`job_list_scoped`），非 owner 的作业
/// **连存在性都不暴露**。
///
/// 与 `sync_tasks_from_store` 同一形态：失败只写状态文案，不清空既有快照
/// （清空会让「正在跑」在界面上瞬间消失，比显示旧数据更误导）。
pub fn sync_jobs_from_registry(state: &mut TuiState) {
    match job_list_scoped(JobListArgs {}, Some(state.session_id.as_str())) {
        Ok(result) => {
            state.jobs_state = Some(JobsViewState::from_items(result.jobs));
        }
        Err(error) => {
            state.status_text = format!("后台任务同步失败: {error}");
        }
    }
}

/// 抽屉 Jobs 页签的纯文本行（无 `Frame` 依赖，供测试与诊断读取）。
///
/// 空态**一行**：抽屉空间有限，解释性长句留给 `/panel` 之外的可发现性通道；
/// 同一页签同一时刻只给一句可行动结论。
///
/// 渲染走 [`styled_lines`]（带样式与选中态），本函数是它的纯文本投影 —— 两者共用
/// [`summary_text`] / [`job_line`]，不会出现「测试看到 A、屏幕上画 B」。
#[expect(dead_code, reason = "纯文本投影保留给诊断/测试读取，当前仅带样式路径上屏")]
pub(crate) fn render_lines(state: &TuiState) -> Vec<String> {
    let Some(view) = state.jobs_state.as_ref() else {
        return vec!["尚未同步后台任务".to_string()];
    };
    if view.jobs.is_empty() {
        return vec!["当前没有后台任务".to_string()];
    }
    let (running, attention) = view.counts();
    let mut lines = vec![summary_text(view.jobs.len(), running, attention)];
    for job in &view.jobs {
        lines.push(job_line(job));
    }
    if running > 0 {
        lines.push(String::new());
        lines.push(JOB_KILL_HINT.to_string());
    }
    lines
}

/// 可终止作业的底部提示（逐字冻结，供渲染与测试共用）。
pub(crate) const JOB_KILL_HINT: &str = "↑↓ 选择 · x / Ctrl+K 终止后台任务";

/// 顶部紧凑统计行：`后台任务 {total} · 运行中 {running}`（有失败时补 `· 失败 N`）。
fn summary_text(total: usize, running: usize, attention: usize) -> String {
    if attention > 0 {
        format!("后台任务 {total} · 运行中 {running} · 失败 {attention}")
    } else {
        format!("后台任务 {total} · 运行中 {running}")
    }
}

/// 单条作业行：`{图标} {主文案}{ · 时长}`（无 pid / 无 owned|promoted / 无英文状态词）。
fn job_line(job: &JobViewRow) -> String {
    let elapsed = if job.status.is_running() {
        format_elapsed(job.started_at)
    } else {
        String::new()
    };
    let suffix = if elapsed.is_empty() {
        String::new()
    } else {
        format!(" · {elapsed}")
    };
    format!("{} {}{suffix}", job.status.glyph(), job.primary_text())
}

/// 抽屉 Jobs 页签的结构化行（多 `Span` + 精确样式；渲染层按宽度折行）。
///
/// 与 [`render_lines`] 同源（同一 `primary_text` / 统计口径），只是多了样式与选中态。
pub(crate) fn styled_lines(state: &TuiState, selected: Option<usize>) -> Vec<Vec<Span<'static>>> {
    let Some(view) = state.jobs_state.as_ref() else {
        return vec![vec![Span::styled(
            "尚未同步后台任务",
            Style::default().fg(theme::GHOST),
        )]];
    };
    if view.jobs.is_empty() {
        return vec![vec![Span::styled(
            "当前没有后台任务",
            Style::default().fg(theme::GHOST),
        )]];
    }
    let (running, attention) = view.counts();
    let mut rows: Vec<Vec<Span<'static>>> = vec![vec![Span::styled(
        summary_text(view.jobs.len(), running, attention),
        Style::default().fg(theme::MUTED),
    )]];
    for (index, job) in view.jobs.iter().enumerate() {
        rows.push(job_spans(job, RowSelection::from_match(selected, index)));
    }
    if running > 0 {
        rows.push(Vec::new());
        rows.push(vec![Span::styled(
            JOB_KILL_HINT,
            Style::default().fg(theme::GHOST),
        )]);
    }
    rows
}

/// 单条作业的带样式片段：语义色图标 + 主文案 + 弱化时长；选中行铺 `SURFACE_SEL` + `▏` 选中条。
fn job_spans(job: &JobViewRow, selection: RowSelection) -> Vec<Span<'static>> {
    let glyph_style = match job.status {
        JobStatus::Running => Style::default().fg(theme::ACCENT),
        JobStatus::Completed => Style::default().fg(theme::SUCCESS),
        JobStatus::Failed => Style::default().fg(theme::DANGER),
        JobStatus::Killed => Style::default().fg(theme::MUTED),
    };
    let mut spans: Vec<Span<'static>> = Vec::new();
    if selection.is_selected() {
        spans.push(Span::styled("▏", Style::default().fg(theme::ACCENT)));
    }
    spans.push(Span::styled(job.status.glyph(), glyph_style));
    spans.push(Span::raw(" "));
    let text_style = if selection.is_selected() {
        Style::default()
            .fg(theme::INK)
            .bg(theme::SURFACE_SEL)
    } else {
        Style::default().fg(theme::INK)
    };
    spans.push(Span::styled(job.primary_text(), text_style));
    let elapsed = if job.status.is_running() {
        format_elapsed(job.started_at)
    } else {
        String::new()
    };
    if !elapsed.is_empty() {
        spans.push(Span::styled(
            format!(" · {elapsed}"),
            Style::default().fg(theme::GHOST),
        ));
    }
    spans
}
