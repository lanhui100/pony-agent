//! TUI 协议契约与数据结构定义（PA-120 / TUI 进阶交互契约）
//! 覆盖：权限审批(Permission)、AskUser问答、Plan模式与交互、Task看板与Goal展示。

use serde::{Deserialize, Serialize};

// -----------------------------------------------------------------------------
// 1. 权限请求与审批状态数据结构 (Permission)
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionChoice {
    AllowOnce,
    AllowSession,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub request_id: String,
    pub session_id: String,
    pub tool_name: String,
    pub description: String,
    pub command_or_path: Option<String>,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionState {
    pub request: PermissionRequest,
    pub selected_choice_index: usize,
    pub choices: Vec<PermissionChoice>,
}

impl PermissionState {
    pub fn new(request: PermissionRequest) -> Self {
        Self {
            request,
            selected_choice_index: 0,
            choices: vec![
                PermissionChoice::AllowOnce,
                PermissionChoice::AllowSession,
                PermissionChoice::Deny,
            ],
        }
    }

    pub fn selected_choice(&self) -> PermissionChoice {
        self.choices.get(self.selected_choice_index).copied().unwrap_or(PermissionChoice::Deny)
    }

    pub fn next_choice(&mut self) {
        if !self.choices.is_empty() {
            self.selected_choice_index = (self.selected_choice_index + 1) % self.choices.len();
        }
    }

    pub fn prev_choice(&mut self) {
        if !self.choices.is_empty() {
            if self.selected_choice_index == 0 {
                self.selected_choice_index = self.choices.len() - 1;
            } else {
                self.selected_choice_index -= 1;
            }
        }
    }
}

// -----------------------------------------------------------------------------
// 2. AskUser 交互数据结构 (AskUser)
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskUserQuestion {
    pub id: String,
    pub header: Option<String>,
    pub question: String,
    pub multi_select: bool,
    pub options: Vec<QuestionOption>,
}

/// 契约 F-M：单个问题的已答记录（`question_id` 与提问一一对应）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskAnswer {
    pub question_id: String,
    pub values: Vec<String>,
}

/// 契约 F-M：多问推进的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextQuestion {
    /// 还有下一问，索引指向它。
    Advance(usize),
    /// 已经是最后一问（或没有问题）：可以提交了。
    Finish,
}

/// 契约 F-M：多问逐个推进的纯函数。
///
/// `total == 0` ⇒ `Finish`；`current + 1 < total` ⇒ `Advance(current + 1)`；否则 `Finish`。
pub fn next_question_index(current: usize, total: usize) -> NextQuestion {
    if total == 0 {
        return NextQuestion::Finish;
    }
    let next = current.saturating_add(1);
    if next < total {
        NextQuestion::Advance(next)
    } else {
        NextQuestion::Finish
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskUserState {
    pub request_id: String,
    pub questions: Vec<AskUserQuestion>,
    pub current_question_index: usize,
    pub selected_option_index: usize,
    pub multi_selected_indices: Vec<usize>,
    pub custom_input: String,
    pub is_custom_input_active: bool,
    /// 契约 F-M：已答集合（按提问顺序累积）；答完全部问题后由调用方一次性提交。
    pub answers: Vec<AskAnswer>,
}

impl AskUserState {
    pub fn new(request_id: String, questions: Vec<AskUserQuestion>) -> Self {
        let mut state = Self {
            request_id,
            questions,
            current_question_index: 0,
            selected_option_index: 0,
            multi_selected_indices: Vec::new(),
            custom_input: String::new(),
            is_custom_input_active: false,
            answers: Vec::new(),
        };
        state.sync_custom_input_focus();
        state
    }

    /// 让「自定义输入」焦点与当前问题对齐。
    ///
    /// 无选项的问题**只有**自由输入，故默认即聚焦自定义输入；有选项时选中末位（= 自定义项）才算聚焦。
    /// 旧实现在无选项问题上永远不聚焦，用户打字没有任何反应。
    fn sync_custom_input_focus(&mut self) {
        self.is_custom_input_active = match self.current_question() {
            Some(question) => {
                question.options.is_empty() || self.selected_option_index >= question.options.len()
            }
            None => false,
        };
    }

    pub fn current_question(&self) -> Option<&AskUserQuestion> {
        self.questions.get(self.current_question_index)
    }

    /// 契约 F-M：记录当前问题的答案并推进到下一问（**不提交**，提交由调用方负责）。
    ///
    /// - `Advance(next)`：索引已推进到 `next`，并按新问题重置选择状态；
    /// - `Finish`：已是最后一问，索引保持在末问（答案已记录，调用方据此提交全部答案）。
    pub fn record_and_advance(&mut self, values: Vec<String>) -> NextQuestion {
        let question_id = self
            .current_question()
            .map(|question| question.id.clone())
            .unwrap_or_default();
        self.answers.push(AskAnswer {
            question_id,
            values,
        });
        match next_question_index(self.current_question_index, self.questions.len()) {
            NextQuestion::Advance(next) => {
                self.current_question_index = next;
                self.selected_option_index = 0;
                self.multi_selected_indices.clear();
                self.custom_input.clear();
                self.sync_custom_input_focus();
                NextQuestion::Advance(next)
            }
            NextQuestion::Finish => NextQuestion::Finish,
        }
    }

    /// 已答问题数。
    pub fn answered_count(&self) -> usize {
        self.answers.len()
    }

    /// 是否已答完全部问题（没有问题视为未答完）。
    pub fn all_answered(&self) -> bool {
        !self.questions.is_empty() && self.answers.len() >= self.questions.len()
    }

    pub fn toggle_current_option(&mut self) {
        if let Some(q) = self.current_question() {
            if q.multi_select {
                if let Some(pos) = self.multi_selected_indices.iter().position(|&i| i == self.selected_option_index) {
                    self.multi_selected_indices.remove(pos);
                } else {
                    self.multi_selected_indices.push(self.selected_option_index);
                }
            }
        }
    }

    pub fn next_option(&mut self) {
        let (total, options_len) = match self.current_question() {
            Some(q) => (q.options.len() + 1, q.options.len()),
            None => return,
        };
        if total > 0 {
            self.selected_option_index = (self.selected_option_index + 1) % total;
            self.is_custom_input_active = self.selected_option_index == options_len;
            self.sync_custom_input_focus();
        }
    }

    pub fn prev_option(&mut self) {
        let (total, options_len) = match self.current_question() {
            Some(q) => (q.options.len() + 1, q.options.len()),
            None => return,
        };
        if total > 0 {
            if self.selected_option_index == 0 {
                self.selected_option_index = total - 1;
            } else {
                self.selected_option_index -= 1;
            }
            self.is_custom_input_active = self.selected_option_index == options_len;
            self.sync_custom_input_focus();
        }
    }
}

// -----------------------------------------------------------------------------
// 3. Plan 交互与展示数据结构 (Plan)
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    pub step_id: String,
    pub name: String,
    pub summary: String,
    pub status: PlanStepStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanState {
    pub plan_id: String,
    pub revision: u64,
    pub summary: String,
    pub steps: Vec<PlanStep>,
    pub is_active_plan_mode: bool,
    pub selected_step_index: usize,
    pub awaiting_approval: bool,
}

impl PlanState {
    pub fn new(plan_id: String, summary: String) -> Self {
        Self {
            plan_id,
            revision: 1,
            summary,
            steps: Vec::new(),
            is_active_plan_mode: false,
            selected_step_index: 0,
            awaiting_approval: false,
        }
    }

    pub fn toggle_plan_mode(&mut self) {
        self.is_active_plan_mode = !self.is_active_plan_mode;
    }
}

// -----------------------------------------------------------------------------
// 4. Task 交互与展示数据结构 (Task)
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TuiTaskStatus {
    Pending,
    InProgress,
    Completed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuiTask {
    pub id: String,
    pub subject: String,
    pub status: TuiTaskStatus,
    pub owner: Option<String>,
    pub blocked_by: Vec<String>,
    pub write_scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskBoardState {
    pub tasks: Vec<TuiTask>,
    pub selected_index: usize,
    pub is_collapsed: bool,
}

impl TaskBoardState {
    pub fn new(tasks: Vec<TuiTask>) -> Self {
        Self {
            tasks,
            selected_index: 0,
            is_collapsed: false,
        }
    }

    pub fn toggle_collapse(&mut self) {
        self.is_collapsed = !self.is_collapsed;
    }

    pub fn next_task(&mut self) {
        if !self.tasks.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.tasks.len();
        }
    }

    pub fn prev_task(&mut self) {
        if !self.tasks.is_empty() {
            if self.selected_index == 0 {
                self.selected_index = self.tasks.len() - 1;
            } else {
                self.selected_index -= 1;
            }
        }
    }
}

// -----------------------------------------------------------------------------
// 5. Goal 交互与展示数据结构 (Goal)
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalActivation {
    Armed,
    Disarmed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalPhase {
    Active,
    Completed,
    Blocked,
    Paused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuiGoal {
    pub id: String,
    pub objective: String,
    pub phase: GoalPhase,
    pub activation: GoalActivation,
    pub round_count: u32,
    pub max_rounds: Option<u32>,
    pub revision: u64,
}

// -----------------------------------------------------------------------------
// 6. Team 角色花名册 (Agent Roster / W3 可见性)
// -----------------------------------------------------------------------------

/// `/agents` 花名册的一行：全部字段来自**真实运行时**，不造数据。
///
/// - `target`/`name`/`role`/`status`：`orchestration::list_agents()` 的名册快照；
/// - `pending_inbox`：该 target 收件箱里尚未驱动的消息数（`inbox_pending_len`）；
/// - `driving`：是否有后台驱动线程正在跑该 target（`is_driving`）；
/// - `session_id`：该 teammate 最近一次**成功** turn 的子会话 id
///   （`teammate_session_id`；`None` = 尚未成功运行过，Enter 时给出提示）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRosterEntry {
    pub target: String,
    pub name: String,
    pub role: String,
    pub status: String,
    pub pending_inbox: usize,
    pub driving: bool,
    pub session_id: Option<String>,
}

/// `/agents` 面板状态：条目快照 + 当前选择（`selected` 恒落在 `0..len`，空表时为 0）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRosterState {
    pub entries: Vec<AgentRosterEntry>,
    pub selected: usize,
}

impl AgentRosterState {
    pub fn new(entries: Vec<AgentRosterEntry>) -> Self {
        Self {
            entries,
            selected: 0,
        }
    }

    pub fn selected_entry(&self) -> Option<&AgentRosterEntry> {
        self.entries.get(self.selected)
    }

    /// 下移选择（空表/单条时不动，不 wrap 到另一端，避免误按触发跳转）。
    pub fn next(&mut self) {
        if self.selected + 1 < self.entries.len() {
            self.selected += 1;
        }
    }

    /// 上移选择（同 `next` 的边界口径）。
    pub fn prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }
}

impl TuiGoal {
    pub fn new(id: String, objective: String, max_rounds: Option<u32>) -> Self {
        Self {
            id,
            objective,
            phase: GoalPhase::Active,
            activation: GoalActivation::Armed,
            round_count: 0,
            max_rounds,
            revision: 1,
        }
    }
}
