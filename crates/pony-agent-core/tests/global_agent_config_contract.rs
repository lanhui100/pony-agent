//! 用户级（全局）指令与 Skills 分层加载的公开入口契约（PA-148 / ADR 0045）。
//!
//! 断言的是**默认入口**（`resolve_constitution` / `scan_skills`，不注入 `global_homes`）：
//! 通过 `$PONY_AGENT_HOME` 指向唯一临时目录，证明用户级规则与技能会被真实装配，
//! 且同名技能由项目级覆盖全局级。
//!
//! 隔离要点：
//! - 环境变量在本测试二进制内唯一修改，`EnvGuard` 在 Drop（含 panic）时恢复原值；
//! - 宿主机可能另有 `~/.agents` / `~/.claude` / 产品配置根，故只断言「存在性与覆盖语义」，
//!   不断言技能总数。

use pony_agent_core::agent::workspace_hotloader::{AgentScope, WorkspaceHotloader};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const ENV_KEY: &str = "PONY_AGENT_HOME";

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
            "pony-agent-{tag}-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("创建全局配置契约测试临时目录失败");
        Self { root: dir }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn write(&self, relative: &str, content: &str) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("创建临时子目录失败");
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

/// 环境变量守卫：Drop 时恢复修改前的取值。
struct EnvGuard {
    previous: Option<OsString>,
}

impl EnvGuard {
    fn set(key: &str, value: &Path) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(ENV_KEY, value),
            None => std::env::remove_var(ENV_KEY),
        }
    }
}

fn skill_markdown(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n\n# Body\n")
}

#[test]
fn global_home_env_feeds_default_entry_points_and_project_overrides_skill() {
    let global_home = TempDir::new("global-home");
    let workspace = TempDir::new("global-workspace");

    global_home.write(
        "AGENTS.md",
        "# User Level Rules\nAlways answer in Chinese and keep output terse.",
    );
    global_home.write(
        "skills/auditor/SKILL.md",
        &skill_markdown("auditor", "Global audit skill"),
    );
    let _guard = EnvGuard::set(ENV_KEY, global_home.path());

    let hotloader = WorkspaceHotloader::new();

    // 1）指令：工作区没有 AGENTS.md，只有用户级规则也必须进入上下文
    let (constitution, _) = hotloader
        .resolve_constitution(workspace.path())
        .expect("用户级 AGENTS.md 必须被默认入口加载");
    assert!(
        constitution.contains("# User Level Rules"),
        "用户级规则正文缺失，实际内容：{constitution}"
    );
    assert!(
        constitution.contains("Global user-level instructions"),
        "多层渲染必须标注全局层来源，实际内容：{constitution}"
    );

    // 2）技能：用户级技能以 global 作用域出现在默认扫描结果中
    let (skills, _) = hotloader.scan_skills(workspace.path());
    let global_auditor = skills
        .iter()
        .find(|skill| skill.name == "auditor")
        .unwrap_or_else(|| {
            panic!(
                "用户级技能 auditor 必须被默认扫描发现，实际：{:?}",
                skills.iter().map(|skill| &skill.name).collect::<Vec<_>>()
            )
        });
    assert_eq!(global_auditor.scope, AgentScope::Global);
    assert_eq!(global_auditor.description, "Global audit skill");

    // 3）同名覆盖：工作区出现同名技能后，项目级胜出且只剩一条
    workspace.write(
        ".agents/skills/auditor/SKILL.md",
        &skill_markdown("auditor", "Project audit skill"),
    );
    let (merged, _) = hotloader.scan_skills(workspace.path());
    let same_name: Vec<_> = merged
        .iter()
        .filter(|skill| skill.name == "auditor")
        .collect();
    assert_eq!(same_name.len(), 1, "同名技能必须去重为一条");
    assert_eq!(same_name[0].description, "Project audit skill");
    assert_eq!(same_name[0].scope, AgentScope::Project);
}
