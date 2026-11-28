//! 会话级 **Plan 模式**（协作模式），对齐 DeepSeek Harness `packages/plan/plan-mode`。
//!
//! ## 为什么不是步骤清单
//!
//! 旧实现（`plan_state.rs`）把「Plan」做成了带 `steps[]` / `complete_step` / `revision` CAS
//! 的第二套任务清单，于是模型与用户长期在「Plan 还是 Todo」之间串名：`TodoWrite` 的进度被
//! `context.rs` 回灌成 `Plan: x/y completed`，而模型又要为「Plan」维护另一份步骤表。
//!
//! DSH 的口径是：**Plan 是一种会话级协作模式**——先调研、出方案，用户批准后才动手。
//! 它只有 `active: boolean`、一份 Markdown 方案正文与「待生效的切换意图（`pending`）」，
//! 不承载任何步骤进度；步骤进度一律归 `TodoWrite`（单智能体）或 `team_task_*`（团队）。
//!
//! ## 生命周期
//!
//! - **进入**：宿主 `/plan`（或前端开关）→ [`request_toggle`] 写 `pending = Some(true)`；
//!   下一轮 pre-step 由 [`commit_pending`] 落到 `active`（本回合内的工具调用看到的仍是旧值，
//!   与 DSH「用户选择在下一次 pre-step 生效」一致）。
//! - **退出（审核）**：模型调用 `ExitPlanMode`（primitive `plan_control`）提交 Markdown 方案；
//!   该工具走宿主 Ask 通道（`WaitingHost`），用户批准后由
//!   [`resolve_review`] 关闭模式并记录已批准方案；「继续规划」则保持激活。
//! - **退出（直接）**：宿主 `/plan off` → [`request_toggle(false)`]。
//!
//! ## 持久化
//!
//! 进程内 `Mutex` 注册表 + JSON sidecar（`*.tmp` + rename 原子写），键为 session id，
//! 由 `PONY_AGENT_PLAN_MODE_STORE_PATH` 覆盖路径。恢复语义：同一路径下重启后
//! `plan_mode_state(session).active` 与已批准方案保持一致。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

/// 激活期注入动态上下文的策略段（`plan:policy`）。
///
/// 只进**动态层**（`context.rs::volatile_context_notes`），因此稳定层提示词前缀缓存不受影响。
pub const PLAN_MODE_POLICY: &str = "Plan mode is active: research and produce a written plan only. Write and execute tools are blocked until the user approves the plan through ExitPlanMode; keep each step list in TodoWrite.";

/// 审核问句的两个固定选项（宿主 Ask 卡片按此渲染按钮）。
pub const PLAN_REVIEW_APPROVE: &str = "批准执行";
pub const PLAN_REVIEW_KEEP_PLANNING: &str = "继续规划";

/// Plan 模式的切换意图（枚举替代 `bool` 入参：本仓 `clippy::fn_params_excessive_bools`
/// 阈值为 0，新增 bool 形参即判红；调用点 `request_toggle(s, ModeToggle::Enter)` 也比
/// `request_toggle(s, true)` 可读）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeToggle {
    /// 进入模式（下一轮 pre-step 生效）。
    Enter,
    /// 退出模式（`/plan off`，立即生效）。
    Exit,
}

impl ModeToggle {
    pub fn is_enter(self) -> bool {
        matches!(self, Self::Enter)
    }
}

/// 方案审核结果（同上：枚举替代 `bool`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanReviewDecision {
    /// 批准执行：退出模式并标记方案已批准。
    Approve,
    /// 继续规划：保持激活，方案仍未批准。
    KeepPlanning,
}

impl PlanReviewDecision {
    pub fn is_approved(self) -> bool {
        matches!(self, Self::Approve)
    }
}

/// 一条会话的 Plan 模式状态。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanModeState {
    /// 已生效的模式开关。
    pub active: bool,
    /// 用户在本回合内表达的切换意图，下一轮 pre-step 落盘（DSH「选择待提交」语义）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<bool>,
    /// 最近一次提交审核的方案正文（Markdown）。批准后保留为「已批准方案」供 UI 展示。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// 该方案是否已获用户批准（`false` = 仍在规划中）。
    #[serde(default)]
    pub plan_approved: bool,
    /// 每次状态迁移自增，宿主可据此判断是否需要刷新。
    #[serde(default)]
    pub revision: u64,
}

impl PlanModeState {
    /// 生效值：`pending` 优先（尚未 commit 时按用户最新意图回答，避免 UI 与实际开关错位）。
    pub fn effective_active(&self) -> bool {
        self.pending.unwrap_or(self.active)
    }
}

fn store() -> &'static Mutex<BTreeMap<String, PlanModeState>> {
    static STORE: OnceLock<Mutex<BTreeMap<String, PlanModeState>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// sidecar 路径真值：env 覆盖优先，缺省落在数据目录旁的 `plan_mode_store.json`。
pub fn plan_mode_store_path() -> PathBuf {
    if let Ok(raw) = std::env::var("PONY_AGENT_PLAN_MODE_STORE_PATH") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    let base = dirs::data_local_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("pony-agent").join("plan_mode_store.json")
}

fn sidecar_loaded() -> &'static Mutex<bool> {
    static LOADED: OnceLock<Mutex<bool>> = OnceLock::new();
    LOADED.get_or_init(|| Mutex::new(false))
}

/// 首次访问时把 sidecar 读进进程内注册表；解析失败降级为空（不得 panic）。
fn ensure_loaded() {
    let mut loaded = sidecar_loaded().lock().expect("plan mode loader poisoned");
    if *loaded {
        return;
    }
    *loaded = true;
    let path = plan_mode_store_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(parsed) = serde_json::from_str::<BTreeMap<String, PlanModeState>>(&text) else {
        return;
    };
    let mut map = store().lock().expect("plan mode store poisoned");
    map.extend(parsed);
}

/// 原子写：`*.tmp` + rename（沿用 goal sidecar 的写入纪律）。
fn persist(map: &BTreeMap<String, PlanModeState>) {
    let path = plan_mode_store_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(body) = serde_json::to_string_pretty(map) else {
        return;
    };
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    if std::fs::write(&tmp, body).is_err() {
        return;
    }
    let _ = std::fs::rename(&tmp, &path);
}

fn mutate<F: FnOnce(&mut PlanModeState)>(session_id: &str, apply: F) -> PlanModeState {
    ensure_loaded();
    let mut map = store().lock().expect("plan mode store poisoned");
    let entry = map.entry(session_id.to_string()).or_default();
    apply(entry);
    entry.revision = entry.revision.saturating_add(1);
    let snapshot = entry.clone();
    persist(&map);
    snapshot
}

/// 读取一条会话的 Plan 模式状态（未知会话返回缺省值，不写盘）。
pub fn plan_mode_state(session_id: &str) -> PlanModeState {
    ensure_loaded();
    let map = store().lock().expect("plan mode store poisoned");
    map.get(session_id).cloned().unwrap_or_default()
}

/// 模式是否生效（`pending` 优先）。
pub fn is_active(session_id: &str) -> bool {
    plan_mode_state(session_id).effective_active()
}

/// 表达切换意图：写入 `pending`，下一轮 pre-step 由 [`commit_pending`] 落地。
///
/// 重新进入（`active = true`）会**清掉上一份方案的已批准标记**：批准是一次性状态迁移，
/// 新一次规划必须重新提交方案并重新走宿主审核（否则上一份批准会变成长期通行证）。
pub fn request_toggle(session_id: &str, toggle: ModeToggle) -> PlanModeState {
    mutate(session_id, |state| {
        state.pending = Some(toggle.is_enter());
        if toggle.is_enter() {
            state.plan_approved = false;
        }
    })
}

/// 立即切换（宿主 `/plan off` 与审核批准都走这里：不再等下一轮）。
pub fn set_active(session_id: &str, toggle: ModeToggle) -> PlanModeState {
    mutate(session_id, |state| {
        state.active = toggle.is_enter();
        state.pending = None;
    })
}

/// 把 `pending` 落到 `active`；返回是否发生了切换。
pub fn commit_pending(session_id: &str) -> bool {
    let current = plan_mode_state(session_id);
    let Some(pending) = current.pending else {
        return false;
    };
    mutate(session_id, |state| {
        state.active = pending;
        state.pending = None;
    });
    true
}

/// 记录模型提交的方案（尚未批准）。
pub fn submit_plan(session_id: &str, plan: &str) -> PlanModeState {
    mutate(session_id, |state| {
        state.plan = Some(plan.to_string());
        state.plan_approved = false;
    })
}

/// 审核结果落地：`approved = true` 关闭模式并标记方案已批准；否则保持激活。
pub fn resolve_review(session_id: &str, decision: PlanReviewDecision) -> PlanModeState {
    mutate(session_id, |state| {
        if decision.is_approved() {
            state.active = false;
            state.pending = None;
            state.plan_approved = true;
        } else {
            state.active = true;
            state.plan_approved = false;
        }
    })
}

/// 宿主答案 → 是否「批准执行」。
///
/// 宿主（TUI / 桌面端）提交的 Ask 答案形如 `{"answer": "批准执行"}`、`{"text": "..."}`
/// 或纯字符串；空答案视为「继续规划」（fail-safe：默认不放大权限）。
pub fn answer_is_approval(answer: &serde_json::Value) -> bool {
    fn text_of(value: &serde_json::Value) -> Option<&str> {
        match value {
            serde_json::Value::String(text) => Some(text.as_str()),
            serde_json::Value::Object(map) => ["answer", "text", "value", "selected"]
                .iter()
                .find_map(|key| map.get(*key).and_then(text_of)),
            serde_json::Value::Array(items) => items.iter().find_map(text_of),
            _ => None,
        }
    }
    text_of(answer).is_some_and(|text| text.trim().contains(PLAN_REVIEW_APPROVE))
}

/// 测试接缝：清空进程内注册表并强制下次访问重新读盘。
pub fn reset_plan_mode_store_for_test() {
    if let Ok(mut map) = store().lock() {
        map.clear();
    }
    if let Ok(mut loaded) = sidecar_loaded().lock() {
        *loaded = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 隔离 sidecar 路径（进程级一次性设置）。
    ///
    /// 注意**不要**在每例里 `reset_plan_mode_store_for_test()`：注册表是进程级单例，
    /// 并行测试共享它，清空会互相踩（表现为随机失败）。每例改用唯一 session id 隔离即可。
    fn isolate() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let dir = std::env::temp_dir().join(format!(
                "pony-agent-plan-mode-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default()
            ));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            std::env::set_var("PONY_AGENT_PLAN_MODE_STORE_PATH", dir.join("plan_mode.json"));
        });
    }

    /// 每例唯一会话 id（进程级注册表的并行隔离手段）。
    fn session_id(label: &str) -> String {
        format!("{label}-{}", uuid::Uuid::new_v4())
    }

    #[test]
    fn pending_toggle_takes_effect_only_after_commit() {
        isolate();
        let session = session_id("s-pending");
        let session = session.as_str();
        assert!(!is_active(session));

        request_toggle(session, ModeToggle::Enter);
        assert!(
            plan_mode_state(session).effective_active(),
            "pending 期间生效值必须按用户最新意图回答"
        );
        assert!(
            !plan_mode_state(session).active,
            "commit 前不得改写已生效开关（本回合工具仍按旧值判定）"
        );

        assert!(commit_pending(session));
        assert!(plan_mode_state(session).active);
        assert!(!commit_pending(session), "无 pending 时 commit 必须是幂等空操作");
    }

    #[test]
    fn approval_closes_mode_and_keeps_the_approved_plan() {
        isolate();
        let session = session_id("s-review");
        let session = session.as_str();
        set_active(session, ModeToggle::Enter);
        submit_plan(session, "# 方案\n- 步骤一");

        resolve_review(session, PlanReviewDecision::Approve);
        let state = plan_mode_state(session);
        assert!(!state.active, "批准后必须退出 Plan 模式（解锁写权限）");
        assert!(state.plan_approved);
        assert_eq!(state.plan.as_deref(), Some("# 方案\n- 步骤一"));
    }

    #[test]
    fn keep_planning_keeps_the_mode_active_and_plan_unapproved() {
        isolate();
        let session = session_id("s-keep");
        let session = session.as_str();
        set_active(session, ModeToggle::Enter);
        submit_plan(session, "# 方案");

        resolve_review(session, PlanReviewDecision::KeepPlanning);
        let state = plan_mode_state(session);
        assert!(state.active);
        assert!(!state.plan_approved);
    }

    #[test]
    fn answer_shapes_resolve_to_approval_or_keep_planning() {
        isolate();
        assert!(answer_is_approval(&serde_json::json!({"answer": "批准执行"})));
        assert!(answer_is_approval(&serde_json::json!("批准执行")));
        assert!(answer_is_approval(&serde_json::json!({"selected": ["批准执行"]})));
        assert!(!answer_is_approval(&serde_json::json!({"answer": "继续规划"})));
        assert!(
            !answer_is_approval(&serde_json::json!({})),
            "空答案必须落回「继续规划」：默认不放大权限"
        );
    }

    /// 持久化：状态必须写进 sidecar（重启恢复的物理前提）。
    ///
    /// 为什么不断言「清空注册表后能读回」：注册表是**进程级单例**，`reset` 会踩到并行
    /// 运行的其他用例（随机失败）。改为直接校验 sidecar 内容——它才是跨重启的载体。
    #[test]
    fn state_is_persisted_to_the_sidecar() {
        isolate();
        let session = session_id("s-persist");
        let session = session.as_str();
        set_active(session, ModeToggle::Enter);
        submit_plan(session, "# 持久化方案");

        let body = std::fs::read_to_string(plan_mode_store_path()).expect("sidecar must exist");
        let parsed: BTreeMap<String, PlanModeState> =
            serde_json::from_str(&body).expect("sidecar must be valid json");
        let persisted = parsed.get(session).expect("本会话必须落在 sidecar 里");
        assert!(persisted.active, "模式开关必须写入 sidecar");
        assert_eq!(persisted.plan.as_deref(), Some("# 持久化方案"));
    }
}
