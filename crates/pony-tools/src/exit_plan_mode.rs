//! `ExitPlanMode`：Plan 模式的**唯一**退出审查入口（对齐 DSH `exit_plan_mode`）。
//!
//! 语义（不是任务清单，见 [`crate::plan_mode`]）：
//!
//! 1. 仅在 Plan 模式生效时可调用；未激活时 fail-closed 为 `plan_mode_inactive`，
//!    避免模型把「写方案」当成绕过写门禁的通用工具。
//! 2. 入参只有 `plan`（完整 Markdown 方案，必须以 `# ` 一级标题开头），
//!    与 DSH 一致：拒绝空方案与无标题方案，防止用一句「我打算这样」换解锁。
//! 3. 调用即把方案登记到会话状态，返回 `awaitingReview: true`；**不在此处**判定批准——
//!    批准由宿主 Ask 通道（`WaitingHost` 控制请求）回填，见
//!    `pony_agent_core::agent::control_plane` 的审核落地点。
//!
//! 该 handler 只做「登记 + 校验 + 回执」，不做任何文件写入与命令执行。

use serde_json::{Value, json};

use crate::plan_mode::{self, PLAN_MODE_POLICY, PLAN_REVIEW_APPROVE, PLAN_REVIEW_KEEP_PLANNING};
use crate::tool_runtime::{PrimitiveToolHandler, PrimitiveToolHandlerRequest};

/// `plan_control` primitive 的模型可见工具名（槽位不变，语义换成审查退出）。
pub const EXIT_PLAN_MODE_NAME: &str = "ExitPlanMode";

/// 审核失败的错误码：模型必须在 Plan 模式下才能调用本工具。
pub const CODE_MODE_INACTIVE: &str = "plan_mode_inactive";
/// 审核失败的错误码：方案正文不合法（空 / 无一级标题）。
pub const CODE_INVALID_PLAN: &str = "invalid_plan";
/// 审核失败的错误码：dispatch 未注入权威会话。
pub const CODE_MISSING_SESSION: &str = "missing_session";

/// `ExitPlanMode` handler：登记方案并返回待审回执。
#[derive(Clone, Debug, Default)]
pub struct ExitPlanModeHandler;

impl ExitPlanModeHandler {
    pub fn new() -> Self {
        Self
    }

    /// 校验并登记方案。`session_id` 必须来自 dispatch context（模型实参不是权威会话）。
    pub fn submit(&self, session_id: Option<&str>, arguments: &Value) -> Result<Value, String> {
        let Some(session_id) = session_id.map(str::trim).filter(|id| !id.is_empty()) else {
            return Err(format!(
                "{CODE_MISSING_SESSION}: ExitPlanMode needs the session injected by the dispatch context"
            ));
        };
        if !plan_mode::is_active(session_id) {
            return Err(format!(
                "{CODE_MODE_INACTIVE}: ExitPlanMode is only available in plan mode (use /plan to enter)"
            ));
        }
        let plan = arguments
            .get("plan")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if !plan.starts_with('#') || plan.lines().next().map(str::trim) == Some("#") {
            return Err(format!(
                "{CODE_INVALID_PLAN}: ExitPlanMode requires a non-empty markdown plan starting with a `# ` heading"
            ));
        }

        let state = plan_mode::submit_plan(session_id, plan);
        Ok(json!({
            "awaitingReview": true,
            "approved": false,
            "plan": state.plan,
            "revision": state.revision,
            "options": [PLAN_REVIEW_APPROVE, PLAN_REVIEW_KEEP_PLANNING],
            "policy": PLAN_MODE_POLICY,
        }))
    }
}

impl PrimitiveToolHandler for ExitPlanModeHandler {
    fn execute(&self, request: &PrimitiveToolHandlerRequest) -> Result<Value, String> {
        self.submit(request.session_id.as_deref(), &request.arguments)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolate() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let dir = std::env::temp_dir().join(format!(
                "pony-agent-exit-plan-mode-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default()
            ));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            std::env::set_var(
                "PONY_AGENT_PLAN_MODE_STORE_PATH",
                dir.join("plan_mode.json"),
            );
        });
    }

    /// 每例唯一会话 id（进程级注册表的并行隔离手段；不要清空注册表，会踩别的测试）。
    fn session_id(label: &str) -> String {
        format!("{label}-{}", uuid::Uuid::new_v4())
    }

    #[test]
    fn submit_is_rejected_while_plan_mode_is_inactive() {
        isolate();
        let err = ExitPlanModeHandler::new()
            .submit(Some("s1"), &json!({"plan": "# 方案"}))
            .expect_err("非 Plan 模式必须 fail-closed");
        assert!(err.starts_with(CODE_MODE_INACTIVE), "{err}");
    }

    #[test]
    fn submit_requires_the_dispatch_injected_session() {
        isolate();
        let s2 = session_id("s2");
        crate::plan_mode::set_active(&s2, crate::plan_mode::ModeToggle::Enter);
        let err = ExitPlanModeHandler::new()
            .submit(None, &json!({"plan": "# 方案"}))
            .expect_err("无权威会话必须 fail-closed");
        assert!(err.starts_with(CODE_MISSING_SESSION), "{err}");
    }

    #[test]
    fn submit_rejects_empty_or_headingless_plans() {
        isolate();
        let s3 = session_id("s3");
        crate::plan_mode::set_active(&s3, crate::plan_mode::ModeToggle::Enter);
        let handler = ExitPlanModeHandler::new();

        for bad in [json!({}), json!({"plan": "   "}), json!({"plan": "随便写写"}), json!({"plan": "#"})] {
            let err = handler
                .submit(Some(&s3), &bad)
                .expect_err("非法方案必须被拒绝");
            assert!(err.starts_with(CODE_INVALID_PLAN), "{bad} => {err}");
        }
    }

    #[test]
    fn submit_registers_the_plan_and_requests_review_without_closing_the_mode() {
        isolate();
        let s4 = session_id("s4");
        crate::plan_mode::set_active(&s4, crate::plan_mode::ModeToggle::Enter);

        let out = ExitPlanModeHandler::new()
            .submit(Some(&s4), &json!({"plan": "# 方案\n- 步骤"}))
            .expect("合法方案必须被接受");

        assert_eq!(out["awaitingReview"], json!(true));
        assert_eq!(
            out["approved"],
            json!(false),
            "批准只能由宿主 Ask 回填，工具自身不得声称已批准"
        );
        assert_eq!(out["options"], json!([PLAN_REVIEW_APPROVE, PLAN_REVIEW_KEEP_PLANNING]));
        let state = crate::plan_mode::plan_mode_state(&s4);
        assert!(state.active, "提交方案不等于退出模式");
        assert_eq!(state.plan.as_deref(), Some("# 方案\n- 步骤"));
    }
}
