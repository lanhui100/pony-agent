//! 后台作业（`job_start` / Run 到期 promote）的 TUI 可见面。
//!
//! 为什么需要这一层：`crates/pony-agent-core/src/agent/jobs.rs` 的注册表此前在 `src/bin/`
//! 下**引用数为 0** —— 作业只在聊天流里留一行工具回执，还在跑 / 跑完 / 失败都没有家，
//! 状态条也无法回答「现在有什么在跑」（见 ADR `2026-11-17-tui-task-visibility-bottom-drawer`）。
//!
//! 分层：本模块只做「注册表 → 视图态」投影（[`JobsViewState`]，可无 TTY 单测）与行渲染；
//! 刷新是**拉取式**（与 `team-panel-live-polling` 同构），不新增事件通道、不改作业生命周期。

use super::*;
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

/// 一条作业的视图行（只留排障必需字段：命令、状态、pid、来源）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobViewRow {
    pub id: String,
    pub command: String,
    pub status: JobStatus,
    pub pid: u32,
    pub kind: String,
}

impl JobViewRow {
    fn from_item(item: JobListItem) -> Self {
        Self {
            id: item.job_id,
            command: item.command,
            status: JobStatus::from_wire(&item.status),
            pid: item.pid,
            kind: item.kind,
        }
    }
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

/// 抽屉 Jobs 页签的纯文本行（无 `Frame` 依赖，渲染与测试共用）。
///
/// 空态**一行**：抽屉空间有限，解释性长句留给 `/panel` 之外的可发现性通道；
/// 同一页签同一时刻只给一句可行动结论。
pub fn render_lines(state: &TuiState) -> Vec<String> {
    let Some(view) = state.jobs_state.as_ref() else {
        return vec!["尚未同步后台任务".to_string()];
    };
    if view.jobs.is_empty() {
        return vec!["当前没有后台任务".to_string()];
    }
    let (running, attention) = view.counts();
    let mut lines = vec![format!(
        "后台任务 {} · 运行中 {running} · 需关注 {attention}",
        view.jobs.len()
    )];
    lines.push(String::new());
    for job in &view.jobs {
        lines.push(format!(
            "  {} {} · pid {} · {} · {}",
            job.status.glyph(),
            job.status.label(),
            job.pid,
            job.kind,
            job.command.trim()
        ));
    }
    lines
}
