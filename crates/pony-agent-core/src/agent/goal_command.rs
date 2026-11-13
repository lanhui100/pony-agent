//! 契约 C-1 / C-2：`/goal` 人类斜杠命令的语法与状态机**单一真源**。
//!
//! 桌面（Tauri `goal_command`）与 TUI（`bin/tui/goal_command.rs`）只做接线与渲染，
//! 解析口径、状态迁移与回执文案全部落在本模块，禁止两端各写一份。
//! 本模块只读写 goal store：不读会话历史、不发起 turn、不消耗模型轮次。

use serde::Serialize;

use super::goal::{
    clear_goal, create_goal, get_goal, update_goal, CreateGoalArgs, GetGoalArgs, GoalActivation,
    GoalData, UpdateGoalAction, UpdateGoalArgs,
};

/// 帮助文案真值源（桌面 / TUI / 错误回执共用，禁止两端各写一份）。
pub const GOAL_COMMAND_USAGE: &str = "/goal [<目标>|edit <目标>|pause|resume|complete|clear]";

/// 契约 C-1：`/goal` 之后的剩余文本 → 请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalCommandRequest {
    Show,
    Create { objective: String },
    Edit { objective: String },
    Pause,
    Resume,
    Complete,
    Clear,
}

/// 契约 C-2：命令执行结果（两端渲染的唯一输入）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalCommandOutcome {
    /// "show" | "create" | "edit" | "pause" | "resume" | "complete" | "clear"
    pub action: String,
    /// 已渲染的中文用户回执（多行；不含 CAS/revision 内部细节）。
    pub message: String,
    /// 变更后的目标快照；`clear` 与「无目标」的 show 为 None。
    pub goal: Option<GoalData>,
}

/// 契约 C-1：解析 `/goal` 之后的剩余文本（调用方负责剥离命令 token）。
///
/// `Err` 只表示**语法**错误（edit 缺目标描述），不读存储。口径：
/// 1. 空 ⇒ `Show`；
/// 2. 首 token 为 `clear` / `pause` / `resume` / `complete` 且无额外 token ⇒ 对应请求，
///    带额外 token ⇒ 整串按 `Create{objective}`（正文不得被吞）；
/// 3. `edit` 无剩余 ⇒ `Err`，有剩余 ⇒ `Edit{objective: 剩余 trim}`；
/// 4. 其余任意文本 ⇒ `Create{objective: args.trim()}`；
/// 5. 空 objective 的 `Create` 不可达（第 1 条已覆盖）。
pub fn parse_goal_command(args: &str) -> Result<GoalCommandRequest, String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Ok(GoalCommandRequest::Show);
    }

    // 按**首个空白**切 token；其余文本保持原样（仅做边界 trim）。
    let (head, rest) = match trimmed.split_once(char::is_whitespace) {
        Some((head, rest)) => (head, rest.trim()),
        None => (trimmed, ""),
    };

    match head.to_lowercase().as_str() {
        "clear" if rest.is_empty() => Ok(GoalCommandRequest::Clear),
        "pause" if rest.is_empty() => Ok(GoalCommandRequest::Pause),
        "resume" if rest.is_empty() => Ok(GoalCommandRequest::Resume),
        "complete" if rest.is_empty() => Ok(GoalCommandRequest::Complete),
        "edit" => {
            if rest.is_empty() {
                Err("目标编辑需要提供新的目标描述".to_string())
            } else {
                Ok(GoalCommandRequest::Edit {
                    objective: rest.to_string(),
                })
            }
        }
        _ => Ok(GoalCommandRequest::Create {
            objective: trimmed.to_string(),
        }),
    }
}

/// 契约 C-2：执行命令（真值源）。`session_id` 先 trim，空 ⇒ `Err`。
pub fn execute_goal_command(session_id: &str, args: &str) -> Result<GoalCommandOutcome, String> {
    let session = session_id.trim();
    if session.is_empty() {
        return Err("会话 ID 不能为空".to_string());
    }

    match parse_goal_command(args)? {
        GoalCommandRequest::Show => {
            let goal = current_goal(session)?;
            let message = match goal.as_ref() {
                Some(goal) => render_show(goal),
                None => format!("当前会话未设置目标。\n{GOAL_COMMAND_USAGE}"),
            };
            Ok(GoalCommandOutcome {
                action: "show".to_string(),
                message,
                goal,
            })
        }
        GoalCommandRequest::Create { objective } => {
            let existing = current_goal(session)?;
            if let Some(goal) = existing.as_ref() {
                if goal.phase != "completed" {
                    return Err(format!(
                        "当前已存在{}目标；请用 /goal edit <目标> 修改，或先 /goal clear 清除",
                        phase_label(&goal.phase)
                    ));
                }
            }
            let created = create_goal(CreateGoalArgs {
                session_id: Some(session.to_string()),
                objective,
                // 沿用既有预算（completed 目标被覆盖时不让用户配置静默回落默认值）。
                max_goal_rounds: existing.as_ref().map(|goal| goal.max_goal_rounds),
            })?
            .goal;
            Ok(GoalCommandOutcome {
                action: "create".to_string(),
                message: render_change("目标已创建。自动续轮已启动。", &created),
                goal: Some(created),
            })
        }
        GoalCommandRequest::Edit { objective } => {
            let current = require_goal(session, "edit")?;
            if current.phase == "completed" {
                // 与 DSH 对齐：已完成目标不复活，edit 等价于新建。
                let created = create_goal(CreateGoalArgs {
                    session_id: Some(session.to_string()),
                    objective,
                    max_goal_rounds: Some(current.max_goal_rounds),
                })?
                .goal;
                return Ok(GoalCommandOutcome {
                    action: "create".to_string(),
                    message: render_change("目标已创建。自动续轮已启动。", &created),
                    goal: Some(created),
                });
            }

            let updated = update_goal(UpdateGoalArgs {
                session_id: Some(session.to_string()),
                goal_id: current.id,
                revision: current.revision,
                action: UpdateGoalAction::Edit,
                objective: Some(objective),
                blocked_reason: None,
                max_goal_rounds: None,
            })?
            .goal;
            Ok(GoalCommandOutcome {
                action: "edit".to_string(),
                message: render_change("目标已更新。", &updated),
                goal: Some(updated),
            })
        }
        GoalCommandRequest::Pause => {
            let current = require_goal(session, "pause")?;
            let updated = update_goal(UpdateGoalArgs {
                session_id: Some(session.to_string()),
                goal_id: current.id,
                revision: current.revision,
                action: UpdateGoalAction::Pause,
                objective: None,
                blocked_reason: None,
                max_goal_rounds: None,
            })?
            .goal;
            Ok(GoalCommandOutcome {
                action: "pause".to_string(),
                message: render_change("目标已暂停。自动续轮已停止。", &updated),
                goal: Some(updated),
            })
        }
        GoalCommandRequest::Resume => {
            let current = require_goal(session, "resume")?;
            let updated = update_goal(UpdateGoalArgs {
                session_id: Some(session.to_string()),
                goal_id: current.id,
                revision: current.revision,
                action: UpdateGoalAction::Resume,
                objective: None,
                blocked_reason: None,
                max_goal_rounds: None,
            })?
            .goal;
            Ok(GoalCommandOutcome {
                action: "resume".to_string(),
                message: render_change("目标已恢复。自动续轮已启动。", &updated),
                goal: Some(updated),
            })
        }
        GoalCommandRequest::Complete => {
            let current = require_goal(session, "complete")?;
            let updated = update_goal(UpdateGoalArgs {
                session_id: Some(session.to_string()),
                goal_id: current.id,
                revision: current.revision,
                action: UpdateGoalAction::Complete,
                objective: None,
                blocked_reason: None,
                max_goal_rounds: None,
            })?
            .goal;
            Ok(GoalCommandOutcome {
                action: "complete".to_string(),
                message: render_change("目标已完成。自动续轮已停止。", &updated),
                goal: Some(updated),
            })
        }
        GoalCommandRequest::Clear => {
            let removed = clear_goal(session)?;
            Ok(GoalCommandOutcome {
                action: "clear".to_string(),
                message: if removed {
                    "目标已清除。".to_string()
                } else {
                    "当前没有可清除的目标。".to_string()
                },
                goal: None,
            })
        }
    }
}

fn current_goal(session: &str) -> Result<Option<GoalData>, String> {
    Ok(get_goal(GetGoalArgs {
        session_id: Some(session.to_string()),
    })?
    .goal)
}

fn require_goal(session: &str, action: &str) -> Result<GoalData, String> {
    current_goal(session)?.ok_or_else(|| missing_goal(action))
}

/// 契约 C-2 §9：无目标时的统一拒绝文案。
fn missing_goal(action: &str) -> String {
    format!("当前未设置目标；/goal {action} 需要一个目标。\n{GOAL_COMMAND_USAGE}")
}

/// 相位中文标签（与 TUI goal banner 同口径）。
fn phase_label(phase: &str) -> &'static str {
    match phase {
        "active" => "进行中",
        "paused" => "已暂停",
        "blocked" => "已阻塞",
        "completed" => "已完成",
        _ => "未知状态",
    }
}

fn activation_label(activation: &GoalActivation) -> &'static str {
    match activation {
        GoalActivation::Armed => "armed",
        GoalActivation::Disarmed => "disarmed",
    }
}

/// 按相位的可执行命令建议。
fn phase_commands(phase: &str) -> &'static str {
    match phase {
        "active" => "/goal edit <目标> | /goal pause | /goal complete | /goal clear",
        "paused" | "blocked" => "/goal resume | /goal edit <目标> | /goal clear",
        "completed" => "/goal <新目标> | /goal clear",
        _ => GOAL_COMMAND_USAGE,
    }
}

fn render_show(goal: &GoalData) -> String {
    let mut lines = vec![
        format!("目标状态: {}", phase_label(&goal.phase)),
        format!("目标: {}", goal.objective.trim()),
        format!("轮次: {}/{}", goal.rounds_started, goal.max_goal_rounds),
        format!("激活: {}", activation_label(&goal.activation)),
    ];
    if goal.phase == "blocked" {
        if let Some(reason) = goal
            .blocked_reason
            .as_deref()
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
        {
            lines.push(format!("阻塞原因: {reason}"));
        }
    }
    lines.push(format!("可执行命令: {}", phase_commands(&goal.phase)));
    lines.join("\n")
}

fn render_change(prefix: &str, goal: &GoalData) -> String {
    format!(
        "{prefix}\n目标: {}\n轮次: {}/{}\n可执行命令: {}",
        goal.objective.trim(),
        goal.rounds_started,
        goal.max_goal_rounds,
        phase_commands(&goal.phase)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::goal::{active_goal_for_session, reset_goal_store_for_test, GOAL_STORE_PATH_ENV};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 独立性：每个用例独占临时 sidecar，Drop 时复原 env 并复位进程内缓存。
    struct StoreEnvGuard {
        previous: Option<std::ffi::OsString>,
        dir: PathBuf,
    }

    impl StoreEnvGuard {
        fn install(tag: &str) -> (Self, PathBuf) {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0);
            let dir = std::env::temp_dir().join(format!(
                "pony-goal-cmd-{tag}-{}-{nanos}",
                std::process::id()
            ));
            fs::create_dir_all(&dir).expect("create temp goal store dir");
            let store_path = dir.join("goal_store.json");
            let guard = Self {
                previous: std::env::var_os(GOAL_STORE_PATH_ENV),
                dir,
            };
            std::env::set_var(GOAL_STORE_PATH_ENV, &store_path);
            reset_goal_store_for_test();
            (guard, store_path)
        }
    }

    impl Drop for StoreEnvGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var(GOAL_STORE_PATH_ENV, value),
                None => std::env::remove_var(GOAL_STORE_PATH_ENV),
            }
            let _ = fs::remove_dir_all(&self.dir);
            reset_goal_store_for_test();
        }
    }

    fn create(session: &str, objective: &str) -> GoalData {
        create_goal(CreateGoalArgs {
            session_id: Some(session.to_string()),
            objective: objective.to_string(),
            max_goal_rounds: Some(10),
        })
        .expect("create_goal")
        .goal
    }

    #[test]
    fn parse_covers_contract_matrix() {
        // 口径 1：空 ⇒ Show。
        assert_eq!(parse_goal_command("").unwrap(), GoalCommandRequest::Show);
        assert_eq!(parse_goal_command("   ").unwrap(), GoalCommandRequest::Show);

        // 口径 2：四个无参控制命令（大小写不敏感）。
        assert_eq!(parse_goal_command("clear").unwrap(), GoalCommandRequest::Clear);
        assert_eq!(parse_goal_command("CLEAR").unwrap(), GoalCommandRequest::Clear);
        assert_eq!(parse_goal_command("Pause").unwrap(), GoalCommandRequest::Pause);
        assert_eq!(parse_goal_command("resume ").unwrap(), GoalCommandRequest::Resume);
        assert_eq!(parse_goal_command("complete").unwrap(), GoalCommandRequest::Complete);

        // 口径 2 反向：带额外 token ⇒ 整串按 Create（正文不得被吞）。
        assert_eq!(
            parse_goal_command("clear 这个需求怎么拆").unwrap(),
            GoalCommandRequest::Create {
                objective: "clear 这个需求怎么拆".to_string()
            }
        );
        assert_eq!(
            parse_goal_command("pause 一下再看").unwrap(),
            GoalCommandRequest::Create {
                objective: "pause 一下再看".to_string()
            }
        );

        // 口径 3：edit 缺描述 ⇒ Err；有描述 ⇒ Edit。
        assert_eq!(
            parse_goal_command("edit").unwrap_err(),
            "目标编辑需要提供新的目标描述"
        );
        assert_eq!(parse_goal_command("edit   ").unwrap_err(), "目标编辑需要提供新的目标描述");
        assert_eq!(
            parse_goal_command("EDIT  把报告写完 ").unwrap(),
            GoalCommandRequest::Edit {
                objective: "把报告写完".to_string()
            }
        );

        // 口径 4：其余任意文本 ⇒ Create（内部空白保留）。
        assert_eq!(
            parse_goal_command("把报告写完，并且给出 3 条结论").unwrap(),
            GoalCommandRequest::Create {
                objective: "把报告写完，并且给出 3 条结论".to_string()
            }
        );
        assert_eq!(
            parse_goal_command("  GOAL 大小写 不改变正文 ").unwrap(),
            GoalCommandRequest::Create {
                objective: "GOAL 大小写 不改变正文".to_string()
            }
        );
    }

    #[test]
    fn show_without_goal_is_ok_and_usage_bearing() {
        let _lock = crate::agent::goal::test_support::lock();
        let (_env, _path) = StoreEnvGuard::install("show-empty");
        let outcome = execute_goal_command("goal-cmd-show-empty", "").expect("show 恒 Ok");
        assert_eq!(outcome.action, "show");
        assert_eq!(
            outcome.message,
            format!("当前会话未设置目标。\n{GOAL_COMMAND_USAGE}")
        );
        assert!(outcome.goal.is_none());
    }

    #[test]
    fn show_renders_phase_rounds_and_activation() {
        let _lock = crate::agent::goal::test_support::lock();
        let (_env, _path) = StoreEnvGuard::install("show-goal");
        let session = "goal-cmd-show-goal";
        let created = create(session, "把报告写完");

        let outcome = execute_goal_command(session, "").expect("show");
        assert_eq!(outcome.action, "show");
        assert_eq!(outcome.goal.as_ref().map(|goal| goal.id.as_str()), Some(created.id.as_str()));
        assert!(outcome.message.contains("目标状态: 进行中"));
        assert!(outcome.message.contains("目标: 把报告写完"));
        assert!(outcome.message.contains("轮次: 0/10"));
        assert!(outcome.message.contains("激活: armed"));
        assert!(outcome.message.contains("可执行命令: /goal edit <目标>"));
        // 非 blocked 不渲染阻塞原因。
        assert!(!outcome.message.contains("阻塞原因"));

        let paused = execute_goal_command(session, "pause").expect("pause");
        let shown = execute_goal_command(session, "").expect("show paused");
        assert!(shown.message.contains("目标状态: 已暂停"));
        assert!(shown.message.contains("激活: disarmed"));
        assert_eq!(paused.goal.map(|goal| goal.phase), Some("paused".to_string()));
    }

    #[test]
    fn create_conflicts_with_active_goal_and_overrides_completed() {
        let _lock = crate::agent::goal::test_support::lock();
        let (_env, _path) = StoreEnvGuard::install("create-conflict");
        let session = "goal-cmd-create";

        let created = execute_goal_command(session, "把报告写完").expect("create");
        assert_eq!(created.action, "create");
        assert!(created.message.starts_with("目标已创建"));
        let created_goal = created.goal.clone().expect("created goal");

        let conflict = execute_goal_command(session, "另一个目标").unwrap_err();
        assert!(conflict.contains("当前已存在进行中目标"), "unexpected: {conflict}");
        assert!(conflict.contains("/goal edit <目标>"));
        assert!(conflict.contains("/goal clear"));

        // completed 后可覆盖新建（沿用既有预算）。
        execute_goal_command(session, "complete").expect("complete");
        let recreated = execute_goal_command(session, "新目标").expect("create after complete");
        assert_eq!(recreated.action, "create");
        let goal = recreated.goal.expect("goal");
        assert_eq!(goal.objective, "新目标");
        assert_eq!(goal.phase, "active");
        assert_eq!(goal.max_goal_rounds, created_goal.max_goal_rounds, "沿用既有预算");
        assert_ne!(goal.id, created_goal.id);
    }

    #[test]
    fn edit_requires_objective_and_existing_goal() {
        let _lock = crate::agent::goal::test_support::lock();
        let (_env, _path) = StoreEnvGuard::install("edit-missing");
        let session = "goal-cmd-edit-missing";

        assert_eq!(
            execute_goal_command(session, "edit").unwrap_err(),
            "目标编辑需要提供新的目标描述"
        );

        let missing = execute_goal_command(session, "edit 新描述").unwrap_err();
        assert_eq!(
            missing,
            format!("当前未设置目标；/goal edit 需要一个目标。\n{GOAL_COMMAND_USAGE}")
        );

        create(session, "旧描述");
        let edited = execute_goal_command(session, "edit 新描述").expect("edit");
        assert_eq!(edited.action, "edit");
        assert!(edited.message.starts_with("目标已更新"));
        assert_eq!(edited.goal.map(|goal| goal.objective), Some("新描述".to_string()));
    }

    #[test]
    fn edit_on_completed_goal_creates_new_goal() {
        let _lock = crate::agent::goal::test_support::lock();
        let (_env, _path) = StoreEnvGuard::install("edit-completed");
        let session = "goal-cmd-edit-completed";

        let created = create(session, "旧描述");
        execute_goal_command(session, "complete").expect("complete");
        let outcome = execute_goal_command(session, "edit 重开一轮").expect("edit completed");
        assert_eq!(outcome.action, "create");
        assert!(outcome.message.starts_with("目标已创建"));
        let goal = outcome.goal.expect("goal");
        assert_eq!(goal.objective, "重开一轮");
        assert_eq!(goal.phase, "active");
        assert_ne!(goal.id, created.id);
    }

    #[test]
    fn pause_resume_complete_walk_the_state_machine() {
        let _lock = crate::agent::goal::test_support::lock();
        let (_env, _path) = StoreEnvGuard::install("state-machine");
        let session = "goal-cmd-state";

        // 无目标时三个动作都必须拒绝并带 usage。
        for (args, verb) in [("pause", "pause"), ("resume", "resume"), ("complete", "complete")] {
            let err = execute_goal_command(session, args).unwrap_err();
            assert_eq!(err, format!("当前未设置目标；/goal {verb} 需要一个目标。\n{GOAL_COMMAND_USAGE}"));
        }

        create(session, "把报告写完");

        let paused = execute_goal_command(session, "pause").expect("pause");
        assert_eq!(paused.action, "pause");
        assert!(paused.message.starts_with("目标已暂停"));
        assert_eq!(paused.goal.map(|goal| goal.phase), Some("paused".to_string()));
        assert!(active_goal_for_session(session).is_none(), "paused 必须停续轮");

        let resumed = execute_goal_command(session, "resume").expect("resume");
        assert_eq!(resumed.action, "resume");
        assert!(resumed.message.starts_with("目标已恢复"));
        assert!(active_goal_for_session(session).is_some(), "resumed 必须重新武装");

        let completed = execute_goal_command(session, "complete").expect("complete");
        assert_eq!(completed.action, "complete");
        assert!(completed.message.starts_with("目标已完成"));
        assert_eq!(completed.goal.map(|goal| goal.phase), Some("completed".to_string()));
        assert!(active_goal_for_session(session).is_none());

        // B127 守卫：/goal resume 必须拒绝复活已完成目标
        let resume_err = execute_goal_command(session, "resume").unwrap_err();
        assert!(resume_err.contains("Cannot resume a completed goal"));
    }

    #[test]
    fn clear_is_idempotent_and_stops_auto_rounds() {
        let _lock = crate::agent::goal::test_support::lock();
        let (_env, _path) = StoreEnvGuard::install("clear-idempotent");
        let session = "goal-cmd-clear";

        let empty = execute_goal_command(session, "clear").expect("clear without goal");
        assert_eq!(empty.action, "clear");
        assert_eq!(empty.message, "当前没有可清除的目标。");
        assert!(empty.goal.is_none());

        create(session, "把报告写完");
        let cleared = execute_goal_command(session, "clear").expect("clear");
        assert_eq!(cleared.action, "clear");
        assert_eq!(cleared.message, "目标已清除。");
        assert!(cleared.goal.is_none());
        assert!(current_goal(session).expect("get_goal").is_none());
        assert!(active_goal_for_session(session).is_none(), "clear 后续轮必须立即停");

        // 幂等：再清一次仍 Ok，不落盘也不报错。
        let again = execute_goal_command(session, "clear").expect("clear twice");
        assert_eq!(again.message, "当前没有可清除的目标。");
    }

    #[test]
    fn clear_persist_failure_rolls_back_memory_record() {
        let _lock = crate::agent::goal::test_support::lock();
        let (env_guard, store_path) = StoreEnvGuard::install("clear-rollback");
        let session = "goal-cmd-clear-rollback";
        create(session, "把报告写完");

        // 让落盘必然失败：把 store 路径的父目录换成一个普通文件。
        let blocker = env_guard.dir.join("blocker");
        fs::write(&blocker, b"not-a-directory").expect("write blocker file");
        std::env::set_var(GOAL_STORE_PATH_ENV, blocker.join("goal_store.json"));

        let error = clear_goal(session).expect_err("persist 失败必须原样向上传播");
        assert!(
            error.contains("failed to create goal store directory"),
            "unexpected error: {error}"
        );

        // 内存记录必须被放回：不得出现「报错但内存已删」。
        assert!(
            current_goal(session).expect("get_goal").is_some(),
            "落盘失败后必须回滚内存记录"
        );

        // 恢复可写路径后重试成功：内存记录消失、续轮停。
        std::env::set_var(GOAL_STORE_PATH_ENV, &store_path);
        let cleared = execute_goal_command(session, "clear").expect("clear after restore");
        assert_eq!(cleared.message, "目标已清除。");
        assert!(current_goal(session).expect("get_goal").is_none());
        assert!(active_goal_for_session(session).is_none());
    }
}
