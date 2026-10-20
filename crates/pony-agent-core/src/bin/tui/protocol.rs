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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskUserState {
    pub request_id: String,
    pub questions: Vec<AskUserQuestion>,
    pub current_question_index: usize,
    pub selected_option_index: usize,
    pub multi_selected_indices: Vec<usize>,
    pub custom_input: String,
    pub is_custom_input_active: bool,
}

impl AskUserState {
    pub fn new(request_id: String, questions: Vec<AskUserQuestion>) -> Self {
        Self {
            request_id,
            questions,
            current_question_index: 0,
            selected_option_index: 0,
            multi_selected_indices: Vec::new(),
            custom_input: String::new(),
            is_custom_input_active: false,
        }
    }

    pub fn current_question(&self) -> Option<&AskUserQuestion> {
        self.questions.get(self.current_question_index)
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
