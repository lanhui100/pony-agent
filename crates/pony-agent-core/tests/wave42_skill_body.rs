//! Wave-42 / PA-149 —— 技能正文按需加载（契约矩阵 C1）红相验收。
//!
//! 契约依据：`.dev-team/contract-matrix-wave42-global-config.md` C1 表 + task-1 描述。
//!
//! 生命周期（L3：涉及安全白名单 fail-closed 边界）：
//! - 每个用例使用 `std::env::temp_dir()` + PID/纳秒/序号唯一名，`TempDir` 在 Drop（含 panic）
//!   时清理，不污染宿主；
//! - 涉及 `$PONY_AGENT_HOME` 的用例在同一二进制内串行（`ENV_LOCK`），并保证同进程**仅一处**
//!   设置该变量，`EnvGuard` 在 Drop 时恢复原值；
//! - 不联网、不读宿主 `~` 现状作为断言依据（只通过注入目录证明语义）。
//!
//! 公开 API 说明：`load_skill_body` / `resolve_skill_by_name` / `SkillBodyError` 已在
//! `agent::skill_body` 落桩，本文件只使用这些公开符号，**不引用**尚未落地的
//! `path_permission::trusted_readonly_roots` / `is_within_trusted_readonly_roots`
//! （避免编译期假红）。工具 `skill_load` 尚未注册，本文件通过既有公开工具面投影入口
//! `builtin_turn_tool_contract_views()` / `builtin_tools()` 断言其间接可见性 ——
//! 这是与 `tools.rs` 内既有 28 项契约测试**同源**的入口。

use pony_agent_core::agent::skill_body::{
    load_skill_body, resolve_skill_by_name, SkillBodyError, MAX_SKILL_BODY_BYTES,
};
use pony_agent_core::agent::tools::{builtin_tools, builtin_turn_tool_contract_views};
use pony_agent_core::agent::workspace_hotloader::{
    AgentScope, FileFingerprint, HotloadedSkill, PONY_AGENT_HOME_ENV,
};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

/// `$PONY_AGENT_HOME` 是进程级全局状态：本二进制内所有涉事用例必须串行。
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// 唯一临时目录：Drop（含 panic 展开路径）时递归清理。
struct TempDir {
    root: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let sequence = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!(
            "wave42-c1-{tag}-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("创建 C1 测试临时目录失败");
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn mkdir(&self, relative: &str) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(&path).expect("创建临时子目录失败");
        path
    }

    fn write(&self, relative: &str, content: &str) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("创建临时文件父目录失败");
        }
        fs::write(&path, content).expect("写入临时文件失败");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// 环境变量守卫：Drop 时恢复修改前的取值（panic 路径同样生效）。
struct EnvGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &Path) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

/// 伪造一条扫描记录（用于构造越界路径，模拟被篡改/被污染的 hotloader 结果）。
fn forged_skill(path: &Path) -> HotloadedSkill {
    HotloadedSkill {
        name: "forged".to_string(),
        description: "forged descriptor".to_string(),
        user_invocable: true,
        allowed_tools: Vec::new(),
        path: path.to_path_buf(),
        fingerprint: FileFingerprint {
            path: path.to_path_buf(),
            mtime: None,
            file_size: 0,
            content_hash: 0,
        },
        scope: AgentScope::Global,
        rank: 50,
    }
}

fn expect_error<T>(result: Result<T, SkillBodyError>, what: &str) -> SkillBodyError {
    match result {
        Ok(_) => panic!("{what}：期望 fail-closed 报错，实际返回了成功值"),
        Err(error) => error,
    }
}

// ============================================================
// C1-1 白名单内技能可加载正文
// ============================================================

#[test]
fn c1_loads_skill_body_from_whitelisted_global_home() {
    // Arrange：唯一临时全局根（`$PONY_AGENT_HOME`）+ 空工作区
    let _lock = env_lock();
    let home = TempDir::new("home-ok");
    let workspace = TempDir::new("ws-ok");
    let body_marker = "# ALPHA BODY MARKER\nRun the alpha workflow end to end.\n";
    let skill_file = home.write(
        "skills/alpha/SKILL.md",
        &format!("---\nname: alpha\ndescription: alpha skill\n---\n\n{body_marker}"),
    );
    let _env = EnvGuard::set(PONY_AGENT_HOME_ENV, home.path());

    // Act：按名解析（含全局层）→ 读正文
    let resolved = resolve_skill_by_name(workspace.path(), "alpha")
        .expect("C1-1：白名单内的全局技能 `alpha` 必须可被 resolve_skill_by_name 解析");
    assert_eq!(resolved.name, "alpha", "C1-1：解析结果名必须与请求一致");
    assert_eq!(
        resolved.scope,
        AgentScope::Global,
        "C1-1：$PONY_AGENT_HOME 下的技能作用域必须是 Global"
    );
    assert_eq!(resolved.path, skill_file, "C1-1：解析路径必须是 SKILL.md 本身");

    let body = match load_skill_body(&resolved, MAX_SKILL_BODY_BYTES) {
        Ok(body) => body,
        Err(error) => panic!("C1-1：白名单内技能正文必须加载成功，实际 {error:?}"),
    };

    // Assert：正文、作用域、路径、未截断标记
    assert!(
        body.content.contains(body_marker.trim_end()),
        "C1-1：正文必须包含 SKILL.md 真实内容，实际：{:?}",
        body.content
    );
    assert_eq!(body.name, "alpha", "C1-1：正文记录名必须与请求一致");
    assert_eq!(
        body.scope,
        AgentScope::Global,
        "C1-1：正文作用域必须与扫描结果一致"
    );
    assert_eq!(body.path, skill_file, "C1-1：正文路径必须指向真实文件");
    assert!(
        !body.truncated,
        "C1-1：未超上限的正文不得被标记截断（禁止静默截断返回成功）"
    );
}

// ============================================================
// C1-2 越界路径 fail-closed（禁止静默返回空正文）
// ============================================================

#[test]
fn c1_rejects_paths_outside_trusted_readonly_roots() {
    let _lock = env_lock();
    let home = TempDir::new("home-deny");
    home.mkdir("skills");
    let _env = EnvGuard::set(PONY_AGENT_HOME_ENV, home.path());
    let outside = TempDir::new("outside-deny");
    let outside_file = outside.write("secret.md", "# TOP SECRET\nnever inject this\n");

    // 反例 1：系统敏感文件
    let error = expect_error(
        load_skill_body(&forged_skill(Path::new("/etc/passwd")), MAX_SKILL_BODY_BYTES),
        "C1-2：/etc/passwd 必须被拒绝",
    );
    match error {
        SkillBodyError::OutOfAllowedRoot(reported) => {
            assert!(
                !reported.starts_with(home.path()),
                "C1-2：越界错误里的路径不应落在受信根内，实际 {reported:?}"
            );
        }
        other => panic!("C1-2：/etc/passwd 必须返回 OutOfAllowedRoot，实际 {other:?}"),
    }

    // 反例 2：受信根之外的普通文件（存在且可读，但不在白名单）
    let error = expect_error(
        load_skill_body(&forged_skill(&outside_file), MAX_SKILL_BODY_BYTES),
        "C1-2：白名单外的可读文件必须被拒绝",
    );
    match error {
        SkillBodyError::OutOfAllowedRoot(reported) => {
            assert!(
                reported == outside_file || reported.ends_with("secret.md"),
                "C1-2：越界错误必须携带被拒绝的路径，实际 {reported:?}"
            );
            assert!(
                !reported.starts_with(home.path()),
                "C1-2：错误路径不得被错误地标记为受信根内"
            );
        }
        other => panic!(
            "C1-2：白名单外文件必须返回 OutOfAllowedRoot（不得静默返回空正文），实际 {other:?}"
        ),
    }
}

// ============================================================
// C1-3 超限拒绝（禁止截断返回成功）
// ============================================================

#[test]
fn c1_rejects_oversize_skill_body_with_size_and_limit() {
    let _lock = env_lock();
    let home = TempDir::new("home-large");
    let workspace = TempDir::new("ws-large");
    let payload = "L".repeat(4_096);
    let content = format!("---\nname: bulky\ndescription: bulky skill\n---\n\n{payload}\n");
    let skill_file = home.write("skills/bulky/SKILL.md", &content);
    let _env = EnvGuard::set(PONY_AGENT_HOME_ENV, home.path());

    let skill = resolve_skill_by_name(workspace.path(), "bulky")
        .expect("C1-3：前置条件：全局技能 `bulky` 必须可解析");
    assert_eq!(skill.path, skill_file, "C1-3：前置条件：解析路径必须正确");

    let limit = 512usize;
    let error = expect_error(
        load_skill_body(&skill, limit),
        "C1-3：超过 max_bytes 的正文必须被拒绝",
    );
    match error {
        SkillBodyError::TooLarge {
            size,
            limit: reported_limit,
        } => {
            assert_eq!(
                reported_limit, limit,
                "C1-3：错误必须携带调用方给定的 limit"
            );
            assert_eq!(
                size,
                content.as_bytes().len(),
                "C1-3：错误必须携带真实文件字节数 size"
            );
            assert!(
                size > reported_limit,
                "C1-3：size({size}) 必须大于 limit({reported_limit})"
            );
        }
        other => panic!("C1-3：超限必须返回 TooLarge{{size,limit}}，实际 {other:?}"),
    }
}

// ============================================================
// C1-4 未知名解析失败（且 Known 名可解析，避免空实现"永远 None"式假绿）
// ============================================================

#[test]
fn c1_unknown_skill_name_resolves_to_none() {
    let _lock = env_lock();
    let home = TempDir::new("home-unknown");
    home.mkdir("skills");
    let workspace = TempDir::new("ws-unknown");
    let _env = EnvGuard::set(PONY_AGENT_HOME_ENV, home.path());

    assert!(
        resolve_skill_by_name(workspace.path(), "wave42-definitely-absent-skill").is_none(),
        "C1-4：未知名必须返回 None（禁止返回空字符串式'成功'）"
    );

    // 反空实现证据：同一入口对已安装技能必须返回 Some。
    home.write(
        "skills/present/SKILL.md",
        "---\nname: present\ndescription: present skill\n---\n\nbody\n",
    );
    let present = resolve_skill_by_name(workspace.path(), "present");
    assert!(
        present.is_some(),
        "C1-4：已安装技能必须可解析，否则 None 断言无法区分'未实现'与'未命中'"
    );
}

// ============================================================
// C1-5 工具注册：模型可见面 28 → 29，`skill_load` 在 schema 列表内
// ============================================================

#[test]
fn c1_skill_load_is_registered_on_the_model_visible_surface() {
    // 与 `tools.rs` 内既有 28 项契约测试同源入口。
    let views = builtin_turn_tool_contract_views();
    let names: Vec<&str> = views.iter().map(|view| view.name.as_str()).collect();
    assert_eq!(
        names.len(),
        29,
        "C1-5：新增 `skill_load` 后模型可见面必须是 29 项（基线 28），实际：{names:?}"
    );
    let view = views
        .iter()
        .find(|view| view.name == "skill_load")
        .unwrap_or_else(|| panic!("C1-5：模型可见工具面缺少 `skill_load`，实际：{names:?}"));
    let properties = view
        .input_schema
        .get("properties")
        .and_then(|value| value.as_object())
        .unwrap_or_else(|| panic!("C1-5：`skill_load` 必须有对象 properties，实际：{}", view.input_schema));
    assert!(
        properties.contains_key("name"),
        "C1-5：`skill_load` 入参契约是 {{\"name\": string}}，实际属性：{:?}",
        properties.keys().collect::<Vec<_>>()
    );

    // 内部注册表也必须包含（仅仅投影一个空壳 name 不算注册）。
    let internal = builtin_tools();
    assert_eq!(
        internal.len(),
        45,
        "C1-5：内部注册表必须为 45 项（基线 44 + skill_load）"
    );
    assert!(
        internal.iter().any(|tool| tool.name == "skill_load"),
        "C1-5：内部注册表缺少 `skill_load`（只改投影不改注册是假注册）"
    );
}
