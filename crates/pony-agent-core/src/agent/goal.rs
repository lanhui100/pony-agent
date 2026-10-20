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
    crate::agent::sqlite_session::default_sqlite_path()
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
        Ok(goals) => {
            GOAL_STORE_DEGRADED.store(false, std::sync::atomic::Ordering::SeqCst);
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
fn persist_goal_store(goals: &HashMap<String, GoalData>) -> Result<(), String> {
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
    let to_write = if path.exists() {
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

/// 契约 C3：goal-round 用户消息的唯一文案实现（宿主循环与 TUI 必须共用）。
pub fn build_goal_round_message(objective: &str, round: u32, max_rounds: u32) -> String {
    format!(
        "<goal_round round=\"{round}\" max=\"{max_rounds}\">继续推进目标：{objective}。以工作区与工具结果为准；仅当目标真正达成才 update_goal complete。若本轮仍有未完成步骤，直接继续执行，不要询问用户。</goal_round>"
    )
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

    if let Err(error) = persist_goal_store(goals) {
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

    Ok(updated)
}

fn resolve_session(session_id: Option<String>) -> String {
    session_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".to_string())
}

pub fn create_goal(args: CreateGoalArgs) -> Result<CreateGoalResult, String> {
    let session = resolve_session(args.session_id);
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

    let mut guard = goal_store_guard();
    ensure_goal_store_loaded(&mut guard);
    let goals = guard.as_mut().expect("goal store must be loaded");
    let previous = goals.get(&session).cloned();
    goals.insert(session.clone(), goal.clone());

    if let Err(error) = persist_goal_store(goals) {
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

    Ok(CreateGoalResult { goal })
}

pub fn get_goal(args: GetGoalArgs) -> Result<GetGoalResult, String> {
    let session = resolve_session(args.session_id);
    let mut guard = goal_store_guard();
    ensure_goal_store_loaded(&mut guard);
    let goals = guard.as_ref().expect("goal store must be loaded");
    Ok(GetGoalResult {
        goal: goals.get(&session).cloned(),
    })
}

pub fn update_goal(args: UpdateGoalArgs) -> Result<UpdateGoalResult, String> {
    let session = resolve_session(args.session_id);
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
                current.phase = "paused".to_string();
                current.activation = GoalActivation::Disarmed;
            }
            UpdateGoalAction::Resume => {
                current.phase = "active".to_string();
                current.activation = GoalActivation::Armed;
                current.blocked_reason = None;
            }
            UpdateGoalAction::Complete => {
                current.phase = "completed".to_string();
                current.activation = GoalActivation::Disarmed;
                current.blocked_reason = None;
            }
            UpdateGoalAction::Blocked => {
                let reason = args
                    .blocked_reason
                    .ok_or_else(|| "Blocked action requires blocked_reason".to_string())?;
                current.phase = "blocked".to_string();
                current.activation = GoalActivation::Disarmed;
                current.blocked_reason = Some(reason);
            }
            UpdateGoalAction::Edit => {
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

    if let Err(error) = persist_goal_store(goals) {
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

    Ok(UpdateGoalResult { goal: updated })
}

/// 进程级 goal store 的测试串行化接缝（lib 单测跨模块共享同一 store，必须互斥）。
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Mutex;

    static GOAL_STORE_TEST_LOCK: Mutex<()> = Mutex::new(());

    pub(crate) fn lock() -> std::sync::MutexGuard<'static, ()> {
        GOAL_STORE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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

        // Blocked requires reason
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
}

