use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;

/// 契约 C2 / §8：goal JSON sidecar 路径覆盖用环境变量（非空即整体替代默认路径）。
pub const GOAL_STORE_PATH_ENV: &str = "PONY_AGENT_GOAL_STORE_PATH";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateGoalAction {
    Complete,
    Pause,
    Resume,
    Edit,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalActivation {
    Armed,
    Disarmed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalData {
    pub id: String,
    pub session_id: String,
    pub objective: String,
    pub phase: String,
    pub activation: GoalActivation,
    pub revision: u64,
    #[serde(rename = "roundsStarted")]
    pub rounds_started: u32,
    #[serde(rename = "maxGoalRounds")]
    pub max_goal_rounds: u32,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateGoalArgs {
    #[serde(default)]
    pub session_id: Option<String>,
    pub objective: String,
    pub max_goal_rounds: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateGoalResult {
    pub goal: GoalData,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GetGoalArgs {
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetGoalResult {
    pub goal: Option<GoalData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateGoalArgs {
    #[serde(default)]
    pub session_id: Option<String>,
    pub goal_id: String,
    pub revision: u64,
    pub action: UpdateGoalAction,
    pub objective: Option<String>,
    pub blocked_reason: Option<String>,
    pub max_goal_rounds: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateGoalResult {
    pub goal: GoalData,
}

/// Map: session_id -> GoalData。
///
/// 契约 C2：进程内缓存 + JSON sidecar 懒加载。`None` 表示尚未从磁盘加载（首次访问时加载）；
/// 内存中的 map 是权威读面，写入成功后立即落盘（原子替换）。损坏/不可读的 sidecar 降级为空，
/// 绝不 panic。
static SESSION_GOALS: Mutex<Option<HashMap<String, GoalData>>> = Mutex::new(None);
static GOAL_STORE_DEGRADED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// B130：goal 领域事实的进程级广播（宿主事件 `goal://*` 的唯一真源）。
///
/// 为什么在 core 而不在 Tauri 命令层发射：goal 变更不止走 `goal_create` / `goal_update` /
/// `goal_command` 三个 IPC 命令，模型工具（`execute_update_goal`）、graph 续轮
/// （`advance_goal_round`）与会话删除清理（`clear_goal`）都直接调本模块。把发射点放在
/// 领域写入成功之后，才能覆盖全部变更路径；桌面层只负责把事件映射成 Tauri 线缆载荷
/// （`src-tauri/src/goal_events.rs`），core 不依赖 Tauri。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalEvent {
    /// `create_goal` / `update_goal` / `advance_goal_round` 成功后的目标快照。
    Changed { goal: GoalData },
    /// 每成功发起一轮 goal-round（`rounds_started` 递增后）。
    RoundStarted { goal: GoalData },
    /// `clear_goal` 真的删除了记录（`clear` 无墓碑，故附带被删目标的 id 供审计/诊断）。
    Cleared {
        session_id: String,
        goal_id: Option<String>,
    },
    /// O6（2026-11-27）：一次自动续轮**停止/暂停**（原因是结构化事实，不是 UI 自造文案）。
    ///
    /// 为什么是领域事件：旧实现只有 TUI 的 `UiEvent::GoalRoundsStopped`，桌面端零事件面 ⇒
    /// 「为什么目标不动了」在所有桌面停止路径上都不可见（审核报告 P1-3）。停止原因本身是
    /// goal 生命周期的领域事实（`GoalRoundStopReason`），由领域层广播才可能被两端同源消费。
    RoundsStopped {
        session_id: String,
        goal_id: String,
        /// `GoalRoundStopReason::code()`（稳定机器识别码，UI 不解析文案）。
        reason_code: String,
        /// `GoalRoundStopReason::visible_line()`（用户可见文案的结构化真源）。
        visible_line: String,
        rounds_started: u32,
        max_rounds: u32,
        consecutive_failures: u32,
    },
}

/// O6：广播一次续轮停止/暂停（`reason` 的结构化投影，字段集与 TUI `GoalRoundsStopped` 对齐）。
pub fn notify_goal_rounds_stopped(
    session_id: &str,
    goal_id: &str,
    reason: &GoalRoundStopReason,
    consecutive_failures: u32,
    rounds_started: u32,
    max_rounds: u32,
) {
    notify_goal_event(&GoalEvent::RoundsStopped {
        session_id: session_id.to_string(),
        goal_id: goal_id.to_string(),
        reason_code: reason.code().to_string(),
        visible_line: reason.visible_line(),
        rounds_started,
        max_rounds,
        consecutive_failures,
    });
}

/// 宿主注入的观察者（桌面层安装一次；测试可替换）。
pub type GoalEventObserver = dyn Fn(&GoalEvent) + Send + Sync;

static GOAL_EVENT_OBSERVER: Mutex<Option<std::sync::Arc<GoalEventObserver>>> = Mutex::new(None);

/// B130：安装进程级 goal 事件观察者（桌面层唯一调用点；重复安装覆盖旧值）。
///
/// 观察者在 goal 写锁**释放之后**被调用，故可以安全地回调本模块的读面（`get_goal` 等）。
pub fn install_goal_event_observer(observer: std::sync::Arc<GoalEventObserver>) {
    let mut slot = GOAL_EVENT_OBSERVER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *slot = Some(observer);
}

/// 测试接缝：复位进程级观察者（集成测试与 lib 单测共用同一进程观察者槽位）。
///
/// 为什么不是 `#[cfg(test)]`：`pony-agent-core` 的 `tests/` 集成测试需要在本用例结束时
/// 卸载收集器，否则观察者会把后续用例的事件也收集进去（跨用例串话）。本仓既有接缝
/// （`test_support` 模块）同样是「常驻导出 + `#[doc(hidden)]`」的口径，此处对齐，
/// 避免集成测试被迫自己再造一份观察者管理。
#[doc(hidden)]
pub fn reset_goal_event_observer_for_test() {
    let mut slot = GOAL_EVENT_OBSERVER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *slot = None;
}

/// 广播一条 goal 事实；无观察者时静默（core 单测 / TUI / CLI 都不需要宿主事件）。
///
/// 失败语义：观察者自身的发射失败只由观察者记录诊断，**绝不回滚已落盘的目标变更**
/// （与 `docs/architecture/plan-goal-contract.md` §7「发射失败只记诊断」一致）。
fn notify_goal_event(event: &GoalEvent) {
    let observer = GOAL_EVENT_OBSERVER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Some(observer) = observer {
        observer(event);
    }
}

#[cfg(test)]
static TEST_DEFAULT_PATH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// 契约 C2 / §8：sidecar 路径真值。env `PONY_AGENT_GOAL_STORE_PATH` 非空即整体覆盖；
/// 默认落在 `default_sqlite_path().parent()/goal_store.json`（与既有会话库同目录）。
pub fn goal_store_path() -> PathBuf {
    if let Some(raw) = std::env::var_os(GOAL_STORE_PATH_ENV) {
        let candidate = PathBuf::from(raw);
        if !candidate.as_os_str().is_empty() {
            return candidate;
        }
    }

    #[cfg(test)]
    {
        return TEST_DEFAULT_PATH
            .get_or_init(|| {
                std::env::temp_dir().join(format!(
                    "pony-goal-test-{}-{}.json",
                    std::process::id(),
                    Uuid::new_v4()
                ))
            })
            .clone();
    }

    #[cfg(not(test))]
    crate::sqlite_session::default_sqlite_path()
        .parent()
        .map(|dir| dir.join("goal_store.json"))
        .unwrap_or_else(|| PathBuf::from("goal_store.json"))
}

fn goal_store_guard() -> std::sync::MutexGuard<'static, Option<HashMap<String, GoalData>>> {
    SESSION_GOALS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 懒加载：首次访问（或 `reset_goal_store_for_test` 之后）从 sidecar 读回。
fn ensure_goal_store_loaded(guard: &mut Option<HashMap<String, GoalData>>) {
    if guard.is_none() {
        *guard = Some(load_goal_store());
    }
}

/// 旧版无条件兜底写入的伪会话键（契约 C1 已移除的 `default` 兜底）。
const LEGACY_FALLBACK_SESSION_ID: &str = "default";

/// B152 存量迁移：剔除旧 `default` 兜底写入的条目，返回被删除的 key（可能为空）。
///
/// 判定只针对「旧兜底产物」：key 去空白后为空、key 是 `default`、或内嵌 `session_id` 是
/// `default`。key 与内嵌 `session_id` 不一致但两者都非 `default` 的记录**保留**——那不属于
/// 本缺陷定义域，误删会丢掉真实会话的目标。
fn purge_legacy_goal_entries(goals: &mut HashMap<String, GoalData>) -> Vec<String> {
    let legacy: Vec<String> = goals
        .iter()
        .filter(|(session, goal)| {
            let key = session.trim();
            key.is_empty()
                || key == LEGACY_FALLBACK_SESSION_ID
                || goal.session_id.trim() == LEGACY_FALLBACK_SESSION_ID
        })
        .map(|(session, _)| session.clone())
        .collect();

    for key in &legacy {
        goals.remove(key);
    }
    legacy
}

/// 契约 C2：读取 sidecar。文件缺失返回空 map；不可读或 JSON 损坏置 degraded 标记。
fn load_goal_store() -> HashMap<String, GoalData> {
    let path = goal_store_path();
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            GOAL_STORE_DEGRADED.store(false, std::sync::atomic::Ordering::SeqCst);
            return HashMap::new();
        }
        Err(err) => {
            eprintln!(
                "[pony-agent] goal store `{}` failed to read ({err}); degraded to readonly",
                path.display()
            );
            GOAL_STORE_DEGRADED.store(true, std::sync::atomic::Ordering::SeqCst);
            return HashMap::new();
        }
    };

    match serde_json::from_str::<HashMap<String, GoalData>>(&raw) {
        Ok(mut goals) => {
            GOAL_STORE_DEGRADED.store(false, std::sync::atomic::Ordering::SeqCst);
            let purged = purge_legacy_goal_entries(&mut goals);
            if !purged.is_empty() {
                eprintln!(
                    "[pony-agent] goal store `{}`: purged {} legacy `{}` entr{}",
                    path.display(),
                    purged.len(),
                    LEGACY_FALLBACK_SESSION_ID,
                    if purged.len() == 1 { "y" } else { "ies" }
                );
                // 必须把同一组 key 作为显式删除交给合并层：`persist_goal_store` 会先把磁盘
                // 内容合入待写快照，只写内存 map 会让旧条目原样复活。回写失败不影响读面
                // （内存已清），下次加载重试。
                if let Err(error) = persist_goal_store(&goals, &purged) {
                    eprintln!(
                        "[pony-agent] goal store `{}` legacy purge write-back failed ({error}); will retry on next load",
                        path.display()
                    );
                }
            }
            goals
        }
        Err(error) => {
            eprintln!(
                "[pony-agent] goal store `{}` is unreadable ({error}); degraded to empty",
                path.display()
            );
            GOAL_STORE_DEGRADED.store(true, std::sync::atomic::Ordering::SeqCst);
            HashMap::new()
        }
    }
}

#[cfg(unix)]
fn lock_file(file: &fs::File) -> Result<(), String> {
    use std::os::unix::io::AsRawFd;
    let res = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if res != 0 {
        return Err(format!("flock failed: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(not(unix))]
fn lock_file(_file: &fs::File) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn unlock_file(file: &fs::File) {
    use std::os::unix::io::AsRawFd;
    unsafe {
        libc::flock(file.as_raw_fd(), libc::LOCK_UN);
    }
}

#[cfg(not(unix))]
fn unlock_file(_file: &fs::File) {}

/// 契约 C2 / NFR `goal_store_atomic_write`：`*.tmp` + rename 原子替换，父目录按需创建。
///
/// `explicit_removals`：本次调用**显式删除**的 session key。跨进程合并只做 `insert` 会把
/// 磁盘上仍存在的旧条目重新插回（`clear_goal` 被静默复活），故合并后必须逐项 `remove`。
/// 非删除类调用一律传 `&[]`，行为与旧实现完全一致。
fn persist_goal_store(
    goals: &HashMap<String, GoalData>,
    explicit_removals: &[String],
) -> Result<(), String> {
    let path = goal_store_path();
    if GOAL_STORE_DEGRADED.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(format!(
            "goal store at `{}` is degraded due to read/parse error; refusing to overwrite",
            path.display()
        ));
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create goal store directory `{}`: {error}",
                    parent.display()
                )
            })?;
        }
    }

    let lock_path = path.with_extension("lock");
    let lock_handle = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| format!("failed to open lock file `{}`: {error}", lock_path.display()))?;

    lock_file(&lock_handle)?;

    // 跨进程写时合并：若磁盘已有更新且合法，保留其他 session
    let mut to_write = if path.exists() {
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(disk_goals) = serde_json::from_str::<HashMap<String, GoalData>>(&raw) {
                let mut merged = disk_goals;
                for (k, v) in goals {
                    merged.insert(k.clone(), v.clone());
                }
                merged
            } else {
                goals.clone()
            }
        } else {
            goals.clone()
        }
    } else {
        goals.clone()
    };

    // 显式删除必须压过磁盘合并结果：否则另一进程写回的旧条目会让 clear 静默复活。
    for key in explicit_removals {
        to_write.remove(key);
    }

    let payload = match serde_json::to_string_pretty(&to_write) {
        Ok(p) => p,
        Err(error) => {
            unlock_file(&lock_handle);
            return Err(format!("failed to serialize goal store: {error}"));
        }
    };

    let tmp_path = goal_store_tmp_path(&path);
    if let Err(error) = fs::write(&tmp_path, payload.as_bytes()) {
        unlock_file(&lock_handle);
        return Err(format!(
            "failed to write goal store tmp `{}`: {error}",
            tmp_path.display()
        ));
    }

    if let Err(error) = fs::rename(&tmp_path, &path) {
        let _ = fs::remove_file(&tmp_path);
        unlock_file(&lock_handle);
        return Err(format!(
            "failed to atomically replace goal store `{}`: {error}",
            path.display()
        ));
    }

    unlock_file(&lock_handle);
    Ok(())
}

fn goal_store_tmp_path(path: &Path) -> PathBuf {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let ext = format!("{pid}.{nanos}.tmp");
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("goal_store.json"));
    name.push(".");
    name.push(ext);
    path.with_file_name(name)
}

/// 契约 C1：唯一目标真值源——同 session 精确匹配 + `phase == "active"` + `activation == Armed`。
/// 不做 `default` 兜底、不做前缀匹配；无命中返回 `None`。
pub fn active_goal_for_session(session_id: &str) -> Option<GoalData> {
    let session = session_id.trim();
    if session.is_empty() {
        return None;
    }

    let mut guard = goal_store_guard();
    ensure_goal_store_loaded(&mut guard);
    let goals = guard.as_ref().expect("goal store must be loaded");
    goals
        .get(session)
        .filter(|goal| goal.phase == "active" && goal.activation == GoalActivation::Armed)
        .cloned()
}

/// 契约 C1 的续轮入口：从 run 的会话事实取「本会话」的 active goal。
///
/// 无会话事实（缺失/空白）或无 active goal ⇒ `None`，调用方**不续轮**。旧实现回落
/// `default` 桶，会把**别的会话**的目标读出来继续跑（跨会话串话根因，B152）。
pub fn continuation_goal_for_run(run_session_id: Option<&str>) -> Option<(String, GoalData)> {
    let session_id = run_session_id
        .map(str::trim)
        .filter(|id| !id.is_empty())?
        .to_string();
    let goal = active_goal_for_session(&session_id)?;
    Some((session_id, goal))
}

/// 契约 C3：goal-round 用户消息的唯一文案实现（宿主循环与 TUI 必须共用）。
pub fn build_goal_round_message(objective: &str, round: u32, max_rounds: u32) -> String {
    format!(
        "<goal_round round=\"{round}\" max=\"{max_rounds}\">继续推进目标：{objective}。以工作区与工具结果为准；仅当目标真正达成才 update_goal complete。若本轮仍有未完成步骤，直接继续执行，不要询问用户。</goal_round>"
    )
}

/// 契约 C3：判定一段 user 轮文本是否为宿主生成的 goal 续轮文案。
///
/// 真值源是本模块的 [`build_goal_round_message`]：
/// `<goal_round round="N" max="M">继续推进目标：…</goal_round>`。会话层
/// （`is_host_continuation_prompt`）据此把宿主注入的续轮文案与真人消息分开——二者混同会让
/// 「新一轮用户消息」把仍然待作答的 Ask 判为过期（自锁循环），也会把续轮文案当会话标题。
pub fn is_goal_round_prompt(text: &str) -> bool {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix("<goal_round ") else {
        return false;
    };
    let Some((attrs, body)) = rest.split_once('>') else {
        return false;
    };
    let Some(body) = body.strip_suffix("</goal_round>") else {
        return false;
    };
    if !body.starts_with("继续推进目标：") {
        return false;
    }
    let is_uint_attr = |name: &str| {
        attrs.split_whitespace().any(|attr| {
            attr.strip_prefix(name)
                .and_then(|value| value.strip_prefix("=\""))
                .and_then(|value| value.strip_suffix('"'))
                .map(|value| !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit()))
                .unwrap_or(false)
        })
    };
    is_uint_attr("round") && is_uint_attr("max")
}

/// 契约 C2：测试接缝——清空进程内缓存并从 `goal_store_path()` 重新懒加载。
pub fn reset_goal_store_for_test() {
    let mut guard = goal_store_guard();
    *guard = Some(load_goal_store());
}

/// 契约 C3：`rounds_started += 1` 并原子持久化，返回更新后的目标快照。
///
/// `rounds_started` 只统计成功发起的 goal-round 轮次（不因 CAS 递增 revision：目标内容未变，
/// 保留 revision 以免打断调用方的 compare-and-swap 窗口）。
pub fn advance_goal_round(goal_id: &str) -> Result<GoalData, String> {
    let target = goal_id.trim();
    if target.is_empty() {
        return Err("Goal ID cannot be empty".to_string());
    }

    let mut guard = goal_store_guard();
    ensure_goal_store_loaded(&mut guard);
    let goals = guard.as_mut().expect("goal store must be loaded");
    let session = goals
        .iter()
        .find(|(_, goal)| goal.id == target)
        .map(|(session, _)| session.clone())
        .ok_or_else(|| format!("No goal found for goal id `{target}`"))?;
    let previous = goals.get(&session).cloned();
    let updated = {
        let goal = goals
            .get_mut(&session)
            .expect("goal must exist after id lookup");
        goal.rounds_started = goal.rounds_started.saturating_add(1);
        goal.clone()
    };

    if let Err(error) = persist_goal_store(goals, &[]) {
        match previous {
            Some(previous) => {
                goals.insert(session, previous);
            }
            None => {
                goals.remove(&session);
            }
        }
        return Err(error);
    }

    // B130：先释放写锁再广播（观察者可能回调本模块读面）。
    drop(guard);
    notify_goal_event(&GoalEvent::Changed {
        goal: updated.clone(),
    });
    notify_goal_event(&GoalEvent::RoundStarted {
        goal: updated.clone(),
    });
    Ok(updated)
}

/// PA-171 C1：同一续轮循环内**连续失败**的止损阈值（任务卡冻结为 2 轮）。
///
/// 旧实现只判「开了几轮」（`should_abort_goal_rounds` 的 8 轮上限）而**不看上一轮成败**，
/// 一次上游 429（每轮首个模型调用即失败）会被重放成 16 轮（2 个轮次线程 × 8），
/// 20 轮预算 74 秒烧光。
pub const MAX_CONSECUTIVE_GOAL_ROUND_FAILURES: u32 = 2;

/// PA-171 C1/C2：单轮 goal 续轮的终态归约。
///
/// 只有 [`GoalRoundOutcome::Succeeded`] 消耗轮次预算（C2：失败不烧预算）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalRoundOutcome {
    Succeeded,
    Failed,
}

/// 停止原因的用户可见严重级（core 层中性枚举，不依赖 TUI 渲染类型）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalStopSeverity {
    Info,
    Warning,
    Error,
}

/// PA-171 C1/C2：续轮循环的停止原因（`Some` ⇒ 停止新增 goal 轮，且**必须告知用户**）。
///
/// ### 一个事实一个家与结构化契约
/// - 对齐 deepseek-harness 的 `GoalBlockReason { code, message }` 结构化拆分规范：
///   - [`Self::code`]: 日志唯一来源（全小写 snake_case 稳定代号）。
///   - [`Self::severity`]: 配色与渲染等级的唯一来源（[`GoalStopSeverity`] 中性枚举）。
///   - [`Self::headline`] / [`Self::detail`] / [`Self::action_hint`]: 用户可见文案的唯一结构化真源，由 [`Self::visible_line`] 拼装。
/// - **本轮不改持久化格式**：[`GoalData::blocked_reason`] 仍保留为 `Option<String>`，
///   理由是持久化升级涉及 serde 向后兼容、桌面端线缆契约以及工具 schema 联动，超出本轮
///   最小必要闭包，显式 deferred。
/// - 契约矩阵 §2.2 C1-S3/C1-S4：前三变体由 [`goal_round_stop_reason`] 产出；
///   其余变体为宿主或 TUI 在特定边界下构造的 additive 变体。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalRoundStopReason {
    /// 用户按 Esc 取消（最高优先级，不得被失败计数掩盖）。
    Cancelled,
    /// 连续失败达到 [`MAX_CONSECUTIVE_GOAL_ROUND_FAILURES`]（上游限流/超时的放大面）。
    ConsecutiveFailures { failures: u32 },
    /// `rounds_started` 用满 `max_goal_rounds`。
    BudgetExhausted {
        rounds_started: u32,
        max_rounds: u32,
    },
    /// 每线程自动续轮次数用尽（既有 F-G 契约的 8 轮预算；不静默 break）。
    AutoContinueBudget,
    /// PA-171 C1：本轮**挂起**等待用户（Ask / 审批待作答）；续轮让位，不再起新轮。
    ///
    /// 与 `AutoContinueBudget` 同为 additive 变体：`goal_round_stop_reason` 不产出它，
    /// 由宿主按 `TurnStreamOutcome::Suspended` 构造（否则「模型挂起等用户」会被当成功多开一轮）。
    TurnSuspended,
    /// PA-171 C1：本轮**无可用终态**（收口失败 / 旧无 run 事实兜底路径）。
    ///
    /// additive 变体，由宿主按 `TurnStreamOutcome::Unknown` 或派发 `Err` 构造；
    /// 语义是「不知道成没成」⇒ 停止续轮，且**不计失败、不烧预算**（不得冒充失败轮）。
    TurnUnclassified,
    /// PA-171 C1-T2：**同一会话**已有另一个续轮循环持租约（NFR `concurrency_lock` 声明按会话
    /// 互斥）。本线程不推进任何 goal 轮，但必须可见（不得静默 return）。
    ///
    /// 其 detail（「本线程让位，未推进轮次」）含「让位」是整张表中机制说明的唯一例外，
    /// 理由是并发租约冲突属于运行时调度层面的多线程避让，必须向用户阐明本线程主动让位且
    /// 未推进轮次，避免用户误解为当前任务异常挂死。
    LoopAlreadyOwned,
    /// PA-172 §8 B4（遗留 #9）：本轮成功但**预算推进落盘失败** ⇒ 停止自动续轮。
    ///
    /// additive 第 8 变体，由 TUI 侧在 `advance_goal_round_for_outcome` 返回 `Err` 时构造；
    /// [`goal_round_stop_reason`] 三值真值表**不产出**它。语义与 [`Self::BudgetExhausted`]
    /// 正交：这里预算**没用完**，是「记不住」⇒ 不得复用「预算已用尽」文案。
    BudgetPersistFailed,
    /// PA-172 §8 B5（遗留 #23）：续轮循环发现**当前会话已无进行中的目标** ⇒ 无轮可续。
    ///
    /// additive 第 9 变体，由 TUI 侧构造（原先该路径是裸 `break`，用户看不到原因）；
    /// [`goal_round_stop_reason`] 三值真值表**不产出**它。
    NoActiveGoal,
    /// B193（2026-11-15）：**首轮门未通过** ⇒ 自动续轮从未启动。
    ///
    /// PA-171 C1-T1 的首轮门规定「只有本轮真正成功收口才进入自动续轮」，但旧实现在首轮
    /// 终态非 `Completed`（provider 失败 / 取消 / 挂起 / 无可用终态）时**裸 `return`**，
    /// 既不产生 `GoalRoundsStopped`，也不写状态行、消息区 ⇒ 用户只看到「这一轮结束了」，
    /// 误判会话终止，需手动发「继续」才发现还能推进（症状取证见 backlog B193）。
    ///
    /// additive 第 10 变体，由 TUI 侧按首轮 `TurnStreamOutcome` 构造；
    /// [`goal_round_stop_reason`] 三值真值表**不产出**它。
    FirstRoundNotSucceeded,
}

impl GoalRoundStopReason {
    /// 稳定机器识别码（snake_case），作为结构化日志与追踪的唯一来源。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::ConsecutiveFailures { .. } => "consecutive_failures",
            Self::BudgetExhausted { .. } => "budget_exhausted",
            Self::AutoContinueBudget => "auto_continue_budget",
            Self::TurnSuspended => "turn_suspended",
            Self::TurnUnclassified => "turn_unclassified",
            Self::LoopAlreadyOwned => "loop_already_owned",
            Self::BudgetPersistFailed => "budget_persist_failed",
            Self::NoActiveGoal => "no_active_goal",
            Self::FirstRoundNotSucceeded => "first_round_not_succeeded",
        }
    }

    /// 停止原因的用户可见严重级（用于 TUI/UI 配色与指示图标）。
    pub fn severity(&self) -> GoalStopSeverity {
        match self {
            Self::Cancelled | Self::LoopAlreadyOwned => GoalStopSeverity::Info,
            Self::AutoContinueBudget
            | Self::TurnSuspended
            | Self::TurnUnclassified
            | Self::NoActiveGoal => GoalStopSeverity::Warning,
            Self::ConsecutiveFailures { .. }
            | Self::BudgetExhausted { .. }
            | Self::BudgetPersistFailed
            | Self::FirstRoundNotSucceeded => GoalStopSeverity::Error,
        }
    }

    /// 用户可见的简短标题/主说明。
    ///
    /// **只描述「自动续轮」这一个事实，不冒认阶段**：目标本身可能仍是 `GoalPhase::Active`
    /// （目标还在，只是这一轮之后不再自动往下推）。旧措辞「目标已暂停：…」会在 Ctrl+T 的
    /// Goal 页签里与阶段行「进行中」同屏出现——同一状态被两个结论同时声明，用户只能读到矛盾。
    /// 阶段措辞的唯一家是 `goal_panel::phase_label` / `task_center::phase_text`；
    /// 本函数的家是续轮循环的启动/停止。
    pub fn headline(&self) -> &'static str {
        match self {
            Self::LoopAlreadyOwned => "本会话已有续轮循环在运行",
            // 「未启动」是 `never_started()` 的专属措辞：这三个原因不存在「停下来」这个事实。
            Self::FirstRoundNotSucceeded => "自动续轮未启动",
            // O1-B（PR #106）：挂起/待作答是**暂停**而不是终止——作答后宿主会重入续轮，措辞必须一致，
            // 否则用户看到「续轮已停止」会以为目标已死，于是不去作答，而作答正是唯一的解锁动作。
            //
            // 2026-11-28：本变体同时覆盖 Ask 与审批（`TurnSuspended` 是两者共用的挂起信号），
            // 且旧文案把「等什么」说了两遍——headline「续轮暂停：等你作答」+ detail
            // 「本轮在等你作答/审批」，同一行里「等你作答」出现两次，读起来像一句自我复述。
            // 现在：①「等什么」只在 headline 说一次，detail 归 `None`（无第二层原因可讲）；
            // ② 去掉只有实现者才懂的「续轮」一词；③ 不再枚举「/审批」——本变体同时服务 Ask
            // 与审批两种挂起，但用户此刻面对的是**屏幕上那一个待回应面板**，把两种入口都写进
            // 一行只会让人去找「另一个审批在哪」；两者的公共动作就是「作答」。
            // ④ 仍**不**写「已暂停」，避开 `GoalPhase::Paused` 的既有译词
            // （`goal_panel::stopped_reason_never_contradicts_active_phase` 会判红）。
            Self::TurnSuspended => "暂停中，等你作答",
            _ => "自动续轮已停止",
        }
    }

    /// 停止原因的具体详细信息（若有）。
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::Cancelled => Some("用户取消".to_string()),
            Self::ConsecutiveFailures { failures } => Some(format!("连续 {failures} 轮失败")),
            Self::BudgetExhausted {
                rounds_started,
                max_rounds,
            } => Some(format!("目标轮次已用尽 {rounds_started}/{max_rounds}")),
            Self::AutoContinueBudget => Some("连续自动推进达上限".to_string()),
            // 2026-11-28：旧 detail「本轮在等你作答/审批」与 headline「等你作答」同义重复，
            // 同一行里把「等什么」说了两遍。归 None —— 本变体没有第二层原因可讲。
            Self::TurnSuspended => None,
            Self::TurnUnclassified => Some("本轮未产出可用终态".to_string()),
            Self::LoopAlreadyOwned => Some("本线程让位，未推进轮次".to_string()),
            Self::BudgetPersistFailed => Some("本轮预算推进写盘失败".to_string()),
            // 撞词避让：「进行中」是 `GoalPhase::Active` 的既有译词（`goal_panel::phase_label`），
            // 这里若沿用「没有进行中的目标」会让横幅的「阶段 进行中」与停止原因互相打脸。
            Self::NoActiveGoal => Some("当前会话没有可续的目标".to_string()),
            Self::FirstRoundNotSucceeded => Some("首轮失败".to_string()),
        }
    }

    /// 面向下一步操作的用户指引动作提示（若有）。
    pub fn action_hint(&self) -> Option<&'static str> {
        match self {
            Self::Cancelled | Self::AutoContinueBudget => Some("发消息可继续"),
            Self::ConsecutiveFailures { .. } => Some("查看上方错误，修复后发「继续」"),
            Self::BudgetExhausted { .. } => Some("/goal 查看或提高轮次上限"),
            // O1-B：挂起 ≠ 终止——承诺「作答后自动继续」，用户才会去作答（P0-1 的可见面）。
            Self::TurnSuspended => Some("作答后自动继续"),
            Self::TurnUnclassified => Some("查看日志后发「继续」"),
            Self::LoopAlreadyOwned => None,
            Self::BudgetPersistFailed => Some("检查磁盘后发「继续」"),
            Self::NoActiveGoal => Some("/goal 创建目标"),
            Self::FirstRoundNotSucceeded => Some("发「继续」重试"),
        }
    }

    /// 是否应写入会话历史/持久化阻塞记录。
    pub fn enters_history(&self) -> bool {
        !matches!(self, Self::LoopAlreadyOwned)
    }

    /// 续轮**从未启动**（而非「跑起来后停下」）。
    ///
    /// 语义区分是 B193 决策记录的硬约束：`FirstRoundNotSucceeded`（首轮门未过）、
    /// `NoActiveGoal`（本就无轮可续）、`LoopAlreadyOwned`（本线程没推进任何轮）都不存在
    /// 「停下来」这个事实，任何把它们说成「已停止」的展示都是假结论。
    /// `headline()` 已按本判据取措辞（「自动续轮未启动」vs「自动续轮已停止」）；
    /// 底栏自 2026-11-27 起只用图标区分状态，不再派生 `·停` / `·未续` 文字后缀。
    pub fn never_started(&self) -> bool {
        matches!(
            self,
            Self::FirstRoundNotSucceeded | Self::NoActiveGoal | Self::LoopAlreadyOwned
        )
    }

    /// 全部 10 个变体（参数变体取代表值）：契约测试与「枚举驱动映射」的唯一遍历源。
    ///
    /// 新增变体时本表必须同步，否则 `goal_stop_*` 系列契约测试会因长度不符而红——
    /// 这是刻意的：漏登记新变体 = 日志 / 配色 / 文案三处同时漏配。
    pub fn all_variants() -> [GoalRoundStopReason; 10] {
        [
            Self::Cancelled,
            Self::ConsecutiveFailures { failures: 2 },
            Self::BudgetExhausted {
                rounds_started: 20,
                max_rounds: 20,
            },
            Self::AutoContinueBudget,
            Self::TurnSuspended,
            Self::TurnUnclassified,
            Self::LoopAlreadyOwned,
            Self::BudgetPersistFailed,
            Self::NoActiveGoal,
            Self::FirstRoundNotSucceeded,
        ]
    }

    /// 停止原因的单行可见文本。以 `headline` 开头，用全角 `｜` 拼接非 None 段。
    pub fn visible_line(&self) -> String {
        let mut parts = vec![self.headline().to_string()];
        if let Some(detail) = self.detail() {
            parts.push(detail);
        }
        if let Some(hint) = self.action_hint() {
            parts.push(hint.to_string());
        }
        parts.join("｜")
    }
}

/// O2（2026-11-27）：续轮判据的**唯一输入**。
///
/// 为什么要有它：旧实现在 TUI（`run_goal_rounds`）与 core（`execute_graph_run_stream` /
/// `advance_graph_run`）各写一套停止判据，能力不对等——core 缺待作答让位、缺连续失败熔断、
/// 失败轮照烧预算且停止不可见。把全部判据收进一个纯 struct + 一个纯函数，两端共用同一真值表。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoalRoundGateInput {
    /// 本会话是否仍有 `active + Armed` 的目标（无 ⇒ 无轮可续）。
    pub has_active_goal: bool,
    /// 用户已请求停止该会话的续轮（Esc / 切走会话）。
    pub cancelled: bool,
    /// 待作答的 Ask 数量（宿主请求，续轮必须让位）。
    pub pending_asks: usize,
    /// 待决策的审批数量（同上）。
    pub pending_approvals: usize,
    /// 本循环内连续失败的轮数。
    pub consecutive_failures: u32,
    /// 目标已成功推进的轮数。
    pub rounds_started: u32,
    /// 目标轮次上限（`GoalData::max_goal_rounds`）。
    pub max_rounds: u32,
    /// 本次用户输入触发的自动续轮次数（每线程预算）。
    pub consecutive_auto_continues: u32,
    /// 每线程自动续轮上限（`MAX_CONSECUTIVE_AUTO_CONTINUES`）。
    pub max_auto_continues: u32,
}

/// O2：续轮边界决策。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalRoundGateDecision {
    /// 可以开下一轮。
    Continue,
    /// **暂停**：有人在等宿主（Ask/审批待作答）。目标保持 Armed，作答后宿主必须重入续轮
    /// （O1-B 的语义支点：暂停 ≠ 终止）。可见原因复用 [`GoalRoundStopReason::TurnSuspended`]。
    Pause,
    /// **终止**本轮自动续轮（原因必须可见）。
    Stop(GoalRoundStopReason),
}

impl GoalRoundGateDecision {
    /// 决策对应的可见原因（`Continue` ⇒ `None`；`Pause` ⇒ `TurnSuspended`）。
    pub fn stop_reason(&self) -> Option<GoalRoundStopReason> {
        match self {
            Self::Continue => None,
            Self::Pause => Some(GoalRoundStopReason::TurnSuspended),
            Self::Stop(reason) => Some(reason.clone()),
        }
    }

    /// 是否应继续开下一轮。
    pub fn is_continue(&self) -> bool {
        matches!(self, Self::Continue)
    }

    /// 是否为「暂停等人类」（不是终止：作答后必须重入续轮）。
    pub fn is_pause(&self) -> bool {
        matches!(self, Self::Pause)
    }
}

/// O2：续轮循环的**唯一轮次边界判定**（纯函数，可单测）。
///
/// 判定顺序（冻结真值表，TUI 与 core 必须逐字一致）：
/// `has_active_goal=false` → `cancelled` → 有待作答宿主请求（**Pause**） →
/// 连续失败 ≥ [`MAX_CONSECUTIVE_GOAL_ROUND_FAILURES`] → 目标预算用满 →
/// 每线程自动续轮用尽 → `Continue`。
///
/// 为什么让位排在失败/预算之前：有待作答的 Ask 时**根本不该起新轮**（新轮入口会把旧提问判为
/// 过期，模型只能重复提问）；这不计入失败、也不消耗预算。
pub fn goal_round_gate(input: GoalRoundGateInput) -> GoalRoundGateDecision {
    if !input.has_active_goal {
        return GoalRoundGateDecision::Stop(GoalRoundStopReason::NoActiveGoal);
    }
    if input.cancelled {
        return GoalRoundGateDecision::Stop(GoalRoundStopReason::Cancelled);
    }
    if input.pending_asks > 0 || input.pending_approvals > 0 {
        return GoalRoundGateDecision::Pause;
    }
    if input.consecutive_failures >= MAX_CONSECUTIVE_GOAL_ROUND_FAILURES {
        return GoalRoundGateDecision::Stop(GoalRoundStopReason::ConsecutiveFailures {
            failures: input.consecutive_failures,
        });
    }
    if input.rounds_started >= input.max_rounds {
        return GoalRoundGateDecision::Stop(GoalRoundStopReason::BudgetExhausted {
            rounds_started: input.rounds_started,
            max_rounds: input.max_rounds,
        });
    }
    if input.consecutive_auto_continues >= input.max_auto_continues {
        return GoalRoundGateDecision::Stop(GoalRoundStopReason::AutoContinueBudget);
    }
    GoalRoundGateDecision::Continue
}

/// O2：单轮终态（宿主归约后）→ 续轮决策。
///
/// 「首轮门」与「每步归约」共用本函数：失败 ⇒ [`GoalRoundStopReason::FirstRoundNotSucceeded`]
/// （续轮从未启动；不得复用 `ConsecutiveFailures`——那读作「已经连跑了好几轮」），
/// 挂起 ⇒ **Pause**（`TurnSuspended` 语义：目标仍 Armed，作答后重入）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalRoundStepOutcome {
    Completed,
    Failed,
    Suspended,
    Cancelled,
    Unclassified,
}

pub fn goal_round_outcome_gate(outcome: GoalRoundStepOutcome) -> GoalRoundGateDecision {
    match outcome {
        GoalRoundStepOutcome::Completed => GoalRoundGateDecision::Continue,
        GoalRoundStepOutcome::Failed => {
            GoalRoundGateDecision::Stop(GoalRoundStopReason::FirstRoundNotSucceeded)
        }
        GoalRoundStepOutcome::Suspended => GoalRoundGateDecision::Pause,
        GoalRoundStepOutcome::Cancelled => {
            GoalRoundGateDecision::Stop(GoalRoundStopReason::Cancelled)
        }
        GoalRoundStepOutcome::Unclassified => {
            GoalRoundGateDecision::Stop(GoalRoundStopReason::TurnUnclassified)
        }
    }
}

/// PA-171 C1/C2 兼容读面：等价于 [`goal_round_gate`] 的「无待作答、无自动续轮预算」投影。
///
/// 保留本函数只为既有单测/调用方（它们只关心 cancelled/失败/目标预算三值）；**新代码一律
/// 用 [`goal_round_gate`]**——两处判据漂移正是 O2 要消灭的缺陷。
pub fn goal_round_stop_reason(
    cancelled: bool,
    consecutive_failures: u32,
    rounds_started: u32,
    max_rounds: u32,
) -> Option<GoalRoundStopReason> {
    goal_round_gate(GoalRoundGateInput {
        has_active_goal: true,
        cancelled,
        pending_asks: 0,
        pending_approvals: 0,
        consecutive_failures,
        rounds_started,
        max_rounds,
        consecutive_auto_continues: 0,
        max_auto_continues: u32::MAX,
    })
    .stop_reason()
}

/// PA-171 C2：按**本轮终态**推进轮次预算——只有成功轮 `rounds_started += 1`。
///
/// 失败轮**绝不写盘**（只回读当前快照），从而保证「一次限流」不会烧掉后续轮次预算；
/// 与 [`advance_goal_round`] 相比，本函数是续轮循环唯一应使用的入口。
pub fn advance_goal_round_for_outcome(
    goal_id: &str,
    outcome: GoalRoundOutcome,
) -> Result<GoalData, String> {
    match outcome {
        GoalRoundOutcome::Succeeded => advance_goal_round(goal_id),
        GoalRoundOutcome::Failed => goal_snapshot(goal_id),
    }
}

/// O5（2026-11-27）：`update_goal blocked` 的**最小轮次门槛**。
///
/// 旧实现只在提示词里写「同一阻塞连续 3 轮才可标记 blocked」（`tools/prompt_contract.rs`），
/// 代码零校验 ⇒ 模型第 1 轮就能 blocked，目标被人为提前判死。DSH 的
/// `GOAL_TOOL_BLOCK_THRESHOLD` 是硬拒（`tool-goal/src/index.ts`），这里对齐。
pub const GOAL_BLOCK_MIN_ROUNDS: u32 = 3;

/// O7（2026-11-27）：**每次用户输入**允许的连续自动续轮上限（双预算之一）。
///
/// goal 的轮次预算从来是**两个**数，此前只把目标级那一个展示给用户，于是出现「承诺 50 轮、
/// 实跑 8 轮」的观感落差（审核报告 P2-1）。两个预算的语义分工（不得相乘、不得合并成单闸门）：
///
/// - **本常量（每输入预算）**：一次用户输入触发的自动续轮链最多走 8 轮，随后必须回到人类。
///   它是「别让一次输入无限自治」的安全阀，归零条件是**人类再次发话**。
/// - **`GoalData::max_goal_rounds`（目标预算，默认 50）**：目标**跨多次输入**累计的成功轮次上限。
///   它是「这个目标值不值得继续烧」的预算，归零条件是**目标完成/放弃**。
///
/// 两者同时生效、取先到者：8 轮内没推进完 ⇒ 停下等人类（目标仍 Armed，说句话即可继续）；
/// 累计到 50 轮 ⇒ 目标落 `blocked(round_limit)`（`resume` 会明确拒绝并给出原因）。
///
/// 单源约定：`pony_agent_core::agent::planner::MAX_CONSECUTIVE_AUTO_CONTINUES` 必须引用本常量
/// （core 依赖 storage 的方向合法，反向不合法），禁止另立字面量。
pub const MAX_AUTO_CONTINUES_PER_INPUT: usize = 8;

/// O7：双预算的**唯一**展示文案（`/goal show` 与桌面 GoalBar 共用，避免两端各写一套说法）。
///
/// 返回 `(每输入预算行, 目标预算行)`：
/// - 每输入行恒定（与具体目标无关），说明「一次输入最多自动续 8 轮」；
/// - 目标行带当前进度（`rounds_started / max_goal_rounds`）与「跨多次输入累计」的口径。
#[must_use]
pub fn budget_summary_lines(goal: &GoalData) -> (String, String) {
    (
        format!(
            "自动续轮: 每次输入最多 {MAX_AUTO_CONTINUES_PER_INPUT} 轮（之后等你发话，目标保持激活）"
        ),
        format!(
            "目标预算: {}/{} 轮（跨多次输入累计；用满后目标转 blocked(round_limit)）",
            goal.rounds_started, goal.max_goal_rounds
        ),
    )
}

/// O5：预算耗尽时的稳定阻塞原因（`GoalData::blocked_reason` 的取值，UI 与 `resume` 指引共用）。
pub const GOAL_BLOCKED_REASON_ROUND_LIMIT: &str = "round_limit";

/// O5：预算耗尽 ⇒ 落 `blocked`（而不是静默 `return`）。
///
/// 旧实现里 `rounds_started >= max_goal_rounds` 只是不再续轮，目标仍停在 `active + Armed`：
/// UI 永久显示「进行中」，`resume` 又被「预算耗尽」拒绝（`goal.rs` 的 Resume 分支）⇒ 死局
/// 且没有任何解释。落 `blocked` 让相位与原因都可见，`resume` 的拒绝也有据可依。
pub fn block_goal_for_round_limit(goal_id: &str) -> Result<GoalData, String> {
    let target = goal_id.trim();
    if target.is_empty() {
        return Err("Goal ID cannot be empty".to_string());
    }

    let mut guard = goal_store_guard();
    ensure_goal_store_loaded(&mut guard);
    let goals = guard.as_mut().expect("goal store must be loaded");
    let session = goals
        .iter()
        .find(|(_, goal)| goal.id == target)
        .map(|(session, _)| session.clone())
        .ok_or_else(|| format!("No goal found for goal id `{target}`"))?;
    let previous = goals.get(&session).cloned();
    let updated = {
        let goal = goals
            .get_mut(&session)
            .expect("goal must exist after id lookup");
        goal.phase = "blocked".to_string();
        goal.activation = GoalActivation::Disarmed;
        goal.blocked_reason = Some(GOAL_BLOCKED_REASON_ROUND_LIMIT.to_string());
        goal.clone()
    };

    if let Err(error) = persist_goal_store(goals, &[]) {
        match previous {
            Some(previous) => {
                goals.insert(session, previous);
            }
            None => {
                goals.remove(&session);
            }
        }
        return Err(error);
    }

    // B130：先释放写锁再广播（观察者可能回调本模块读面）。
    drop(guard);
    notify_goal_event(&GoalEvent::Changed {
        goal: updated.clone(),
    });
    Ok(updated)
}

/// 只读回读某 goal 的当前快照（不推进预算、不落盘）；不存在 ⇒ `Err`。
fn goal_snapshot(goal_id: &str) -> Result<GoalData, String> {
    let target = goal_id.trim();
    if target.is_empty() {
        return Err("Goal ID cannot be empty".to_string());
    }

    let mut guard = goal_store_guard();
    ensure_goal_store_loaded(&mut guard);
    let goals = guard.as_ref().expect("goal store must be loaded");
    goals
        .values()
        .find(|goal| goal.id == target)
        .cloned()
        .ok_or_else(|| format!("No goal found for goal id `{target}`"))
}

/// 契约 C1：会话 id 是 goal 的**唯一身份**，禁止 `default` 兜底。
///
/// 旧实现把 `None` 折叠成字面量 `"default"`，于是所有缺少会话事实的读取方（宿主续轮、
/// TUI 轮次线程、工具回退路径）共享同一个桶：A 会话的目标会被 B 会话读出来继续跑。
/// fail-closed：无会话 id ⇒ 显式 `Err`，由调用方决定「不续轮 / 报错」，
/// 「借用别的会话的目标」不再是一种可能。
fn resolve_session(session_id: Option<String>, entry: &str) -> Result<String, String> {
    session_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            format!(
                "{entry} requires a non-empty session_id (goal is session-scoped; `default` fallback removed)"
            )
        })
}

pub fn create_goal(args: CreateGoalArgs) -> Result<CreateGoalResult, String> {
    let session = resolve_session(args.session_id, "create_goal")?;
    let mut guard = goal_store_guard();
    ensure_goal_store_loaded(&mut guard);
    let goals = guard.as_mut().expect("goal store must be loaded");

    if let Some(existing) = goals.get(&session) {
        if existing.phase != "completed" {
            return Err(format!(
                "Goal already exists for session {session} in phase '{}'; cannot overwrite in-progress goal",
                existing.phase
            ));
        }
    }

    let goal_id = format!("goal-{}", Uuid::new_v4());
    let goal = GoalData {
        id: goal_id,
        session_id: session.clone(),
        objective: args.objective,
        phase: "active".to_string(),
        activation: GoalActivation::Armed,
        revision: 1,
        rounds_started: 0,
        max_goal_rounds: args.max_goal_rounds.unwrap_or(50),
        blocked_reason: None,
    };

    let previous = goals.get(&session).cloned();
    goals.insert(session.clone(), goal.clone());

    if let Err(error) = persist_goal_store(goals, &[]) {
        match previous {
            Some(prev) => {
                goals.insert(session, prev);
            }
            None => {
                goals.remove(&session);
            }
        }
        return Err(error);
    }

    // B130：create 也是「目标变了」——GoalBar 需在不刷新整轮的情况下即时更新。
    drop(guard);
    notify_goal_event(&GoalEvent::Changed { goal: goal.clone() });
    Ok(CreateGoalResult { goal })
}

pub fn get_goal(args: GetGoalArgs) -> Result<GetGoalResult, String> {
    let session = resolve_session(args.session_id, "get_goal")?;
    let mut guard = goal_store_guard();
    ensure_goal_store_loaded(&mut guard);
    let goals = guard.as_ref().expect("goal store must be loaded");
    Ok(GetGoalResult {
        goal: goals.get(&session).cloned(),
    })
}

pub fn update_goal(args: UpdateGoalArgs) -> Result<UpdateGoalResult, String> {
    let session = resolve_session(args.session_id, "update_goal")?;
    let mut guard = goal_store_guard();
    ensure_goal_store_loaded(&mut guard);
    let goals = guard.as_mut().expect("goal store must be loaded");
    let previous = goals.get(&session).cloned();

    let updated = {
        let current = goals
            .get_mut(&session)
            .ok_or_else(|| "No goal found for this session".to_string())?;

        if current.id != args.goal_id {
            return Err("Goal ID mismatch".to_string());
        }

        if current.revision != args.revision {
            return Err(format!(
                "Stale revision: expected {}, got {}",
                current.revision, args.revision
            ));
        }

        match args.action {
            UpdateGoalAction::Pause => {
                if current.phase != "active" {
                    return Err(format!("Cannot pause goal in phase '{}'", current.phase));
                }
                current.phase = "paused".to_string();
                current.activation = GoalActivation::Disarmed;
            }
            UpdateGoalAction::Resume => {
                if current.phase == "completed" {
                    return Err("Cannot resume a completed goal".to_string());
                }
                if current.rounds_started >= current.max_goal_rounds {
                    return Err(format!(
                        "Cannot resume goal: rounds budget exhausted ({}/{})",
                        current.rounds_started, current.max_goal_rounds
                    ));
                }
                current.phase = "active".to_string();
                current.activation = GoalActivation::Armed;
                current.blocked_reason = None;
            }
            UpdateGoalAction::Complete => {
                if current.phase == "completed" {
                    return Err("Goal is already completed".to_string());
                }
                current.phase = "completed".to_string();
                current.activation = GoalActivation::Disarmed;
                current.blocked_reason = None;
            }
            UpdateGoalAction::Blocked => {
                if current.phase == "completed" {
                    return Err("Cannot block a completed goal".to_string());
                }
                // O5：硬门槛（对齐 DSH `GOAL_TOOL_BLOCK_THRESHOLD`）。错误串前缀是稳定契约：
                // 工具层据此映射 `goal_block_threshold` 错误码，不得改成别的措辞。
                if current.rounds_started < GOAL_BLOCK_MIN_ROUNDS {
                    return Err(format!(
                        "goal_block_threshold: blocked requires at least {GOAL_BLOCK_MIN_ROUNDS} \
                         goal rounds, current rounds_started={}",
                        current.rounds_started
                    ));
                }
                let reason = args
                    .blocked_reason
                    .ok_or_else(|| "Blocked action requires blocked_reason".to_string())?;
                current.phase = "blocked".to_string();
                current.activation = GoalActivation::Disarmed;
                current.blocked_reason = Some(reason);
            }
            UpdateGoalAction::Edit => {
                if current.phase == "completed" {
                    return Err("Cannot edit a completed goal".to_string());
                }
                if let Some(obj) = args.objective {
                    current.objective = obj;
                }
                if let Some(max) = args.max_goal_rounds {
                    current.max_goal_rounds = max;
                }
            }
        }

        current.revision += 1;
        current.clone()
    };

    if let Err(error) = persist_goal_store(goals, &[]) {
        match previous {
            Some(previous) => {
                goals.insert(session, previous);
            }
            None => {
                goals.remove(&session);
            }
        }
        return Err(error);
    }

    // B130：人类命令（/goal、GoalBar 按钮）与模型工具共用本入口，事件在此统一发射。
    drop(guard);
    notify_goal_event(&GoalEvent::Changed {
        goal: updated.clone(),
    });
    Ok(UpdateGoalResult { goal: updated })
}

/// 契约 C-3：删除该 session 的目标记录并原子持久化；返回是否真的删掉了记录。
///
/// - `session_id` 去空白后为空 ⇒ `Err`；
/// - 无记录 ⇒ `Ok(false)`（幂等，不落盘）；
/// - 有记录 ⇒ 内存删除 + `persist_goal_store`；落盘失败则把原记录放回并原样返回 `Err`。
///   （不新增 `UpdateGoalAction` 变体：避免枚举扩散到模型工具 schema 与 `src/types/goal.ts`。）
pub fn clear_goal(session_id: &str) -> Result<bool, String> {
    let session = session_id.trim();
    if session.is_empty() {
        return Err("Session ID cannot be empty".to_string());
    }

    let mut guard = goal_store_guard();
    ensure_goal_store_loaded(&mut guard);
    let goals = guard.as_mut().expect("goal store must be loaded");
    let previous = match goals.remove(session) {
        Some(goal) => goal,
        None => return Ok(false),
    };

    // 显式把被删 session 传给持久化层：磁盘上若仍有该条目（另一进程写过盘），必须一并移除。
    let removed_goal_id = previous.id.clone();
    if let Err(error) = persist_goal_store(goals, &[session.to_string()]) {
        goals.insert(session.to_string(), previous);
        return Err(error);
    }

    // B130：clear 是「目标消失」而非「目标变成某相位」，故用独立的 Cleared 事实
    // （桌面线缆事件名见 `src-tauri/src/goal_events.rs`），不复用 Changed 的 GoalData 载荷。
    drop(guard);
    notify_goal_event(&GoalEvent::Cleared {
        session_id: session.to_string(),
        goal_id: Some(removed_goal_id),
    });
    Ok(true)
}

/// 进程级 goal store 的测试串行化接缝（lib 单测跨模块共享同一 store，必须互斥）。
#[doc(hidden)]
pub mod test_support {
    use std::sync::Mutex;

    static GOAL_STORE_TEST_LOCK: Mutex<()> = Mutex::new(());

    pub fn lock() -> std::sync::MutexGuard<'static, ()> {
        GOAL_STORE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 独占 goal sidecar 的测试夹具：把 `PONY_AGENT_GOAL_STORE_PATH` 指向本次用例的临时
    /// 目录，Drop 时复原 env、删目录并复位进程内缓存。
    ///
    /// 为什么必须独占：`SESSION_GOALS` 是进程级静态，多个用例并行写同一 sidecar 会互相
    /// 擦除；因此**安装者必须先持有 [`lock`]**（夹具本身不加锁，避免医生式双重加锁）。
    pub struct TempGoalStore {
        previous: Option<std::ffi::OsString>,
        dir: std::path::PathBuf,
        store_path: std::path::PathBuf,
    }

    impl TempGoalStore {
        pub fn install(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0);
            let dir = std::env::temp_dir().join(format!(
                "pony-goal-store-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).expect("create temp goal store dir");
            let store_path = dir.join("goal_store.json");
            let previous = std::env::var_os(super::GOAL_STORE_PATH_ENV);
            std::env::set_var(super::GOAL_STORE_PATH_ENV, &store_path);
            super::reset_goal_store_for_test();
            Self {
                previous,
                dir,
                store_path,
            }
        }

        pub fn path(&self) -> &std::path::Path {
            &self.store_path
        }

        pub fn dir(&self) -> &std::path::Path {
            &self.dir
        }
    }

    impl Drop for TempGoalStore {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var(super::GOAL_STORE_PATH_ENV, value),
                None => std::env::remove_var(super::GOAL_STORE_PATH_ENV),
            }
            let _ = std::fs::remove_dir_all(&self.dir);
            super::reset_goal_store_for_test();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_goal_lifecycle_and_cas() {
        let _guard = test_support::lock();
        let session = "test_session_goal_1".to_string();
        let create_res = create_goal(CreateGoalArgs {
            session_id: Some(session.clone()),
            objective: "Test Objective".to_string(),
            max_goal_rounds: Some(10),
        })
        .unwrap();

        assert_eq!(create_res.goal.revision, 1);
        assert_eq!(create_res.goal.phase, "active");
        assert_eq!(create_res.goal.activation, GoalActivation::Armed);

        let get_res = get_goal(GetGoalArgs {
            session_id: Some(session.clone()),
        })
        .unwrap();
        assert_eq!(get_res.goal.unwrap().id, create_res.goal.id);

        // Stale revision test
        let stale_err = update_goal(UpdateGoalArgs {
            session_id: Some(session.clone()),
            goal_id: create_res.goal.id.clone(),
            revision: 99,
            action: UpdateGoalAction::Pause,
            objective: None,
            blocked_reason: None,
            max_goal_rounds: None,
        });
        assert!(stale_err.is_err());

        // Pause
        let pause_res = update_goal(UpdateGoalArgs {
            session_id: Some(session.clone()),
            goal_id: create_res.goal.id.clone(),
            revision: 1,
            action: UpdateGoalAction::Pause,
            objective: None,
            blocked_reason: None,
            max_goal_rounds: None,
        })
        .unwrap();
        assert_eq!(pause_res.goal.revision, 2);
        assert_eq!(pause_res.goal.phase, "paused");
        assert_eq!(pause_res.goal.activation, GoalActivation::Disarmed);

        // Blocked requires reason（O5：先满足最小轮次门槛，否则会在门槛处先失败，
        // 那样这条断言就不再校验「缺 reason」本身，故先把 rounds_started 推到门槛）
        for _ in 0..GOAL_BLOCK_MIN_ROUNDS {
            advance_goal_round(&create_res.goal.id).expect("advance to block threshold");
        }
        let block_without_reason = update_goal(UpdateGoalArgs {
            session_id: Some(session.clone()),
            goal_id: create_res.goal.id.clone(),
            revision: 2,
            action: UpdateGoalAction::Blocked,
            objective: None,
            blocked_reason: None,
            max_goal_rounds: None,
        });
        assert!(block_without_reason.is_err());

        // Blocked with reason
        let block_res = update_goal(UpdateGoalArgs {
            session_id: Some(session.clone()),
            goal_id: create_res.goal.id.clone(),
            revision: 2,
            action: UpdateGoalAction::Blocked,
            objective: None,
            blocked_reason: Some("Rate limited".to_string()),
            max_goal_rounds: None,
        })
        .unwrap();
        assert_eq!(block_res.goal.phase, "blocked");
        assert_eq!(block_res.goal.blocked_reason.as_deref(), Some("Rate limited"));
        assert_eq!(block_res.goal.activation, GoalActivation::Disarmed);

        // Resume
        let resume_res = update_goal(UpdateGoalArgs {
            session_id: Some(session.clone()),
            goal_id: create_res.goal.id.clone(),
            revision: 3,
            action: UpdateGoalAction::Resume,
            objective: None,
            blocked_reason: None,
            max_goal_rounds: None,
        })
        .unwrap();
        assert_eq!(resume_res.goal.phase, "active");
        assert_eq!(resume_res.goal.activation, GoalActivation::Armed);
        assert!(resume_res.goal.blocked_reason.is_none());

        // Complete
        let complete_res = update_goal(UpdateGoalArgs {
            session_id: Some(session.clone()),
            goal_id: create_res.goal.id.clone(),
            revision: 4,
            action: UpdateGoalAction::Complete,
            objective: None,
            blocked_reason: None,
            max_goal_rounds: None,
        })
        .unwrap();
        assert_eq!(complete_res.goal.phase, "completed");
        assert_eq!(complete_res.goal.activation, GoalActivation::Disarmed);

        // B127 守卫断言：completed 状态为终态，不可复活、不可再次完成、不可 pause/edit/block
        let resume_completed = update_goal(UpdateGoalArgs {
            session_id: Some(session.clone()),
            goal_id: create_res.goal.id.clone(),
            revision: 5,
            action: UpdateGoalAction::Resume,
            objective: None,
            blocked_reason: None,
            max_goal_rounds: None,
        });
        assert!(resume_completed.is_err(), "Resume 必须拒绝复活已完成目标");

        let pause_completed = update_goal(UpdateGoalArgs {
            session_id: Some(session.clone()),
            goal_id: create_res.goal.id.clone(),
            revision: 5,
            action: UpdateGoalAction::Pause,
            objective: None,
            blocked_reason: None,
            max_goal_rounds: None,
        });
        assert!(pause_completed.is_err(), "Pause 必须拒绝暂停已完成目标");

        let complete_again = update_goal(UpdateGoalArgs {
            session_id: Some(session.clone()),
            goal_id: create_res.goal.id.clone(),
            revision: 5,
            action: UpdateGoalAction::Complete,
            objective: None,
            blocked_reason: None,
            max_goal_rounds: None,
        });
        assert!(complete_again.is_err(), "Complete 必须拒绝再次完成已完成目标");

        // B127 守卫断言：因该 session 上目标已 completed，此时允许创建新目标覆盖
        let recreate_res = create_goal(CreateGoalArgs {
            session_id: Some(session.clone()),
            objective: "Second Goal".to_string(),
            max_goal_rounds: Some(5),
        });
        assert!(recreate_res.is_ok(), "当既有目标为 completed 时允许创建新目标覆盖");

        // 当既有目标处于 active 时，再次 create_goal 必须被拒绝（防静默覆盖）
        let overwrite_active = create_goal(CreateGoalArgs {
            session_id: Some(session.clone()),
            objective: "Third Goal (should fail)".to_string(),
            max_goal_rounds: Some(5),
        });
        assert!(overwrite_active.is_err(), "create_goal 必须拒绝覆盖进行中目标");
    }
}

// ===========================================================================
// Wave-1 green 单测（C1/C2/C3 落点自证；验收测试见 tests/wave_plan_goal_acceptance.rs）
// ===========================================================================

#[cfg(test)]
mod wave1_tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// env + 临时目录兜底清理（panic 路径同样生效），并复位进程内缓存。
    struct GoalStoreEnvGuard {
        previous: Option<std::ffi::OsString>,
        dir: PathBuf,
    }

    impl Drop for GoalStoreEnvGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var(GOAL_STORE_PATH_ENV, value),
                None => std::env::remove_var(GOAL_STORE_PATH_ENV),
            }
            let _ = fs::remove_dir_all(&self.dir);
            // 复位全局缓存，避免本用例的目标泄漏给同进程其他单测。
            reset_goal_store_for_test();
        }
    }

    fn install_temp_store(tag: &str) -> (GoalStoreEnvGuard, PathBuf) {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "pony-goal-unit-{tag}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp goal store dir");
        let store_path = dir.join("goal_store.json");
        let guard = GoalStoreEnvGuard {
            previous: std::env::var_os(GOAL_STORE_PATH_ENV),
            dir,
        };
        std::env::set_var(GOAL_STORE_PATH_ENV, &store_path);
        reset_goal_store_for_test();
        (guard, store_path)
    }

    #[test]
    fn wave1_active_goal_requires_armed_active_and_exact_session() {
        let _guard = test_support::lock();
        let (_env, _path) = install_temp_store("active");
        let session = "wave1-unit-active";
        let created = create_goal(CreateGoalArgs {
            session_id: Some(session.to_string()),
            objective: "把报告写完".to_string(),
            max_goal_rounds: Some(10),
        })
        .expect("create_goal")
        .goal;

        let hit = active_goal_for_session(session).expect("armed active goal must hit");
        assert_eq!(hit.id, created.id);
        assert_eq!(hit.objective, "把报告写完");
        assert!(active_goal_for_session("wave1-unit-active-prefix").is_none());
        assert!(active_goal_for_session("wave1-unit-never-created").is_none());

        update_goal(UpdateGoalArgs {
            session_id: Some(session.to_string()),
            goal_id: created.id.clone(),
            revision: created.revision,
            action: UpdateGoalAction::Pause,
            objective: None,
            blocked_reason: None,
            max_goal_rounds: None,
        })
        .expect("pause goal");
        assert!(active_goal_for_session(session).is_none());
    }

    /// 回归：`clear_goal` 必须能穿透跨进程合并。
    ///
    /// 旧实现 `persist_goal_store` 的合并只做 `insert`，磁盘上仍存在的同 session 条目会被
    /// 重新插回，`clear` 被静默复活 ⇒ 本用例在旧实现下 `get_goal(...).goal` 为 `Some`（失败）。
    #[test]
    fn clear_goal_removes_entry_even_when_disk_still_has_it() {
        let _guard = test_support::lock();
        let (_env, store_path) = install_temp_store("clear-cross-process");
        let session = "wave1-unit-clear-cross-process";

        let created = create_goal(CreateGoalArgs {
            session_id: Some(session.to_string()),
            objective: "要被清掉的目标".to_string(),
            max_goal_rounds: Some(9),
        })
        .expect("create_goal")
        .goal;

        // 模拟「另一进程」把含该 session 的合法 JSON 写回磁盘：此时本进程内存仍持有该条目。
        let foreign: HashMap<String, GoalData> =
            serde_json::from_str(&fs::read_to_string(&store_path).expect("read sidecar"))
                .expect("parse sidecar");
        assert_eq!(foreign.get(session).map(|g| g.id.as_str()), Some(created.id.as_str()));
        fs::write(
            &store_path,
            serde_json::to_string_pretty(&foreign).expect("serialize sidecar"),
        )
        .expect("rewrite sidecar");

        assert!(
            clear_goal(session).expect("clear_goal"),
            "clear_goal must report a real removal"
        );

        reset_goal_store_for_test();
        let after = get_goal(GetGoalArgs {
            session_id: Some(session.to_string()),
        })
        .expect("get_goal after reset")
        .goal;
        assert!(
            after.is_none(),
            "clear_goal must survive cross-process merge (disk still had the entry)"
        );
        assert!(!store_path.exists() || !fs::read_to_string(&store_path)
            .expect("read sidecar")
            .contains(&created.id), "cleared goal must not linger in the sidecar");
    }

    #[test]
    fn wave1_goal_store_persists_across_reset_and_degrades_on_corrupt_json() {
        let _guard = test_support::lock();
        let (_env, store_path) = install_temp_store("persist");
        let session = "wave1-unit-persist";

        let created = create_goal(CreateGoalArgs {
            session_id: Some(session.to_string()),
            objective: "把报告写完".to_string(),
            max_goal_rounds: Some(7),
        })
        .expect("create_goal")
        .goal;
        assert!(store_path.exists(), "create_goal must write the JSON sidecar");

        reset_goal_store_for_test();
        let reloaded = get_goal(GetGoalArgs {
            session_id: Some(session.to_string()),
        })
        .expect("get_goal after reset")
        .goal
        .expect("goal must survive reset (sidecar reload)");
        assert_eq!(reloaded.id, created.id);
        assert_eq!(reloaded.objective, created.objective);
        assert_eq!(reloaded.phase, "active");
        assert_eq!(reloaded.activation, GoalActivation::Armed);
        assert_eq!(reloaded.revision, created.revision);
        assert_eq!(reloaded.rounds_started, created.rounds_started);
        assert_eq!(reloaded.max_goal_rounds, 7);

        fs::write(&store_path, b"{ broken-json, ").expect("write corrupt sidecar");
        reset_goal_store_for_test();
        let degraded = get_goal(GetGoalArgs {
            session_id: Some(session.to_string()),
        })
        .expect("corrupt sidecar must not error");
        assert!(degraded.goal.is_none(), "corrupt sidecar must degrade to empty");
    }

    #[test]
    fn wave1_build_goal_round_message_is_frozen_template() {
        let message = build_goal_round_message("把报告写完", 3, 10);
        assert_eq!(
            message,
            "<goal_round round=\"3\" max=\"10\">继续推进目标：把报告写完。以工作区与工具结果为准；仅当目标真正达成才 update_goal complete。若本轮仍有未完成步骤，直接继续执行，不要询问用户。</goal_round>"
        );
    }

    #[test]
    fn wave1_advance_goal_round_increments_and_persists() {
        let _guard = test_support::lock();
        let (_env, _path) = install_temp_store("advance");
        let session = "wave1-unit-advance";
        let created = create_goal(CreateGoalArgs {
            session_id: Some(session.to_string()),
            objective: "把报告写完".to_string(),
            max_goal_rounds: Some(10),
        })
        .expect("create_goal")
        .goal;

        let advanced = advance_goal_round(&created.id).expect("advance round");
        assert_eq!(advanced.rounds_started, 1);
        let advanced_again = advance_goal_round(&created.id).expect("advance round twice");
        assert_eq!(advanced_again.rounds_started, 2);
        assert_eq!(advanced_again.revision, created.revision, "advance must not bump revision");

        reset_goal_store_for_test();
        let reloaded = get_goal(GetGoalArgs {
            session_id: Some(session.to_string()),
        })
        .expect("get_goal")
        .goal
        .expect("goal survives reset");
        assert_eq!(reloaded.rounds_started, 2);

        assert!(advance_goal_round("goal-does-not-exist").is_err());
    }

    #[test]
    fn wave1_corrupt_file_prevents_overwrite_and_rollback_restores_previous() {
        let _guard = test_support::lock();
        let (_env, store_path) = install_temp_store("rollback-degrade");
        let session = "wave1-unit-degrade";

        let _created = create_goal(CreateGoalArgs {
            session_id: Some(session.to_string()),
            objective: "初始目标".to_string(),
            max_goal_rounds: Some(5),
        })
        .expect("initial create_goal")
        .goal;

        // 模拟损坏文件
        let corrupt_bytes = b"corrupted-not-json-content";
        fs::write(&store_path, corrupt_bytes).expect("write corrupt bytes");

        // 损坏后 reset，store 应当判定为 degraded
        reset_goal_store_for_test();
        assert!(GOAL_STORE_DEGRADED.load(std::sync::atomic::Ordering::SeqCst));

        // 尝试新建目标，必须拒绝覆写磁盘损坏数据
        let res = create_goal(CreateGoalArgs {
            session_id: Some(session.to_string()),
            objective: "尝试覆写目标".to_string(),
            max_goal_rounds: Some(10),
        });
        assert!(res.is_err(), "create_goal 必须拒绝覆写已降级的磁盘");

        // 磁盘内容应完全保留损坏现场，未被覆写
        let disk_content = fs::read(&store_path).expect("read store_path");
        assert_eq!(disk_content, corrupt_bytes, "磁盘内容不得被静默冲掉");
    }

    /// 跨会话串话根因回归：`session_id` 缺失时**不得**折叠进 `default` 桶。
    ///
    /// 旧实现把 `None` 变成 `"default"`，A 会话的 goal 会被任何无会话事实的读取方
    /// （宿主续轮、TUI 轮次线程）读出来继续跑。fail-closed 后必须显式报错。
    #[test]
    fn missing_session_id_is_rejected_instead_of_falling_back_to_default() {
        let _guard = test_support::lock();
        let (_env, _store_path) = install_temp_store("fail-closed-session");

        let create_err = create_goal(CreateGoalArgs {
            session_id: None,
            objective: "不得落进 default 桶".to_string(),
            max_goal_rounds: Some(3),
        })
        .expect_err("create_goal 无 session_id 必须失败");
        assert!(
            create_err.contains("session_id"),
            "错误信息必须指明 session_id 缺失：{create_err}"
        );

        let get_err = get_goal(GetGoalArgs { session_id: None })
            .expect_err("get_goal 无 session_id 必须失败");
        assert!(get_err.contains("session_id"), "{get_err}");

        let update_err = update_goal(UpdateGoalArgs {
            session_id: None,
            goal_id: "goal-x".to_string(),
            revision: 1,
            action: UpdateGoalAction::Pause,
            objective: None,
            blocked_reason: None,
            max_goal_rounds: None,
        })
        .expect_err("update_goal 无 session_id 必须失败");
        assert!(update_err.contains("session_id"), "{update_err}");

        // 空串/纯空白同样视为「无会话事实」。
        let blank_err = create_goal(CreateGoalArgs {
            session_id: Some("   ".to_string()),
            objective: "空白会话 id".to_string(),
            max_goal_rounds: None,
        })
        .expect_err("空白 session_id 必须失败");
        assert!(blank_err.contains("session_id"), "{blank_err}");

        // 关键断言：没有任何记录落进 `default` 桶（否则读取方仍会串话）。
        assert!(
            active_goal_for_session("default").is_none(),
            "fail-closed 后不得再产生 default 桶条目"
        );
    }

    /// B152 存量迁移：旧 `default` 兜底留下的条目必须在加载时清掉，且不因下一次写盘复活。
    #[test]
    fn legacy_default_entries_are_purged_on_load() {
        let _guard = test_support::lock();
        let (_env, store_path) = install_temp_store("legacy-purge");

        let legacy_goal = |id: &str, session_id: &str| GoalData {
            id: id.to_string(),
            session_id: session_id.to_string(),
            objective: "旧兜底目标".to_string(),
            phase: "active".to_string(),
            activation: GoalActivation::Armed,
            revision: 1,
            rounds_started: 0,
            max_goal_rounds: 5,
            blocked_reason: None,
        };

        let mut seeded: HashMap<String, GoalData> = HashMap::new();
        // 三种旧兜底产物：字面量 key、空白 key、key 真实但内嵌 session_id 仍是兜底值。
        seeded.insert(
            "default".to_string(),
            legacy_goal("goal-legacy-default", "default"),
        );
        seeded.insert("   ".to_string(), legacy_goal("goal-legacy-blank", "   "));
        seeded.insert(
            "pa-legacy-mixed".to_string(),
            legacy_goal("goal-legacy-mixed", "default"),
        );
        // 真实会话条目：必须原样保留。
        seeded.insert(
            "pa-legacy-kept".to_string(),
            legacy_goal("goal-legacy-kept", "pa-legacy-kept"),
        );
        fs::write(
            &store_path,
            serde_json::to_string_pretty(&seeded).expect("serialize seed"),
        )
        .expect("seed legacy store");

        reset_goal_store_for_test();

        // 读面：旧兜底条目不可达，真实会话条目可读且仍是 armed/active。
        assert!(
            active_goal_for_session("pa-legacy-kept").is_some(),
            "真实会话条目不得被迁移误删"
        );
        assert!(goal_snapshot("goal-legacy-default").is_err());
        assert!(goal_snapshot("goal-legacy-mixed").is_err());

        // 磁盘面：回写已把旧 key 从 sidecar 删除（不是仅内存屏蔽）。
        let disk: HashMap<String, GoalData> =
            serde_json::from_str(&fs::read_to_string(&store_path).expect("read store"))
                .expect("parse store");
        assert_eq!(
            disk.keys().collect::<Vec<_>>(),
            vec![&"pa-legacy-kept".to_string()],
            "磁盘应只剩真实会话条目"
        );

        // 复活回归：再触发一次普通写盘（推进轮次走 `explicit_removals = &[]`），
        // 跨进程序合并不得把旧 key 从磁盘插回。
        let kept = disk["pa-legacy-kept"].clone();
        advance_goal_round(&kept.id).expect("advance kept goal");
        let after: HashMap<String, GoalData> =
            serde_json::from_str(&fs::read_to_string(&store_path).expect("read store"))
                .expect("parse store");
        assert!(
            !after.values().any(|goal| goal.session_id == "default"),
            "旧兜底条目不得复活：{:?}",
            after.keys().collect::<Vec<_>>()
        );
    }
}

/// B130：领域事件广播（`GoalEvent` / `install_goal_event_observer`）的真值测试。
///
/// 宿主事件 `goal://*` 的**发射时机**在这里被锁死：create / update / advance / clear 四条
/// 写入路径各广播什么、clear 幂等时不广播。桌面层只需把同一批事实映射成线缆载荷
/// （`src-tauri/src/goal_events.rs` 的形状测试覆盖那一半）。
#[cfg(test)]
mod b130_event_tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};

    /// 收集进程级广播；只断言本用例 session 的事件（并行用例可能同时写别的 session）。
    struct EventLog {
        events: Arc<StdMutex<Vec<GoalEvent>>>,
    }

    impl EventLog {
        fn install() -> Self {
            let events: Arc<StdMutex<Vec<GoalEvent>>> = Arc::new(StdMutex::new(Vec::new()));
            let sink = Arc::clone(&events);
            install_goal_event_observer(Arc::new(move |event: &GoalEvent| {
                sink.lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(event.clone());
            }));
            Self { events }
        }

        fn of_session(&self, session: &str) -> Vec<GoalEvent> {
            self.events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .filter(|event| match event {
                    GoalEvent::Changed { goal } | GoalEvent::RoundStarted { goal } => {
                        goal.session_id == session
                    }
                    GoalEvent::Cleared { session_id, .. } => session_id == session,
                    GoalEvent::RoundsStopped { session_id, .. } => session_id == session,
                })
                .cloned()
                .collect()
        }
    }

    impl Drop for EventLog {
        fn drop(&mut self) {
            reset_goal_event_observer_for_test();
        }
    }

    #[test]
    fn goal_writes_broadcast_events_after_persist_b130() {
        let _lock = test_support::lock();
        let _store = test_support::TempGoalStore::install("b130-events");
        let log = EventLog::install();
        let session = "b130-events-session";

        let created = create_goal(CreateGoalArgs {
            session_id: Some(session.to_string()),
            objective: "把事件发射补上".to_string(),
            max_goal_rounds: Some(5),
        })
        .expect("create_goal");

        let events = log.of_session(session);
        assert_eq!(events.len(), 1, "create 只广播一条 Changed: {events:?}");
        match &events[0] {
            GoalEvent::Changed { goal } => {
                assert_eq!(goal.id, created.goal.id);
                assert_eq!(goal.phase, "active");
            }
            other => panic!("create 必须广播 Changed，实际: {other:?}"),
        }

        advance_goal_round(&created.goal.id).expect("advance_goal_round");
        let events = log.of_session(session);
        assert_eq!(
            events.len(),
            3,
            "advance 需同时广播 Changed 与 RoundStarted: {events:?}"
        );
        assert!(matches!(&events[1], GoalEvent::Changed { .. }));
        match &events[2] {
            GoalEvent::RoundStarted { goal } => assert_eq!(goal.rounds_started, 1),
            other => panic!("advance 必须广播 RoundStarted，实际: {other:?}"),
        }

        update_goal(UpdateGoalArgs {
            session_id: Some(session.to_string()),
            goal_id: created.goal.id.clone(),
            revision: created.goal.revision,
            action: UpdateGoalAction::Complete,
            objective: None,
            blocked_reason: None,
            max_goal_rounds: None,
        })
        .expect("update_goal complete");
        let events = log.of_session(session);
        match events.last() {
            Some(GoalEvent::Changed { goal }) => assert_eq!(goal.phase, "completed"),
            other => panic!("update 必须广播 Changed(completed)，实际: {other:?}"),
        }

        assert!(clear_goal(session).expect("clear_goal"));
        let events = log.of_session(session);
        match events.last() {
            Some(GoalEvent::Cleared {
                session_id,
                goal_id,
            }) => {
                assert_eq!(session_id, session);
                assert_eq!(goal_id.as_ref(), Some(&created.goal.id));
            }
            other => panic!("clear 必须广播 Cleared，实际: {other:?}"),
        }

        // 幂等 no-op 不得广播（否则前端会为「本来就没有目标」刷新一次）。
        let before = events.len();
        assert!(!clear_goal(session).expect("idempotent clear"));
        assert_eq!(
            log.of_session(session).len(),
            before,
            "无删除的 clear 不得广播"
        );
    }

    /// 观察者缺席时写入路径照常成功（core 单测 / TUI / CLI 都不装观察者）。
    #[test]
    fn writes_succeed_without_observer_b130() {
        let _lock = test_support::lock();
        let _store = test_support::TempGoalStore::install("b130-no-observer");
        reset_goal_event_observer_for_test();
        let session = "b130-no-observer-session";

        create_goal(CreateGoalArgs {
            session_id: Some(session.to_string()),
            objective: "没有观察者也要能写".to_string(),
            max_goal_rounds: Some(2),
        })
        .expect("create_goal without observer");
        assert!(clear_goal(session).expect("clear_goal without observer"));
    }

    fn all_stop_reasons_for_test() -> Vec<GoalRoundStopReason> {
        vec![
            GoalRoundStopReason::Cancelled,
            GoalRoundStopReason::ConsecutiveFailures { failures: 3 },
            GoalRoundStopReason::BudgetExhausted {
                rounds_started: 5,
                max_rounds: 5,
            },
            GoalRoundStopReason::AutoContinueBudget,
            GoalRoundStopReason::TurnSuspended,
            GoalRoundStopReason::TurnUnclassified,
            GoalRoundStopReason::LoopAlreadyOwned,
            GoalRoundStopReason::BudgetPersistFailed,
            GoalRoundStopReason::NoActiveGoal,
            GoalRoundStopReason::FirstRoundNotSucceeded,
        ]
    }

    #[test]
    fn code_values_are_unique_and_snake_case() {
        let reasons = all_stop_reasons_for_test();
        assert_eq!(reasons.len(), 10);
        let mut codes = std::collections::HashSet::new();

        for reason in &reasons {
            let code = reason.code();
            assert!(!code.is_empty(), "code 不得为空");
            assert!(
                codes.insert(code),
                "code 必须全局唯一，发现重复: {code}"
            );
            assert!(
                code.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "code 必须为全小写 snake_case: {code}"
            );
            assert!(
                !code.starts_with('_') && !code.ends_with('_'),
                "code 不得以下划线开头或结尾: {code}"
            );
            assert!(
                !code.contains("__"),
                "code 不得包含连续下划线: {code}"
            );
        }
    }

    #[test]
    fn first_round_not_succeeded_says_not_started() {
        let reason = GoalRoundStopReason::FirstRoundNotSucceeded;
        let line = reason.visible_line();
        assert!(
            line.contains("未启动"),
            "FirstRoundNotSucceeded visible_line 必须包含「未启动」: {line}"
        );
        assert!(
            !line.contains("已停止"),
            "FirstRoundNotSucceeded visible_line 不得包含「已停止」: {line}"
        );
    }

    #[test]
    fn visible_line_never_says_in_progress() {
        for reason in all_stop_reasons_for_test() {
            let line = reason.visible_line();
            assert!(
                !line.contains("进行中"),
                "停止原因 visible_line 不得包含「进行中」: {line}"
            );
        }
    }

    #[test]
    fn loop_already_owned_does_not_enter_history() {
        let reasons = all_stop_reasons_for_test();
        let mut false_count = 0;
        let mut true_count = 0;

        for reason in &reasons {
            if reason.enters_history() {
                true_count += 1;
            } else {
                false_count += 1;
                assert_eq!(
                    *reason,
                    GoalRoundStopReason::LoopAlreadyOwned,
                    "enters_history 为 false 的变体只能是 LoopAlreadyOwned"
                );
            }
        }

        assert_eq!(false_count, 1, "必须恰好 1 个 false");
        assert_eq!(true_count, 9, "必须恰好 9 个 true");
        assert!(!GoalRoundStopReason::LoopAlreadyOwned.enters_history());
    }

    #[test]
    fn visible_line_starts_with_headline() {
        for reason in all_stop_reasons_for_test() {
            let line = reason.visible_line();
            let headline = reason.headline();
            assert!(
                line.starts_with(headline),
                "visible_line 必须以 headline 开头: {line}"
            );

            let segments: Vec<&str> = line.split('｜').collect();
            assert_eq!(segments[0], headline);

            if let Some(detail) = reason.detail() {
                assert!(
                    segments.contains(&detail.as_str()),
                    "visible_line 必须包含 detail 作为独立段: {line}"
                );
            }
            if let Some(hint) = reason.action_hint() {
                assert!(
                    segments.contains(&hint),
                    "visible_line 必须包含 action_hint 作为独立段: {line}"
                );
            }
        }
    }

    #[test]
    fn severity_is_explicit_not_keyword_derived() {
        assert_eq!(
            GoalRoundStopReason::FirstRoundNotSucceeded.severity(),
            GoalStopSeverity::Error
        );
        assert_eq!(GoalRoundStopReason::Cancelled.severity(), GoalStopSeverity::Info);
        assert_eq!(
            GoalRoundStopReason::ConsecutiveFailures { failures: 2 }.severity(),
            GoalStopSeverity::Error
        );
        assert_eq!(
            GoalRoundStopReason::BudgetExhausted {
                rounds_started: 1,
                max_rounds: 1
            }
            .severity(),
            GoalStopSeverity::Error
        );
        assert_eq!(
            GoalRoundStopReason::AutoContinueBudget.severity(),
            GoalStopSeverity::Warning
        );
        assert_eq!(
            GoalRoundStopReason::TurnSuspended.severity(),
            GoalStopSeverity::Warning
        );
        assert_eq!(
            GoalRoundStopReason::TurnUnclassified.severity(),
            GoalStopSeverity::Warning
        );
        assert_eq!(
            GoalRoundStopReason::LoopAlreadyOwned.severity(),
            GoalStopSeverity::Info
        );
        assert_eq!(
            GoalRoundStopReason::BudgetPersistFailed.severity(),
            GoalStopSeverity::Error
        );
        assert_eq!(
            GoalRoundStopReason::NoActiveGoal.severity(),
            GoalStopSeverity::Warning
        );
    }

    #[test]
    fn banned_terms_absent() {
        for reason in all_stop_reasons_for_test() {
            if matches!(reason, GoalRoundStopReason::LoopAlreadyOwned) {
                continue;
            }
            let line = reason.visible_line();
            assert!(
                !line.contains("收口"),
                "除 LoopAlreadyOwned 外 visible_line 不得包含「收口」: {line}"
            );
            assert!(
                !line.contains("落盘"),
                "除 LoopAlreadyOwned 外 visible_line 不得包含「落盘」: {line}"
            );
        }
    }

    // ======================================================================
    // O2（2026-11-27）：续轮 gate 真值表锚定（TUI 与 core 的唯一判定点）
    // ======================================================================

    fn gate_input() -> GoalRoundGateInput {
        GoalRoundGateInput {
            has_active_goal: true,
            cancelled: false,
            pending_asks: 0,
            pending_approvals: 0,
            consecutive_failures: 0,
            rounds_started: 1,
            max_rounds: 50,
            consecutive_auto_continues: 0,
            max_auto_continues: 8,
        }
    }

    #[test]
    fn gate_stops_without_active_goal_first() {
        let input = GoalRoundGateInput {
            has_active_goal: false,
            // 即使其余判据都成立，也先报「无轮可续」——否则 UI 会说「预算用尽」而真因是无目标。
            cancelled: true,
            pending_asks: 3,
            ..gate_input()
        };
        assert_eq!(
            goal_round_gate(input),
            GoalRoundGateDecision::Stop(GoalRoundStopReason::NoActiveGoal)
        );
    }

    #[test]
    fn gate_cancelled_outranks_pending_requests() {
        let input = GoalRoundGateInput {
            cancelled: true,
            pending_asks: 1,
            ..gate_input()
        };
        assert_eq!(
            goal_round_gate(input),
            GoalRoundGateDecision::Stop(GoalRoundStopReason::Cancelled)
        );
    }

    #[test]
    fn gate_pauses_on_pending_host_requests_before_failures_and_budget() {
        for (asks, approvals) in [(1usize, 0usize), (0, 1), (2, 3)] {
            let input = GoalRoundGateInput {
                pending_asks: asks,
                pending_approvals: approvals,
                // 同时踩中失败熔断与预算用满：让位优先级更高（不起新轮，不计失败、不烧预算）。
                consecutive_failures: MAX_CONSECUTIVE_GOAL_ROUND_FAILURES,
                rounds_started: 50,
                ..gate_input()
            };
            let decision = goal_round_gate(input);
            assert!(decision.is_pause(), "asks={asks} approvals={approvals} ⇒ {decision:?}");
            assert_eq!(
                decision.stop_reason(),
                Some(GoalRoundStopReason::TurnSuspended),
                "Pause 的可见原因必须是 TurnSuspended（暂停 ≠ 终止）"
            );
        }
    }

    #[test]
    fn gate_stops_on_consecutive_failure_threshold() {
        let input = GoalRoundGateInput {
            consecutive_failures: MAX_CONSECUTIVE_GOAL_ROUND_FAILURES,
            ..gate_input()
        };
        assert_eq!(
            goal_round_gate(input),
            GoalRoundGateDecision::Stop(GoalRoundStopReason::ConsecutiveFailures {
                failures: MAX_CONSECUTIVE_GOAL_ROUND_FAILURES
            })
        );
        // 差一轮不熔断。
        assert!(goal_round_gate(GoalRoundGateInput {
            consecutive_failures: MAX_CONSECUTIVE_GOAL_ROUND_FAILURES - 1,
            ..gate_input()
        })
        .is_continue());
    }

    #[test]
    fn gate_stops_when_goal_budget_exhausted() {
        let input = GoalRoundGateInput {
            rounds_started: 50,
            max_rounds: 50,
            ..gate_input()
        };
        assert_eq!(
            goal_round_gate(input),
            GoalRoundGateDecision::Stop(GoalRoundStopReason::BudgetExhausted {
                rounds_started: 50,
                max_rounds: 50
            })
        );
    }

    #[test]
    fn gate_stops_when_auto_continue_budget_exhausted() {
        let input = GoalRoundGateInput {
            consecutive_auto_continues: 8,
            max_auto_continues: 8,
            ..gate_input()
        };
        assert_eq!(
            goal_round_gate(input),
            GoalRoundGateDecision::Stop(GoalRoundStopReason::AutoContinueBudget)
        );
        assert!(goal_round_gate(GoalRoundGateInput {
            consecutive_auto_continues: 7,
            max_auto_continues: 8,
            ..gate_input()
        })
        .is_continue());
    }

    #[test]
    fn gate_continues_on_clean_input() {
        assert!(goal_round_gate(gate_input()).is_continue());
    }

    #[test]
    fn outcome_gate_suspended_is_pause_not_stop() {
        assert_eq!(
            goal_round_outcome_gate(GoalRoundStepOutcome::Suspended),
            GoalRoundGateDecision::Pause
        );
        assert_eq!(
            goal_round_outcome_gate(GoalRoundStepOutcome::Completed),
            GoalRoundGateDecision::Continue
        );
        assert_eq!(
            goal_round_outcome_gate(GoalRoundStepOutcome::Failed),
            GoalRoundGateDecision::Stop(GoalRoundStopReason::FirstRoundNotSucceeded)
        );
        assert_eq!(
            goal_round_outcome_gate(GoalRoundStepOutcome::Cancelled),
            GoalRoundGateDecision::Stop(GoalRoundStopReason::Cancelled)
        );
        assert_eq!(
            goal_round_outcome_gate(GoalRoundStepOutcome::Unclassified),
            GoalRoundGateDecision::Stop(GoalRoundStopReason::TurnUnclassified)
        );
    }

    #[test]
    fn pause_reason_reads_as_paused_not_dead() {
        let line = GoalRoundStopReason::TurnSuspended.visible_line();
        assert!(line.contains("等你作答"), "{line}");
        assert!(line.contains("作答后自动继续"), "{line}");
        assert!(
            !line.contains("续轮已停止"),
            "暂停语义不得复用「已停止」措辞: {line}"
        );
        assert_eq!(GoalRoundStopReason::all_variants().len(), 10);
    }

    /// 挂起原因的整行文案**逐字冻结**，且同一事实只说一次。
    ///
    /// 回归现场（Ctrl+T → Goal 页签，2026-11-28 用户报「完全不知道这句的设计意图」）：
    /// `续轮暂停：等你作答｜本轮在等你作答/审批｜作答后自动继续` —— 三段里「等你作答」
    /// 出现两次，用户读到的是自我复述；而「续轮」是只有实现者才懂的内部词。
    #[test]
    fn suspended_line_states_each_fact_once() {
        let line = GoalRoundStopReason::TurnSuspended.visible_line();
        assert_eq!(
            line, "暂停中，等你作答｜作答后自动继续",
            "挂起原因的单行文案被冻结；改文案必须同改本断言与 pa171 契约矩阵"
        );
        // 「等什么」只出现一次（旧实现 headline + detail 各说一遍）。
        assert_eq!(
            line.matches("等你作答").count(),
            1,
            "「等你作答」只允许出现一次（同义复述是噪声）：{line}"
        );
        // 用户可见文案不得外泄「续轮」这类实现词（本变体之外的其他变体另行登记）。
        assert!(
            !line.contains("续轮"),
            "挂起原因不得对用户说「续轮」（内部机制词）：{line}"
        );
        // 不再枚举挂起入口：用户此刻只面对屏幕上那一个待回应面板，Ask 与审批的公共动作
        // 都是「作答」。枚举入口会让用户去找「另一个审批在哪」。
        assert!(
            !line.contains("审批"),
            "挂起原因不再枚举入口（作答即两种入口的公共动作）：{line}"
        );
    }
}

/// O2（2026-11-27）：续轮 gate 的**真值表**单测（C1 冻结判定顺序，逐行锚定）。
///
/// 为什么这些断言必须存在：`goal_round_gate` 是两个宿主循环（core 的
/// `execute_graph_run_stream` / `advance_graph_run` 与 TUI 的 `run_goal_rounds`）**唯一**的
/// 轮次边界判据。判定顺序一旦被后人调换（例如把失败熔断提到待作答之前），`ask_user` 挂起
/// 就会被当成「失败停止」，目标永久失活——正是审核报告 P0-1 的症状，且没有任何编译器或
/// 既有用例会红。故此处把冻结顺序逐条写死。
#[cfg(test)]
mod goal_round_gate_tests {
    use super::*;

    /// 全部绿灯输入：每个用例只改自己关心的字段，从而把「谁优先」变成可判定的问题。
    fn passable() -> GoalRoundGateInput {
        GoalRoundGateInput {
            has_active_goal: true,
            cancelled: false,
            pending_asks: 0,
            pending_approvals: 0,
            consecutive_failures: 0,
            rounds_started: 0,
            max_rounds: 10,
            consecutive_auto_continues: 0,
            max_auto_continues: 8,
        }
    }

    #[test]
    fn all_green_continues() {
        assert_eq!(goal_round_gate(passable()), GoalRoundGateDecision::Continue);
    }

    #[test]
    fn no_active_goal_stops_first() {
        let decision = goal_round_gate(GoalRoundGateInput {
            has_active_goal: false,
            cancelled: true,
            pending_asks: 2,
            consecutive_failures: MAX_CONSECUTIVE_GOAL_ROUND_FAILURES,
            rounds_started: 10,
            ..passable()
        });
        assert_eq!(
            decision,
            GoalRoundGateDecision::Stop(GoalRoundStopReason::NoActiveGoal),
            "无 active goal 必须压过取消/待作答/失败/预算（否则会为不存在的目标起轮）"
        );
    }

    #[test]
    fn cancelled_beats_pending_asks() {
        let decision = goal_round_gate(GoalRoundGateInput {
            cancelled: true,
            pending_asks: 1,
            ..passable()
        });
        assert_eq!(
            decision,
            GoalRoundGateDecision::Stop(GoalRoundStopReason::Cancelled),
            "用户已取消时不得再判「等你作答」（Pause 会让循环继续等待一个已放弃的目标）"
        );
    }

    #[test]
    fn pending_asks_pause_beats_failures_and_budget() {
        let decision = goal_round_gate(GoalRoundGateInput {
            pending_asks: 1,
            consecutive_failures: MAX_CONSECUTIVE_GOAL_ROUND_FAILURES + 5,
            rounds_started: 10,
            consecutive_auto_continues: 8,
            ..passable()
        });
        assert_eq!(
            decision,
            GoalRoundGateDecision::Pause,
            "待作答必须判 Pause（不是 Stop）：作答后宿主才能重入续轮"
        );
    }

    #[test]
    fn pending_approvals_pause() {
        assert_eq!(
            goal_round_gate(GoalRoundGateInput {
                pending_approvals: 1,
                ..passable()
            }),
            GoalRoundGateDecision::Pause,
            "审批待作答与 Ask 同权（都是「有人在等宿主」）"
        );
    }

    #[test]
    fn consecutive_failures_stop_with_actual_count() {
        assert_eq!(
            goal_round_gate(GoalRoundGateInput {
                consecutive_failures: MAX_CONSECUTIVE_GOAL_ROUND_FAILURES,
                rounds_started: 10,
                ..passable()
            }),
            GoalRoundGateDecision::Stop(GoalRoundStopReason::ConsecutiveFailures {
                failures: MAX_CONSECUTIVE_GOAL_ROUND_FAILURES,
            }),
            "恰好达到阈值必须停，且压过预算耗尽"
        );
        assert_eq!(
            goal_round_gate(GoalRoundGateInput {
                consecutive_failures: MAX_CONSECUTIVE_GOAL_ROUND_FAILURES + 3,
                ..passable()
            }),
            GoalRoundGateDecision::Stop(GoalRoundStopReason::ConsecutiveFailures {
                failures: MAX_CONSECUTIVE_GOAL_ROUND_FAILURES + 3,
            }),
            "失败计数必须如实（不得截断成阈值）"
        );
        assert_eq!(
            goal_round_gate(GoalRoundGateInput {
                consecutive_failures: MAX_CONSECUTIVE_GOAL_ROUND_FAILURES - 1,
                ..passable()
            }),
            GoalRoundGateDecision::Continue,
            "未达阈值不得提前停（一次限流不该终止目标）"
        );
    }

    #[test]
    fn round_budget_beats_auto_continue_budget() {
        assert_eq!(
            goal_round_gate(GoalRoundGateInput {
                rounds_started: 10,
                max_rounds: 10,
                consecutive_auto_continues: 8,
                ..passable()
            }),
            GoalRoundGateDecision::Stop(GoalRoundStopReason::BudgetExhausted {
                rounds_started: 10,
                max_rounds: 10,
            }),
            "目标预算耗尽优先于每线程预算（不得把「轮次用尽」说成「连续自动推进达上限」）"
        );
    }

    #[test]
    fn auto_continue_budget_stops_last() {
        assert_eq!(
            goal_round_gate(GoalRoundGateInput {
                consecutive_auto_continues: 8,
                ..passable()
            }),
            GoalRoundGateDecision::Stop(GoalRoundStopReason::AutoContinueBudget)
        );
    }

    #[test]
    fn outcome_gate_truth_table() {
        assert_eq!(
            goal_round_outcome_gate(GoalRoundStepOutcome::Completed),
            GoalRoundGateDecision::Continue,
            "只有成功轮才推进续轮"
        );
        assert_eq!(
            goal_round_outcome_gate(GoalRoundStepOutcome::Failed),
            GoalRoundGateDecision::Stop(GoalRoundStopReason::FirstRoundNotSucceeded),
            "失败轮必须报「本轮未成功」，不得复用 ConsecutiveFailures（那读作已经连跑多轮）"
        );
        assert_eq!(
            goal_round_outcome_gate(GoalRoundStepOutcome::Suspended),
            GoalRoundGateDecision::Pause,
            "挂起是暂停不是终止（作答后必须重入续轮）"
        );
        assert_eq!(
            goal_round_outcome_gate(GoalRoundStepOutcome::Cancelled),
            GoalRoundGateDecision::Stop(GoalRoundStopReason::Cancelled)
        );
        assert_eq!(
            goal_round_outcome_gate(GoalRoundStepOutcome::Unclassified),
            GoalRoundGateDecision::Stop(GoalRoundStopReason::TurnUnclassified),
            "不知道成没成 ⇒ 既不冒充成功也不冒充失败"
        );
    }

    /// `Pause` 的可见原因是 `TurnSuspended`，文案必须表达「等你作答 + 作答后自动继续」。
    #[test]
    fn pause_reason_is_turn_suspended_with_resume_copy() {
        assert_eq!(
            GoalRoundGateDecision::Pause.stop_reason(),
            Some(GoalRoundStopReason::TurnSuspended),
            "Pause 复用既有 TurnSuspended（禁止新增变体）"
        );
        let line = GoalRoundStopReason::TurnSuspended.visible_line();
        assert!(line.contains("等你作答"), "Pause 文案必须点名「等你作答」：{line}");
        assert!(
            line.contains("作答后自动继续"),
            "Pause 文案必须承诺「作答后自动继续」（否则用户以为目标已死）：{line}"
        );
        assert!(
            !line.contains("续轮已停止"),
            "Pause 不得复用「续轮已停止」措辞：{line}"
        );
    }

    /// 兼容读面 `goal_round_stop_reason` 必须与 gate 的三值投影逐行一致（防两处判据漂移）。
    #[test]
    fn legacy_stop_reason_is_gate_projection() {
        assert_eq!(goal_round_stop_reason(false, 0, 0, 10), None);
        assert_eq!(goal_round_stop_reason(false, 1, 3, 10), None);
        assert_eq!(
            goal_round_stop_reason(true, 0, 0, 10),
            Some(GoalRoundStopReason::Cancelled)
        );
        assert_eq!(
            goal_round_stop_reason(false, MAX_CONSECUTIVE_GOAL_ROUND_FAILURES, 4, 10),
            Some(GoalRoundStopReason::ConsecutiveFailures {
                failures: MAX_CONSECUTIVE_GOAL_ROUND_FAILURES,
            })
        );
        assert_eq!(
            goal_round_stop_reason(false, 0, 10, 10),
            Some(GoalRoundStopReason::BudgetExhausted {
                rounds_started: 10,
                max_rounds: 10,
            })
        );
    }
}
