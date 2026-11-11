//! 技能根发现口径契约（PA-173 / ISSUE-20261111-007）。
//!
//! 断言四条口径，缺一条都会回到「技能清单看不见本该可用的技能」：
//! 1. **仓库根上探**：cwd 是仓库子目录时，`.git` 上探到的仓库根三层技能目录同样进入清单
//!    （rank 350/360/370，整体劣于会话根 100/200/300 ⇒ 近者覆盖远者）；
//! 2. **自定义根**：`PONY_AGENT_SKILL_DIRS` 指向的技能根以 `Global` 作用域进入清单（rank 450+），
//!    项目同名技能仍覆盖它（项目带恒优于全局带）；
//! 3. **上探有界**：命中不到 `.git` 时退回会话根单层，绝不把父目录 / `~` / `/` 当项目根；
//! 4. **可见性 = 可加载性**：清单里出现过的仓库根技能必须能被加载路径真读到
//!    （加载侧信任根与扫描侧共用同一枚举，消除「列得出、载不进」）。
//!
//! 隔离要点：全部用例只在临时目录内造 `.git` / 技能文件；不联网、不依赖宿主 `~` 现状
//! （只按技能名断言，不断言技能总数）。

use pony_agent_core::agent::skill_body::{
    is_trusted_skill_file, load_skill_body_for_workspace, MAX_SKILL_BODY_BYTES,
};
use pony_agent_core::agent::workspace_hotloader::{
    find_git_root, AgentScope, HotloadedSkill, WorkspaceHotloader, PONY_AGENT_SKILL_DIRS_ENV,
};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// 唯一临时目录，Drop 时清理（panic 路径也不残留）。
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
        let dir = std::env::temp_dir().join(format!(
            "pony-agent-skillroot-{tag}-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("创建技能根契约测试临时目录失败");
        Self { root: dir }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn mkdir(&self, relative: &str) -> PathBuf {
        let dir = self.root.join(relative);
        fs::create_dir_all(&dir).expect("创建临时子目录失败");
        dir
    }

    fn write(&self, relative: &str, content: &str) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("创建临时子目录失败");
        }
        fs::write(&path, content).expect("写入临时文件失败");
        path
    }

    /// 以目录形态伪造仓库标记（`<repo>/.git`）。
    fn mark_git_dir(&self) {
        fs::create_dir_all(self.root.join(".git")).expect("创建 .git 目录失败");
    }

    /// 以文件形态伪造仓库标记（worktree / submodule 的 `<repo>/.git` 是文件）。
    fn mark_git_file(&self) {
        self.write(".git", "gitdir: /nonexistent/worktrees/fake\n");
    }

    fn write_skill(&self, relative_root: &str, folder: &str, name: &str, description: &str) {
        // `relative_root` 为空表示「根目录下直接放技能目录」（自定义技能根的形态）。
        let relative = if relative_root.is_empty() {
            format!("{folder}/SKILL.md")
        } else {
            format!("{relative_root}/{folder}/SKILL.md")
        };
        self.write(
            &relative,
            &format!("---\nname: {name}\ndescription: {description}\n---\n\n# Body of {name}\n"),
        );
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// 环境变量守卫：Drop（含 panic 展开）时恢复原值。
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

fn by_name<'a>(skills: &'a [HotloadedSkill], name: &str) -> Vec<&'a HotloadedSkill> {
    skills.iter().filter(|skill| skill.name == name).collect()
}

// ============================================================
// 1. 仓库根上探：子目录 cwd 也能看见仓库根技能
// ============================================================

#[test]
fn repo_root_skills_are_discovered_from_subdirectory_cwd() {
    let repo = TempDir::new("subdir-repo");
    repo.mark_git_dir();
    repo.write_skill(
        ".agents/skills",
        "pa-issue-triage",
        "pa-issue-triage",
        "from repo root",
    );
    let cwd = repo.mkdir("crates/pony-agent-core");

    let (skills, _hit) = WorkspaceHotloader::new().scan_skills_with_roots(&cwd, &[], &[]);

    let found = by_name(&skills, "pa-issue-triage");
    assert_eq!(
        found.len(),
        1,
        "子目录 cwd 必须经 `.git` 上探发现仓库根技能，实际清单：{:?}",
        skills.iter().map(|skill| &skill.name).collect::<Vec<_>>()
    );
    assert_eq!(found[0].scope, AgentScope::Project, "仓库根技能属项目域");
    assert_eq!(
        found[0].rank, 360,
        "仓库根 `.agents/skills` 的冻结 rank 是 360，实际 {}",
        found[0].rank
    );
}

#[test]
fn git_file_marker_is_honoured_for_worktrees() {
    let repo = TempDir::new("worktree-repo");
    repo.mark_git_file();
    repo.write_skill(".dsh/skills", "wt-skill", "wt-skill", "worktree repo root");
    let cwd = repo.mkdir("packages/app");

    let (skills, _hit) = WorkspaceHotloader::new().scan_skills_with_roots(&cwd, &[], &[]);

    let found = by_name(&skills, "wt-skill");
    assert_eq!(
        found.len(),
        1,
        "`.git` 为文件形态（worktree）时同样必须上探"
    );
    assert_eq!(
        found[0].rank, 350,
        "仓库根 `.dsh/skills` 的冻结 rank 是 350"
    );
}

#[test]
fn workspace_skill_overrides_repo_skill_with_same_name() {
    let repo = TempDir::new("override-repo");
    repo.mark_git_dir();
    repo.write_skill(".agents/skills", "shared", "shared", "from repo root");
    let cwd = repo.mkdir("app");
    repo.write_skill(
        "app/.agents/skills",
        "shared",
        "shared",
        "from session root",
    );

    let (skills, _hit) = WorkspaceHotloader::new().scan_skills_with_roots(&cwd, &[], &[]);

    let found = by_name(&skills, "shared");
    assert_eq!(found.len(), 1, "同名技能必须去重为一条");
    assert_eq!(
        found[0].description, "from session root",
        "会话根（rank 200）必须覆盖仓库根（rank 360），实际胜出：{:?}（rank={}）",
        found[0].description, found[0].rank
    );
}

// ============================================================
// 2. 上探有界：没有 .git 时不越界收罗父目录
// ============================================================

#[test]
fn ancestor_without_git_marker_is_not_a_project_root() {
    let parent = TempDir::new("no-git-parent");
    parent.write_skill(
        ".agents/skills",
        "parent-only",
        "parent-only",
        "parent dir skill",
    );
    let cwd = parent.mkdir("nested/work");

    let (skills, _hit) = WorkspaceHotloader::new().scan_skills_with_roots(&cwd, &[], &[]);

    assert!(
        by_name(&skills, "parent-only").is_empty(),
        "无 `.git` 标记时不得把父目录当项目根（会越界注入上一层技能）"
    );
    assert_eq!(
        find_git_root(&cwd),
        None,
        "临时目录链上没有 `.git` 时必须返回 None"
    );
}

// ============================================================
// 3. 自定义技能根（PONY_AGENT_SKILL_DIRS）
// ============================================================

#[test]
fn custom_skill_root_is_discovered_as_global_scope() {
    let cwd = TempDir::new("custom-cwd");
    let custom = TempDir::new("custom-root");
    custom.write_skill("", "pa-issue-triage", "pa-issue-triage", "from custom root");

    let (skills, _hit) = WorkspaceHotloader::new().scan_skills_with_roots(
        cwd.path(),
        &[],
        &[custom.path().to_path_buf()],
    );

    let found = by_name(&skills, "pa-issue-triage");
    assert_eq!(
        found.len(),
        1,
        "自定义技能根（其下直接放技能目录）必须被发现，实际清单：{:?}",
        skills.iter().map(|skill| &skill.name).collect::<Vec<_>>()
    );
    assert_eq!(
        found[0].scope,
        AgentScope::Global,
        "自定义根属用户自有域（跨项目共享）"
    );
    assert_eq!(found[0].rank, 450, "自定义根 rank 从 450 起算");
}

#[test]
fn project_skill_overrides_custom_root_skill_with_same_name() {
    let cwd = TempDir::new("custom-override-cwd");
    let custom = TempDir::new("custom-override-root");
    custom.write_skill("", "pa-issue-triage", "pa-issue-triage", "from custom root");
    cwd.write_skill(
        ".agents/skills",
        "pa-issue-triage",
        "pa-issue-triage",
        "from project",
    );

    let (skills, _hit) = WorkspaceHotloader::new().scan_skills_with_roots(
        cwd.path(),
        &[],
        &[custom.path().to_path_buf()],
    );

    let found = by_name(&skills, "pa-issue-triage");
    assert_eq!(found.len(), 1, "同名技能必须去重为一条");
    assert_eq!(
        found[0].description, "from project",
        "项目层（全局带之上）必须覆盖自定义根，实际胜出：{:?}（rank={}）",
        found[0].description, found[0].rank
    );
}

// ============================================================
// 4. 可见性 = 可加载性（扫描与加载共用同一枚举）
// ============================================================

#[test]
fn repo_root_skill_listed_in_catalog_is_loadable() {
    let repo = TempDir::new("loadable-repo");
    repo.mark_git_dir();
    repo.write_skill(
        ".agents/skills",
        "loadable-skill",
        "loadable-skill",
        "repo root skill",
    );
    let cwd = repo.mkdir("deep/nested");

    let (skills, _hit) = WorkspaceHotloader::new().scan_skills_with_roots(&cwd, &[], &[]);
    let found = by_name(&skills, "loadable-skill");
    assert_eq!(found.len(), 1, "前置条件：仓库根技能必须先在清单里");

    let body = load_skill_body_for_workspace(found[0], MAX_SKILL_BODY_BYTES, Some(cwd.as_path()))
        .expect("仓库根技能「列得出」就必须「载得进」（P1-6）");
    assert!(
        body.content.contains("Body of loadable-skill"),
        "加载到的正文不符：{}",
        body.content
    );
}

#[test]
fn custom_root_skill_loads_through_env_configured_trust_root() {
    let _lock = env_lock();
    let cwd = TempDir::new("custom-load-cwd");
    let custom = TempDir::new("custom-load-root");
    custom.write_skill("", "env-skill", "env-skill", "custom root skill");
    let _env = EnvGuard::set(PONY_AGENT_SKILL_DIRS_ENV, custom.path());

    let (skills, _hit) = WorkspaceHotloader::new().scan_skills_with_roots(
        cwd.path(),
        &[],
        &[custom.path().to_path_buf()],
    );
    let found = by_name(&skills, "env-skill");
    assert_eq!(found.len(), 1, "前置条件：自定义根技能必须先在清单里");

    let body = load_skill_body_for_workspace(found[0], MAX_SKILL_BODY_BYTES, Some(cwd.path()))
        .expect("`PONY_AGENT_SKILL_DIRS` 配置的技能必须可加载");
    assert!(body.content.contains("Body of env-skill"));
    assert!(
        is_trusted_skill_file(&found[0].path, custom.path(), true),
        "自定义根按用户自有域判定（允许符号链接逃逸）"
    );
}

// ============================================================
// 5. 负样本：项目根不得经符号链接逃出所属项目域
// ============================================================

#[cfg(unix)]
#[test]
fn repo_root_skill_symlink_escape_is_rejected() {
    use std::os::unix::fs::symlink;

    let repo = TempDir::new("escape-repo");
    repo.mark_git_dir();
    let outside = TempDir::new("escape-outside");
    outside.write_skill(
        "",
        "outside-skill",
        "outside-skill",
        "outside the project domain",
    );
    repo.mkdir(".agents");
    symlink(outside.path(), repo.path().join(".agents/skills")).expect("创建逃逸符号链接失败");
    let cwd = repo.mkdir("sub");

    let (skills, _hit) = WorkspaceHotloader::new().scan_skills_with_roots(&cwd, &[], &[]);

    assert!(
        by_name(&skills, "outside-skill").is_empty(),
        "项目根符号链接逃出所属项目域时必须整根不采纳（workspace 内容是不可信输入）"
    );
}
