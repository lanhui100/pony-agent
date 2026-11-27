//! O4（2026-11-27）：本轮的结构化权威类别（`goal` 元工具授权的唯一真值源）。
//!
//! 独立成模块的理由：它既是**授权真值表**（`create_goal` / `update_goal` 闸门只看它），
//! 又是 `DispatchContext` 的线缆字段类型；留在 `dispatcher.rs` 会让该文件的规模棘轮
//! （`scripts/file-size-baseline.json`）因为一段纯声明式枚举而恶化。放这里后
//! `dispatcher.rs` 只负责 `use` 与字段声明。
//!
//! 与审核报告的关系：`docs/analysis/goal-ask-blocking-audit-2026-11-26.md` §4 O4
//! 要求把 `DispatchContext.is_goal_round: bool` 升级为结构化枚举，由宿主在构造本轮
//! 事实时写入（不靠 `<goal_round ...>` 文案嗅探）。

use serde::{Deserialize, Serialize};

/// O4：本轮的结构化权威类别（取代 `is_goal_round` + `is_direct_human` 两个布尔）。
///
/// 授权真值表（`create_goal` / `update_goal` 的 ghost 动作只看它）：
/// - [`TurnKind::DirectHuman`] ⇒ 人类直发轮，`create_goal` / `pause` / `edit` / `resume` 全允许；
/// - [`TurnKind::GoalRound`] ⇒ 宿主自动续轮，只允许 `complete` / `blocked`（推进既定目标）；
/// - [`TurnKind::AskResume`] / [`TurnKind::ApprovalResume`] ⇒ **恢复轮**，两者都**不**授予
///   ghost 权威（这正是 P1-1 的缺陷本体）；
/// - [`TurnKind::SubAgent`] ⇒ 子代理派发，同样不授予 ghost 权威。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnKind {
    /// 人类直接发起的顶层轮次（默认，与旧 `is_direct_human: true` 等价）。
    #[default]
    DirectHuman,
    /// 宿主自动续轮（`<goal_round ...>` 模板输入）。
    GoalRound,
    /// Ask 作答后的恢复轮。
    AskResume,
    /// 审批决定后的恢复轮。
    ApprovalResume,
    /// 子代理派发的轮次。
    SubAgent,
}

impl TurnKind {
    /// 是否人类直发轮（ghost 类 goal 元工具的唯一授权来源）。
    #[must_use]
    pub fn is_direct_human(self) -> bool {
        matches!(self, Self::DirectHuman)
    }

    /// 是否 goal 自动续轮（`complete` / `blocked` 的第二条授权路径）。
    #[must_use]
    pub fn is_goal_round(self) -> bool {
        matches!(self, Self::GoalRound)
    }

    /// 线缆/日志用的稳定字面量。
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::DirectHuman => "direct_human",
            Self::GoalRound => "goal_round",
            Self::AskResume => "ask_resume",
            Self::ApprovalResume => "approval_resume",
            Self::SubAgent => "sub_agent",
        }
    }

    /// 全部变体（契约测试锚定用；新增变体必须同步登记）。
    #[must_use]
    pub fn all_variants() -> [TurnKind; 5] {
        [
            Self::DirectHuman,
            Self::GoalRound,
            Self::AskResume,
            Self::ApprovalResume,
            Self::SubAgent,
        ]
    }
}

impl TurnKind {
    /// 本类别是否允许 goal 的 **ghost 类**动作（`create`/`pause`/`edit`/`resume`）。
    #[must_use]
    pub fn allows_ghost_goal_actions(self) -> bool {
        self.is_direct_human()
    }

    /// 本类别是否允许 goal 的**推进类**动作（`complete`/`blocked`）。
    #[must_use]
    pub fn allows_goal_progress_actions(self) -> bool {
        self.allows_ghost_goal_actions() || self.is_goal_round()
    }
}
