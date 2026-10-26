//! 用量治理策略与决策引擎（Wave-7 B002，契约 G2 / G3）。
//!
//! 设计边界：
//! - **决策核心是纯函数**（[`decide`]）：无 IO、无全局态、可独立单测；
//! - 策略的**存放**是进程级设置桥接（[`set_policy`] / [`policy`]），与 OTel 上报开关、
//!   成本预算同构（设置读写路径注入，核心只读）；
//! - 账本读取与故障降级在 [`evaluate`] 层面完成（fail-open，见 G3）。
//!
//! 阻断语义（ADR-0048 / 契约 K3.1 supersede）：
//! - `enforcement = false`（**默认**）⇒ 永不 Block，只 `Warn`（`would_block` 仍为 true 供 UI 诊断）；
//! - `enforcement = true` 且触及硬限额 ⇒ `Block`，但**只在新 turn 发起前**生效，
//!   绝不中断进行中的 turn（由调用点保证：只在 turn 入口判定一次）。

use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};

/// 结算窗口。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyWindow {
    /// 会话级（整个 session 的全部历史）。
    Session,
    /// 天级（UTC 天桶）。
    Day,
}

impl Default for PolicyWindow {
    fn default() -> Self {
        Self::Session
    }
}

/// 治理策略（持久化于 settings，camelCase 下发；见契约 G2.1）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsagePolicy {
    #[serde(default)]
    pub window: PolicyWindow,
    /// 软告警比例（如 0.8）。`None` / `<= 0` / 非法 ⇒ 未配置。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft_ratio: Option<f64>,
    /// 硬限额（USD）。`None` / `<= 0` / 非法 ⇒ 未配置。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hard_limit_usd: Option<f64>,
    /// 是否物理拦截（**默认 false**：只有用户显式 opt-in 才阻断新 turn）。
    #[serde(default)]
    pub enforcement: bool,
}

impl Default for UsagePolicy {
    fn default() -> Self {
        Self {
            window: PolicyWindow::Session,
            soft_ratio: None,
            hard_limit_usd: None,
            enforcement: false,
        }
    }
}

impl UsagePolicy {
    /// 当前窗口内的已计价成本（快照取值）。
    pub fn window_usage(&self, snapshot: &UsageSnapshot) -> f64 {
        match self.window {
            PolicyWindow::Session => snapshot.session_cost_usd,
            PolicyWindow::Day => snapshot.day_cost_usd,
        }
    }

    /// 策略是否合法（脏数据防御 G2.2）。
    ///
    /// 非法判据：任一已配置数值为 `NaN` / `±Inf` / 负值；或 `soft_ratio` 非有限。
    pub fn is_valid(&self) -> bool {
        let valid_optional = |value: Option<f64>| match value {
            None => true,
            Some(raw) => raw.is_finite() && raw >= 0.0,
        };
        valid_optional(self.soft_ratio) && valid_optional(self.hard_limit_usd)
    }

    /// 策略版本（用于 Issue fingerprint 去重与 span 属性）。
    ///
    /// 语义：对**稳定维度**做 SHA-256 取前 12 位 hex —— 相同策略（含 window/阈值/开关）
    /// 恒得同一版本号；与具体金额无关（G5.3 禁止动态数字进指纹）。
    pub fn version(&self) -> String {
        use sha2::{Digest, Sha256};
        let canonical = format!(
            "{}|{}|{}|{}",
            match self.window {
                PolicyWindow::Session => "session",
                PolicyWindow::Day => "day",
            },
            self.soft_ratio
                .map(|value| format!("{value:.6}"))
                .unwrap_or_else(|| "none".to_string()),
            self.hard_limit_usd
                .map(|value| format!("{value:.6}"))
                .unwrap_or_else(|| "none".to_string()),
            self.enforcement,
        );
        let digest = Sha256::digest(canonical.as_bytes());
        digest[..6].iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

/// 用量快照（账本聚合结果；由调用方从 `usage_ledger` 读取）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UsageSnapshot {
    pub session_cost_usd: f64,
    pub day_cost_usd: f64,
    pub unpriced_turns: u64,
}

/// 决策输入（纯函数入参）。
#[derive(Debug, Clone, PartialEq)]
pub struct DecideInput<'a> {
    pub session_id: &'a str,
    pub day_key: &'a str,
    pub policy: &'a UsagePolicy,
    pub snapshot: &'a UsageSnapshot,
}

/// 治理决策（契约 G2.1 形状）。
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Allow {
        would_block: bool,
    },
    Warn {
        ratio: f64,
        would_block: bool,
    },
    Block {
        reason: String,
        current: f64,
        limit: f64,
        would_block: bool,
    },
}

impl Decision {
    /// 是否应拒绝创建新 turn（只有 `Block` 为 true）。
    pub fn is_block(&self) -> bool {
        matches!(self, Decision::Block { .. })
    }

    /// 稳定标签（span `governance.decision` 用）：`allow` / `warn` / `block`。
    pub fn label(&self) -> &'static str {
        match self {
            Decision::Allow { .. } => "allow",
            Decision::Warn { .. } => "warn",
            Decision::Block { .. } => "block",
        }
    }

    /// 诊断字段 `would_block`（硬限额是否已被触及）。
    pub fn would_block(&self) -> bool {
        match self {
            Decision::Allow { would_block }
            | Decision::Warn { would_block, .. }
            | Decision::Block { would_block, .. } => *would_block,
        }
    }
}

/// 决策结果（含降级标记与策略版本；契约 G2 的 `Decision` 保持纯净）。
#[derive(Debug, Clone, PartialEq)]
pub struct GovernanceOutcome {
    pub decision: Decision,
    /// fail-open 降级（账本不可读 / 策略非法）；调用方需透出 `governance_degraded`。
    pub governance_degraded: bool,
    pub policy_version: String,
}

impl GovernanceOutcome {
    fn allow_golden(policy_version: String) -> Self {
        Self {
            decision: Decision::Allow { would_block: false },
            governance_degraded: false,
            policy_version,
        }
    }
}

/// 单一硬限额是否触及（`>=` 含等值）—— 纯函数辅助。
pub fn would_block(policy: &UsagePolicy, snapshot: &UsageSnapshot) -> bool {
    if !policy.is_valid() {
        return false;
    }
    match policy.hard_limit_usd {
        Some(limit) if limit > 0.0 => policy.window_usage(snapshot) >= limit,
        _ => false,
    }
}

/// **决策核心（纯函数）**：无 IO、无全局态。
///
/// 规则（契约 G2.2）：
/// 1. 策略非法 ⇒ `Allow { would_block: false }` + `governance_degraded = true`；
/// 2. 硬限额已配置且 `usage >= limit`：
///    - `enforcement == true` ⇒ `Block`；
///    - `enforcement == false`（默认）⇒ `Warn { would_block: true }`（**不阻断**）；
/// 3. 否则若软阈值已配置且 `usage / limit >= soft_ratio` ⇒ `Warn { would_block: false }`；
/// 4. 其余 ⇒ `Allow`。
pub fn decide(input: &DecideInput<'_>) -> GovernanceOutcome {
    let policy_version = input.policy.version();
    if !input.policy.is_valid() {
        return GovernanceOutcome {
            decision: Decision::Allow { would_block: false },
            governance_degraded: true,
            policy_version,
        };
    }

    let usage = input.policy.window_usage(input.snapshot);
    if !usage.is_finite() || usage < 0.0 {
        // 快照脏数据同样按降级处理（G3：静默放行）。
        return GovernanceOutcome {
            decision: Decision::Allow { would_block: false },
            governance_degraded: true,
            policy_version,
        };
    }

    // 2) 硬限额
    if let Some(limit) = input.policy.hard_limit_usd.filter(|value| *value > 0.0) {
        if usage >= limit {
            let ratio = usage / limit;
            if input.policy.enforcement {
                return GovernanceOutcome {
                    decision: Decision::Block {
                        reason: "hard_limit_reached".to_string(),
                        current: usage,
                        limit,
                        would_block: true,
                    },
                    governance_degraded: false,
                    policy_version,
                };
            }
            return GovernanceOutcome {
                decision: Decision::Warn {
                    ratio,
                    would_block: true,
                },
                governance_degraded: false,
                policy_version,
            };
        }
        // 3) 软阈值（未触及硬限额时）
        if let Some(soft_ratio) = input.policy.soft_ratio.filter(|value| *value > 0.0) {
            let ratio = usage / limit;
            if ratio >= soft_ratio {
                return GovernanceOutcome {
                    decision: Decision::Warn {
                        ratio,
                        would_block: false,
                    },
                    governance_degraded: false,
                    policy_version,
                };
            }
        }
    }

    // 4) 无限制 / 未达阈值
    GovernanceOutcome::allow_golden(policy_version)
}

/// Fail-open 评估入口（契约 G3）：账本读取失败/策略非法一律 `Allow` + `degraded`。
///
/// 语义：`snapshot` 为调用方读取账本的结果（`Err` 表示 SQLite 不可读/锁超时/SQL 异常）。
/// 降级时输出结构化 warn，并按 `(kind, policy_version)` 去重上报 PonySentry（不含金额/阈值）。
pub fn evaluate(
    snapshot: Result<UsageSnapshot, String>,
    policy: &UsagePolicy,
) -> GovernanceOutcome {
    let policy_version = policy.version();
    match snapshot {
        Ok(snapshot) => decide(&DecideInput {
            session_id: "",
            day_key: "",
            policy,
            snapshot: &snapshot,
        }),
        Err(error) => {
            report_degraded("ledger_unreadable", &policy_version, &error);
            GovernanceOutcome {
                decision: Decision::Allow { would_block: false },
                governance_degraded: true,
                policy_version,
            }
        }
    }
}

/// 窗口标签（span/log 用）。
pub fn window_label(policy: &UsagePolicy) -> &'static str {
    match policy.window {
        PolicyWindow::Session => "session",
        PolicyWindow::Day => "day",
    }
}

/// 从账本读取当前用量快照（只读）。
///
/// `session_id` 缺省（无会话）时，session 维度视为 0（只看天窗口）。
pub fn snapshot_for_store(
    store: &crate::agent::session::SessionStore,
    session_id: Option<&str>,
    window: PolicyWindow,
) -> Result<UsageSnapshot, String> {
    use crate::agent::usage_ledger::{day_key_for_epoch_ms, now_epoch_ms, LedgerScope};
    let day_key = day_key_for_epoch_ms(now_epoch_ms());
    let session_summary = match session_id.filter(|value| !value.trim().is_empty()) {
        Some(session_id) => store.usage_ledger_window_summary(&LedgerScope::Session(
            session_id.to_string(),
        ))?,
        None => crate::agent::usage_ledger::WindowSummary::zero(),
    };
    let day_summary = store.usage_ledger_window_summary(&LedgerScope::Day(day_key))?;
    // L3 ADVISORY 修复：`session` 行天然包含在 `day` 行内，二者相加会重复计数；
    // 改为**按所判窗口取对应值**（Session ⇒ 会话历史；Day ⇒ 当天桶）。
    let unpriced_turns = match window {
        PolicyWindow::Session => session_summary.unpriced_turns,
        PolicyWindow::Day => day_summary.unpriced_turns,
    };
    Ok(UsageSnapshot {
        session_cost_usd: session_summary.cost_usd,
        day_cost_usd: day_summary.cost_usd,
        unpriced_turns,
    })
}

/// 宏观决策入口（B003 接入点）：读账本 → 纯函数决策 → span/日志。
///
/// 契约 G3：账本不可读/策略非法 ⇒ `Allow` + `governance_degraded = true`（fail-open），
/// 绝不因治理故障阻断 turn。
/// 契约 G5：仅在**本入口**建立 `governance.decide` span（属性白名单 5 项），
/// 纯函数 [`decide`] 内不加任何 span。
pub fn evaluate_with_store(
    store: &crate::agent::session::SessionStore,
    session_id: Option<&str>,
) -> GovernanceOutcome {
    let policy = policy();
    let snapshot = snapshot_for_store(store, session_id, policy.window);
    // L3 ADVISORY 修复：span 必须写**真实**未计价 turn 数（此前恒 0 属 telemetry 说谎）。
    let unpriced_turns = snapshot
        .as_ref()
        .map(|snapshot| snapshot.unpriced_turns)
        .unwrap_or(0);
    let outcome = evaluate(snapshot, &policy);

    // G5.1：宏观决策 span（白名单属性）。
    let mut span = crate::agent::otel::start_governance_decide_span();
    let reason = match &outcome.decision {
        Decision::Block { reason, .. } => Some(reason.as_str()),
        _ => None,
    };
    span.record_verdict(
        window_label(&policy),
        outcome.decision.label(),
        reason,
        &outcome.policy_version,
        unpriced_turns,
    );
    drop(span);

    // G5.2：只在 warn / block / 降级时写结构化日志。
    match &outcome.decision {
        Decision::Warn { ratio, would_block } => {
            crate::agent::otel::log::log_json_with_fields(
                "warn",
                "usage governance warn",
                None,
                &[
                    ("kind", serde_json::json!("governance_warn")),
                    ("window", serde_json::json!(window_label(&policy))),
                    ("ratio", serde_json::json!(*ratio)),
                    ("would_block", serde_json::json!(*would_block)),
                    ("policy_version", serde_json::json!(outcome.policy_version)),
                ],
            );
        }
        Decision::Block { reason, .. } => {
            crate::agent::otel::log::log_json_with_fields(
                "warn",
                "usage governance block (new turn rejected)",
                None,
                &[
                    ("kind", serde_json::json!("governance_block")),
                    ("window", serde_json::json!(window_label(&policy))),
                    ("reason", serde_json::json!(reason)),
                    ("policy_version", serde_json::json!(outcome.policy_version)),
                ],
            );
        }
        Decision::Allow { .. } => {}
    }
    outcome
}

// ------------------------------------------------------- G4 汇总查询面（纯只读）

/// 会话维度的治理汇总。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageGovernanceSessionView {
    pub cost_usd: f64,
    pub unpriced_turns: u64,
}

/// 天桶维度的治理汇总。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageGovernanceDayView {
    pub day_key: String,
    pub cost_usd: f64,
    pub unpriced_turns: u64,
    pub turns: u64,
}

/// 治理汇总查询结果（契约 G4.1 形状，camelCase 下发）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageGovernanceSummaryView {
    pub session: UsageGovernanceSessionView,
    pub days: Vec<UsageGovernanceDayView>,
    pub policy_version: String,
    pub governance_degraded: bool,
}

/// 只读聚合账本生成汇总（G4）。
///
/// 语义：空库返回零值结构（**禁止报错**）；查询失败 ⇒ 降级（`governance_degraded = true`）
/// 并返回已取得的部分数据，绝不抛出/阻断调用方。
pub fn summary_for_store(
    store: &crate::agent::session::SessionStore,
    session_id: Option<&str>,
) -> UsageGovernanceSummaryView {
    use crate::agent::usage_ledger::LedgerScope;
    let mut degraded = false;
    let session = match session_id.filter(|value| !value.trim().is_empty()) {
        Some(session_id) => match store
            .usage_ledger_window_summary(&LedgerScope::Session(session_id.to_string()))
        {
            Ok(summary) => UsageGovernanceSessionView {
                cost_usd: summary.cost_usd,
                unpriced_turns: summary.unpriced_turns,
            },
            Err(error) => {
                degraded = true;
                report_degraded("summary_session_failed", &policy().version(), &error);
                UsageGovernanceSessionView::default()
            }
        },
        None => UsageGovernanceSessionView::default(),
    };
    let days = match store.usage_ledger_day_summaries(session_id) {
        Ok(days) => days
            .into_iter()
            .map(|day| UsageGovernanceDayView {
                day_key: day.day_key,
                cost_usd: day.cost_usd,
                unpriced_turns: day.unpriced_turns,
                turns: day.turns,
            })
            .collect(),
        Err(error) => {
            degraded = true;
            report_degraded("summary_days_failed", &policy().version(), &error);
            Vec::new()
        }
    };
    UsageGovernanceSummaryView {
        session,
        days,
        policy_version: policy().version(),
        governance_degraded: degraded,
    }
}

/// 治理降级上报：结构化 warn + 去重 PonySentry Issue（fingerprint = `(kind, policy_version)`）。
///
/// 约束（G5.3）：payload/指纹**绝不**包含金额、阈值、token 数等动态数字。
pub fn report_degraded(kind: &str, policy_version: &str, detail: &str) {
    crate::agent::otel::log::log_json_with_fields(
        "warn",
        "usage governance degraded (fail-open allow)",
        None,
        &[
            ("kind", serde_json::json!(kind)),
            ("policy_version", serde_json::json!(policy_version)),
            ("error", serde_json::json!(detail)),
        ],
    );
    crate::agent::ponysentry::capture_governance_fault(kind, policy_version, detail);
}

// ------------------------------------------------------- 进程级策略（设置桥接，非决策态）

/// 进程级策略槽：`None` ⇒ 未配置（等价 `UsagePolicy::default()`，永不阻断）。
static POLICY: OnceLock<Mutex<Option<UsagePolicy>>> = OnceLock::new();

fn policy_slot() -> &'static Mutex<Option<UsagePolicy>> {
    POLICY.get_or_init(|| Mutex::new(None))
}

/// 设置生效策略（宿主设置读写路径调用；`None` 复位为默认）。
pub fn set_policy(policy: Option<UsagePolicy>) {
    let mut guard = policy_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = policy;
}

/// 读取当前生效策略（未配置 ⇒ `UsagePolicy::default()`，enforcement=false）。
pub fn policy() -> UsagePolicy {
    policy_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(limit: Option<f64>, soft: Option<f64>, enforcement: bool) -> UsagePolicy {
        UsagePolicy {
            window: PolicyWindow::Session,
            soft_ratio: soft,
            hard_limit_usd: limit,
            enforcement,
        }
    }

    fn snapshot(cost: f64) -> UsageSnapshot {
        UsageSnapshot {
            session_cost_usd: cost,
            day_cost_usd: cost,
            unpriced_turns: 0,
        }
    }

    fn run(policy: &UsagePolicy, cost: f64) -> GovernanceOutcome {
        let snapshot = snapshot(cost);
        decide(&DecideInput {
            session_id: "sess",
            day_key: "2026-10-25",
            policy,
            snapshot: &snapshot,
        })
    }

    /// G2 黄金表 1：未设置限制 ⇒ Allow。
    #[test]
    fn golden_1_no_limits_allows() {
        let outcome = run(&policy(None, None, false), 7.0);
        assert_eq!(outcome.decision, Decision::Allow { would_block: false });
        assert!(!outcome.governance_degraded);
    }

    /// 黄金表 2：低于软阈值 ⇒ Allow。
    #[test]
    fn golden_2_below_soft_allows() {
        let outcome = run(&policy(Some(10.0), Some(0.8), false), 7.0);
        assert_eq!(outcome.decision, Decision::Allow { would_block: false });
    }

    /// 黄金表 3：恰好触及软阈值 ⇒ Warn(0.8)。
    #[test]
    fn golden_3_equal_soft_warns() {
        let outcome = run(&policy(Some(10.0), Some(0.8), false), 8.0);
        match outcome.decision {
            Decision::Warn { ratio, would_block } => {
                assert!((ratio - 0.8).abs() < 1e-9, "ratio 必须为 0.8，实际 {ratio}");
                assert!(!would_block);
            }
            other => panic!("期望 Warn，实际 {other:?}"),
        }
    }

    /// 黄金表 4：软硬之间 ⇒ Warn(0.95)。
    #[test]
    fn golden_4_between_soft_and_hard_warns() {
        let outcome = run(&policy(Some(10.0), Some(0.8), false), 9.5);
        match outcome.decision {
            Decision::Warn { ratio, would_block } => {
                assert!((ratio - 0.95).abs() < 1e-9);
                assert!(!would_block);
            }
            other => panic!("期望 Warn，实际 {other:?}"),
        }
    }

    /// 黄金表 5：触及硬限额但未开强制 ⇒ Warn + would_block=true（不阻断）。
    #[test]
    fn golden_5_hard_limit_without_enforcement_warns_not_blocks() {
        let outcome = run(&policy(Some(10.0), None, false), 10.0);
        match outcome.decision {
            Decision::Warn { ratio, would_block } => {
                assert!((ratio - 1.0).abs() < 1e-9);
                assert!(would_block, "触及硬限额时 would_block 必须为 true（诊断用）");
            }
            other => panic!("期望 Warn，实际 {other:?}"),
        }
    }

    /// 黄金表 6：远超硬限额但未开强制 ⇒ Warn(1.5)。
    #[test]
    fn golden_6_far_over_without_enforcement_still_warns() {
        let outcome = run(&policy(Some(10.0), None, false), 15.0);
        match outcome.decision {
            Decision::Warn { ratio, would_block } => {
                assert!((ratio - 1.5).abs() < 1e-9);
                assert!(would_block);
            }
            other => panic!("期望 Warn，实际 {other:?}"),
        }
    }

    /// 黄金表 7：触及硬限额且开启强制 ⇒ Block。
    #[test]
    fn golden_7_hard_limit_with_enforcement_blocks() {
        let outcome = run(&policy(Some(10.0), None, true), 10.0);
        match outcome.decision {
            Decision::Block { current, limit, would_block, .. } => {
                assert_eq!(current, 10.0);
                assert_eq!(limit, 10.0);
                assert!(would_block);
            }
            other => panic!("期望 Block，实际 {other:?}"),
        }
    }

    /// 黄金表 8：脏数据策略（NaN / 负值）⇒ Allow + degraded。
    #[test]
    fn golden_8_invalid_policy_degrades_to_allow() {
        for invalid in [
            policy(Some(f64::NAN), Some(0.8), true),
            policy(Some(-5.0), None, true),
            policy(Some(10.0), Some(f64::INFINITY), true),
            policy(Some(10.0), Some(-0.5), true),
        ] {
            let outcome = run(&invalid, 100.0);
            assert_eq!(outcome.decision, Decision::Allow { would_block: false });
            assert!(outcome.governance_degraded, "非法策略必须标记 degraded");
        }
    }

    /// 黄金表 9：单调性——成本递增时状态只能 Allow → Warn → Block。
    #[test]
    fn golden_9_decisions_are_monotonic() {
        let rank = |decision: &Decision| match decision {
            Decision::Allow { .. } => 0,
            Decision::Warn { .. } => 1,
            Decision::Block { .. } => 2,
        };
        for enforcement in [false, true] {
            let policy = policy(Some(10.0), Some(0.8), enforcement);
            let mut last_rank = 0;
            let mut cost = 0.0;
            while cost <= 20.0 {
                let outcome = run(&policy, cost);
                let rank = rank(&outcome.decision);
                assert!(
                    rank >= last_rank,
                    "enforcement={enforcement} cost={cost} 出现非单调回跳：{last_rank} -> {rank}"
                );
                last_rank = rank;
                cost += 0.25;
            }
            assert_eq!(last_rank, if enforcement { 2 } else { 1 });
        }
    }

    /// enforcement=false 恒不返回 Block（无论多高成本）。
    #[test]
    fn enforcement_false_never_blocks() {
        for cost in [0.0, 5.0, 10.0, 1_000_000.0] {
            let outcome = run(&policy(Some(10.0), Some(0.8), false), cost);
            assert!(
                !outcome.decision.is_block(),
                "enforcement=false 时绝不允许阻断（cost={cost}）"
            );
        }
    }

    /// 窗口语义：Day 窗口只看当天成本。
    #[test]
    fn day_window_uses_day_cost_only() {
        let policy = UsagePolicy {
            window: PolicyWindow::Day,
            soft_ratio: None,
            hard_limit_usd: Some(1.0),
            enforcement: true,
        };
        let snapshot = UsageSnapshot {
            session_cost_usd: 100.0,
            day_cost_usd: 0.5,
            unpriced_turns: 0,
        };
        let outcome = decide(&DecideInput {
            session_id: "sess",
            day_key: "2026-10-25",
            policy: &policy,
            snapshot: &snapshot,
        });
        assert_eq!(outcome.decision, Decision::Allow { would_block: false });
    }

    /// 治理相关全局态（故障指纹表）串行锁：避免并行用例互相污染计数。
    static GOVERNANCE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn governance_test_lock() -> std::sync::MutexGuard<'static, ()> {
        GOVERNANCE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// G3 判据 1：账本不可读 ⇒ Allow + degraded（fail-open）。
    #[test]
    fn g3_unreadable_ledger_fails_open() {
        let _guard = governance_test_lock();
        let outcome = evaluate(
            Err("SQLITE_BUSY: database is locked".to_string()),
            &policy(Some(1.0), None, true),
        );
        assert_eq!(outcome.decision, Decision::Allow { would_block: false });
        assert!(outcome.governance_degraded);
    }

    /// G3 判据 2：同一 `(kind, policy_version)` 多次故障只产生**一个** Sentry 指纹。
    #[test]
    fn g3_governance_faults_are_deduped_by_fingerprint() {
        let _guard = governance_test_lock();
        let policy_version = policy(Some(10.0), None, true).version();
        // 用独占 kind 前缀，避免与其它用例的 (kind, version) 交叉。
        let kind = "probe_fault_dedupe";
        let before = crate::agent::ponysentry::governance_fault_fingerprint_count();
        report_degraded(kind, &policy_version, "SQLITE_BUSY");
        let after_first = crate::agent::ponysentry::governance_fault_fingerprint_count();
        assert_eq!(after_first - before, 1, "首次必须新增 1 个指纹");
        report_degraded(kind, &policy_version, "SQLITE_BUSY");
        report_degraded(kind, &policy_version, "corrupted");
        assert_eq!(
            crate::agent::ponysentry::governance_fault_fingerprint_count(),
            after_first,
            "同一 (kind, policy_version) 重复故障必须收敛（不再新增指纹）"
        );
        // 不同 kind ⇒ 另一个指纹。
        report_degraded("probe_fault_dedupe_other", &policy_version, "ddl failed");
        assert_eq!(
            crate::agent::ponysentry::governance_fault_fingerprint_count(),
            after_first + 1
        );
    }

    /// G4 判据：空库零值结构；写 2 条已计价 + 1 条未计价后 `unpriced_turns == 1` 且金额为前两条之和。
    #[test]
    fn g4_summary_aggregates_ledger_and_counts_unpriced() {
        use crate::agent::session::{SessionStore, SeparateTraceTableMode};
        use crate::agent::usage_ledger::LedgerEntry;

        let _guard = governance_test_lock();
        let dir = std::env::temp_dir().join(format!(
            "pony-gov-summary-{}-{}",
            std::process::id(),
            crate::agent::usage_ledger::now_epoch_ms()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let store = SessionStore::with_backend(Box::new(
            crate::agent::sqlite_session::SqliteSessionBackend::new_with_trace_mode(
                dir.join("summary.db"),
                SeparateTraceTableMode::WriteSeparate,
            ),
        ));

        // 空库：零值结构 + 不降级 + 不报错。
        let empty = summary_for_store(&store, Some("sess-gov"));
        assert_eq!(empty.session.cost_usd, 0.0);
        assert_eq!(empty.session.unpriced_turns, 0);
        assert!(empty.days.is_empty());
        assert!(!empty.governance_degraded);

        let now = crate::agent::usage_ledger::now_epoch_ms();
        for (turn, cost) in [("turn-1", 0.0125), ("turn-2", 0.0075)] {
            store
                .append_usage_ledger_entry(&LedgerEntry::priced(
                    "sess-gov",
                    turn,
                    now,
                    "openai",
                    "gpt-4o",
                    "openai-completions",
                    100,
                    50,
                    0,
                    cost,
                ))
                .expect("append priced");
        }
        store
            .append_usage_ledger_entry(&LedgerEntry::unpriced(
                "sess-gov",
                "turn-3",
                now,
                "openai",
                "unknown-model",
                "openai-completions",
                10,
                5,
                0,
            ))
            .expect("append unpriced");

        let summary = summary_for_store(&store, Some("sess-gov"));
        assert_eq!(summary.session.cost_usd, 0.02, "金额必须为 2 条已计价之和（6 位小数内）");
        assert_eq!(summary.session.unpriced_turns, 1, "未计价 turn 必须单独计数");
        assert_eq!(summary.days.len(), 1);
        assert_eq!(summary.days[0].turns, 3);
        assert_eq!(summary.days[0].unpriced_turns, 1);
        assert!(!summary.governance_degraded);
        assert_eq!(summary.policy_version.len(), 12);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L3 ADVISORY 2：`unpriced_turns` 必须按**所判窗口**取值，不得 session+day 重复计数。
    #[test]
    fn snapshot_unpriced_turns_follows_window_without_double_counting() {
        use crate::agent::session::{SeparateTraceTableMode, SessionStore};
        use crate::agent::usage_ledger::LedgerEntry;

        let _guard = governance_test_lock();
        let dir = std::env::temp_dir().join(format!(
            "pony-gov-window-{}-{}",
            std::process::id(),
            crate::agent::usage_ledger::now_epoch_ms()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let store = SessionStore::with_backend(Box::new(
            crate::agent::sqlite_session::SqliteSessionBackend::new_with_trace_mode(
                dir.join("window.db"),
                SeparateTraceTableMode::WriteSeparate,
            ),
        ));

        let now = crate::agent::usage_ledger::now_epoch_ms();
        let yesterday = now - 86_400_000;
        // 今天 1 条未计价 + 昨天 1 条未计价（同一会话）。
        for (turn, at) in [("turn-today", now), ("turn-yesterday", yesterday)] {
            let mut entry = LedgerEntry::unpriced(
                "sess-win", turn, at, "openai", "unknown-model", "openai-completions", 10, 5, 0,
            );
            entry.day_key = crate::agent::usage_ledger::day_key_for_epoch_ms(at);
            store.append_usage_ledger_entry(&entry).expect("append");
        }

        let session_scoped = snapshot_for_store(&store, Some("sess-win"), PolicyWindow::Session)
            .expect("session snapshot");
        assert_eq!(session_scoped.unpriced_turns, 2, "Session 窗口应统计全部历史未计价 turn");
        let day_scoped = snapshot_for_store(&store, Some("sess-win"), PolicyWindow::Day)
            .expect("day snapshot");
        assert_eq!(
            day_scoped.unpriced_turns, 1,
            "Day 窗口只统计当天；若实现相加会得到 3（重复计数）"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 策略版本：稳定、与金额无关（G5.3）。
    #[test]
    fn policy_version_is_stable_and_ignores_amounts() {
        let base = policy(Some(10.0), Some(0.8), true);
        let clone = policy(Some(10.0), Some(0.8), true);
        assert_eq!(base.version(), clone.version());
        assert_ne!(base.version(), policy(Some(20.0), Some(0.8), true).version());
        assert_eq!(base.version().len(), 12, "版本号取 12 位 hex");
        // 与运行成本无关（纯策略维度）。
        assert_eq!(base.version(), base.version());
    }
}
