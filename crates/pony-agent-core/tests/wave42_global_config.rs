//! Wave-42 / PA-150/151/152 —— 指令字节预算、层级发现、触达即载（契约矩阵 C2/C3/C4）红相验收。
//!
//! 契约依据：`.dev-team/contract-matrix-wave42-global-config.md` C2/C3/C4 三张表 + task-2/3/4 描述。
//!
//! 生命周期与确定性（L3：跨模块提示词前缀契约 + 缓存基线身份）：
//! - 临时目录一律 `std::env::temp_dir()` + PID/纳秒/序号唯一名，`TempDir` 在 Drop（含 panic）时
//!   递归清理；
//! - 全局层一律走**可注入 API**（`global_homes: &[PathBuf]`），只有 C3-5 的技能 rank 用例需要
//!   `$PONY_AGENT_HOME` 的真实身份，该用例在 `ENV_LOCK` 内串行并 `EnvGuard` 恢复原值；
//! - 不联网、不 sleep 等外部时序、不依赖宿主 `~` 现状。
//!
//! TODO(executor / Lead 复核) 两处契约缝在源码中尚无公开符号，为避免编译期"假红"未直接引用：
//! 1. C2-4「会话第 2 轮**改预算**」需要有预算入参的会话前缀入口（当前
//!    `resolve_session_prompt_context_with_globals` 无 `InstructionBudget` 参数，`InstructionBudget`
//!    也尚未被任何调用点消费）。本文件退一步断言同一契约的可观测面：**会话静态前缀必须受预算约束
//!    （超限层不得进入固化前缀），且跨轮次字节不变 + 返回 notice**。待执行者补齐预算入参后，
//!    C2-4 应追加「预算变更 → 前缀不变 + notice」的直测分支。
//! 2. C4-2 的**工具级**（同一 session）去重需要 `ToolExecutionContext` 上的会话维度集合或显式传参，
//!    该字段尚不存在；本文件在纯函数层（`already_injected`）验证去重语义。

use pony_agent_core::agent::instruction_touch::{
    instructions_for_touched_path, MAX_TOUCH_STEP_BYTES,
};
use pony_agent_core::agent::tools::{ToolCall, ToolRouter};
use pony_agent_core::agent::workspace_hotloader::{
    ancestor_chain, find_project_root, AgentScope, InstructionBudget, WorkspaceHotloader,
    PONY_AGENT_HOME_ENV,
};
use serde_json::json;
use std::collections::BTreeSet;
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
            "wave42-c2c4-{tag}-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("创建 C2/C3/C4 测试临时目录失败");
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

fn budgeted_constitution(
    hotloader: &WorkspaceHotloader,
    workspace: &Path,
    homes: &[PathBuf],
    budget: InstructionBudget,
) -> String {
    match hotloader.resolve_constitution_with_budget(workspace, homes, budget) {
        Some((text, _)) => text,
        None => panic!(
            "C2：`resolve_constitution_with_budget` 对存在指令文件的层级链返回了 None（禁止静默丢弃整条链）"
        ),
    }
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.match_indices(needle).count()
}

// ============================================================
// C2-1 默认预算零漂移：与 resolve_constitution_with_globals 字节完全相等
// ============================================================

#[test]
fn c2_default_budget_matches_legacy_constitution_bytes() {
    // Arrange：项目层 + 两个全局层
    let workspace = TempDir::new("parity-ws");
    let global_one = TempDir::new("parity-g1");
    let global_two = TempDir::new("parity-g2");
    global_one.write("AGENTS.md", "# GLOBAL ONE\nbe terse\n");
    global_two.write("AGENTS.md", "# GLOBAL TWO\nbe exact\n");
    workspace.write("AGENTS.md", "# PROJECT RULES\nuse rust\n");
    let homes = vec![
        global_one.path().to_path_buf(),
        global_two.path().to_path_buf(),
    ];

    // Act
    let legacy = WorkspaceHotloader::new();
    let (legacy_text, _) = legacy
        .resolve_constitution_with_globals(workspace.path(), &homes)
        .expect("C2-1：前置条件：既有全局解析入口必须返回内容");
    let budget_resolver = WorkspaceHotloader::new();
    let budget_text = budgeted_constitution(
        &budget_resolver,
        workspace.path(),
        &homes,
        InstructionBudget::default(),
    );

    // Assert：字节级零漂移（红线 4）
    assert_eq!(
        legacy_text.as_bytes(),
        budget_text.as_bytes(),
        "C2-1：默认预算输出必须与 c1bd454 的 resolve_constitution_with_globals 字节完全一致"
    );

    // 单项目层回归：默认预算下仍返回原始字节，不得多出空行/标题
    let single = TempDir::new("parity-single");
    single.write("AGENTS.md", "# ONLY PROJECT\nno heading added\n");
    let single_text = budgeted_constitution(
        &WorkspaceHotloader::new(),
        single.path(),
        &[],
        InstructionBudget::default(),
    );
    assert_eq!(
        single_text, "# ONLY PROJECT\nno heading added\n",
        "C2-1：单一项目层必须返回原始字节（多出标题/空行即为漂移）"
    );
}

// ============================================================
// C2-2 总量超 max_bytes：省略低优先级全局层 + `omitted <path>`，项目层完整
// ============================================================

#[test]
fn c2_total_budget_omits_low_priority_global_layers() {
    // Arrange：项目层 150B、高优先级全局层 150B、低优先级全局层 ~2KB
    let workspace = TempDir::new("budget-ws");
    let low = TempDir::new("budget-low");
    let high = TempDir::new("budget-high");
    let project_line = format!("# PROJECT-RULES-MARKER\n{}\n", "p".repeat(120));
    let high_line = format!("# HIGH-GLOBAL-MARKER\n{}\n", "h".repeat(120));
    let low_line = format!("# LOW-GLOBAL-MARKER\n{}\n", "w".repeat(2_000));
    workspace.write("AGENTS.md", &project_line);
    high.write("AGENTS.md", &high_line);
    let low_path = low.write("AGENTS.md", &low_line);
    // 全局层顺序：后者优先级更高（与 resolve_global_agent_homes 的既有约定一致）
    let homes = vec![low.path().to_path_buf(), high.path().to_path_buf()];
    let budget = InstructionBudget {
        max_bytes: 600,
        max_source_bytes: 1_048_576,
    };

    // Act
    let rendered = budgeted_constitution(&WorkspaceHotloader::new(), workspace.path(), &homes, budget);

    // Assert
    assert!(
        rendered.contains(project_line.trim_end()),
        "C2-2：项目层（最高优先级）内容必须完整保留，实际：{rendered:?}"
    );
    assert!(
        rendered.contains(high_line.trim_end()),
        "C2-2：高优先级全局层在预算内必须保留，实际：{rendered:?}"
    );
    assert!(
        !rendered.contains("# LOW-GLOBAL-MARKER"),
        "C2-2：低优先级全局层超预算必须被省略（不得截断后仍注入），实际：{rendered:?}"
    );
    assert!(
        rendered.contains("omitted"),
        "C2-2：被省略的层必须在渲染结果里留下 `omitted` 行，实际：{rendered:?}"
    );
    assert!(
        rendered.contains(&low_path.display().to_string()),
        "C2-2：`omitted` 行必须携带被省略层的来源路径 {:?}，实际：{rendered:?}",
        low_path.display().to_string()
    );
}

// ============================================================
// C2-3 单文件超 max_source_bytes：跳过并计入 omitted（不得载入截断内容）
// ============================================================

#[test]
fn c2_oversize_source_file_is_skipped_and_omitted() {
    let workspace = TempDir::new("source-ws");
    let global = TempDir::new("source-global");
    workspace.write("AGENTS.md", "# PROJECT-KEPT-MARKER\nshort rules\n");
    let big_path = global.write(
        "AGENTS.md",
        &format!("# OVERSIZE-GLOBAL-MARKER\n{}\n", "b".repeat(400)),
    );
    let homes = vec![global.path().to_path_buf()];
    let budget = InstructionBudget {
        max_bytes: 65_536,
        max_source_bytes: 120,
    };

    let rendered = budgeted_constitution(&WorkspaceHotloader::new(), workspace.path(), &homes, budget);

    assert!(
        rendered.contains("# PROJECT-KEPT-MARKER"),
        "C2-3：未超 max_source_bytes 的项目层必须保留，实际：{rendered:?}"
    );
    assert!(
        !rendered.contains("# OVERSIZE-GLOBAL-MARKER"),
        "C2-3：超过 max_source_bytes 的单文件必须整层跳过（禁止载入被截断内容），实际：{rendered:?}"
    );
    assert!(
        !rendered.contains(&"b".repeat(400)),
        "C2-3：被跳过的层不得留下任何正文片段，实际：{rendered:?}"
    );
    assert!(
        rendered.contains("omitted"),
        "C2-3：被跳过的单文件必须计入 `omitted` 行，实际：{rendered:?}"
    );
    assert!(
        rendered.contains(&big_path.display().to_string()),
        "C2-3：`omitted` 行必须携带被跳过文件的路径，实际：{rendered:?}"
    );
}

// ============================================================
// C2-4 会话静态前缀受预算约束且跨轮次字节不变 + notice
// ============================================================

#[test]
fn c2_session_static_prefix_is_budget_bounded_and_stable() {
    let workspace = TempDir::new("session-ws");
    let global = TempDir::new("session-global");
    let project_path = workspace.write("AGENTS.md", "# SESSION-PROJECT-RULES\nkeep me\n");
    // 单个全局层 70KB：低于默认 max_source_bytes(1MiB)，但单独就超过默认 max_bytes(64KiB)。
    global.write(
        "AGENTS.md",
        &format!("# HUGE-GLOBAL-LAYER\n{}\n", "g".repeat(70_000)),
    );
    let homes = vec![global.path().to_path_buf()];
    let hotloader = WorkspaceHotloader::new();
    let session_id = "wave42-c2-session-budget";

    // 第 1 轮：固化基线
    let (prefix_turn_1, notice_turn_1) =
        hotloader.resolve_session_prompt_context_with_globals(session_id, 1, workspace.path(), &homes);
    assert!(
        notice_turn_1.is_none(),
        "C2-4：首轮不得产生变更 notice，实际：{notice_turn_1:?}"
    );
    assert!(
        prefix_turn_1.contains("# SESSION-PROJECT-RULES"),
        "C2-4：项目层必须进入静态前缀，实际前缀长度 {}",
        prefix_turn_1.len()
    );
    assert!(
        !prefix_turn_1.contains("# HUGE-GLOBAL-LAYER"),
        "C2-4：超默认预算的全局层不得进入会话静态前缀（预算必须约束前缀装配），实际前缀长度 {}",
        prefix_turn_1.len()
    );
    assert!(
        prefix_turn_1.contains("omitted"),
        "C2-4：静态前缀必须对被省略的层留下 `omitted` 标记，实际前缀长度 {}",
        prefix_turn_1.len()
    );

    // 第 2 轮：项目宪法变更 → 已固化静态前缀必须字节不变，变更以追加 notice 表达
    fs::write(&project_path, "# SESSION-PROJECT-RULES v2\nkeep me\nlonger\n")
        .expect("改写项目 AGENTS.md 失败");
    let (prefix_turn_2, notice_turn_2) =
        hotloader.resolve_session_prompt_context_with_globals(session_id, 2, workspace.path(), &homes);
    assert_eq!(
        prefix_turn_1.as_bytes(),
        prefix_turn_2.as_bytes(),
        "C2-4：第 2 轮静态前缀必须字节完全不变（KV-Cache 头部不得被预算重算冲刷）"
    );
    assert!(
        notice_turn_2.is_some(),
        "C2-4：配置变更必须走 Append-Only notice，实际 None"
    );
}

// ============================================================
// C3-1 `.git` 标记使 find_project_root 上移
// ============================================================

#[test]
fn c3_find_project_root_walks_up_to_git_marker() {
    let repo = TempDir::new("root-repo");
    repo.mkdir(".git");
    let cwd = repo.mkdir("a/b/c");

    assert_eq!(
        find_project_root(&cwd, ".git", repo.path()),
        repo.path(),
        "C3-1：`.git` 目录标记必须使根上移到仓库目录"
    );

    // `.git` 为文件（worktree/submodule 形态）同样应被识别
    let worktree = TempDir::new("root-worktree");
    worktree.write(".git", "gitdir: /tmp/elsewhere\n");
    let worktree_cwd = worktree.mkdir("nested/deep");
    assert_eq!(
        find_project_root(&worktree_cwd, ".git", worktree.path()),
        worktree.path(),
        "C3-1：`.git` 文件标记必须同样被识别"
    );

    // cwd 自身即项目根
    assert_eq!(
        find_project_root(repo.path(), ".git", repo.path()),
        repo.path(),
        "C3-1：cwd 自身含标记时不得再上移"
    );
}

// ============================================================
// C3-2 bound 处停止（禁止爬到 $HOME / 越过调用方上界）
// ============================================================

#[test]
fn c3_find_project_root_stops_at_bound() {
    let repo = TempDir::new("bound-repo");
    repo.mkdir(".git");
    let bound = repo.mkdir("outer");
    let cwd = repo.mkdir("outer/a/b");

    let resolved = find_project_root(&cwd, ".git", &bound);
    assert_eq!(
        resolved, bound,
        "C3-2：向上探测必须停在调用方给定 bound（不得越过上界爬到仓库根/宿主 home）"
    );
    assert_ne!(
        resolved,
        repo.path(),
        "C3-2：不得越过 bound 命中外层 `.git` 标记"
    );
}

// ============================================================
// C3-3 ancestor_chain 顺序 root→cwd；与根探测组合
// ============================================================

#[test]
fn c3_ancestor_chain_order_is_root_to_cwd() {
    let root = TempDir::new("chain-root");
    root.mkdir(".git");
    let cwd = root.mkdir("a/b");
    let a = root.path().join("a");

    assert_eq!(
        ancestor_chain(root.path(), &cwd),
        vec![root.path().to_path_buf(), a, cwd.clone()],
        "C3-3：祖先链必须是 root→cwd 的浅→深序列（含两端）"
    );
    assert_eq!(
        ancestor_chain(root.path(), root.path()),
        vec![root.path().to_path_buf()],
        "C3-3：root==cwd 时祖先链只含自身"
    );

    // 与根探测组合：链首 = 探测到的项目根，链尾 = cwd
    let found = find_project_root(&cwd, ".git", root.path());
    let composed = ancestor_chain(&found, &cwd);
    assert_eq!(
        composed.first().map(PathBuf::as_path),
        Some(root.path()),
        "C3-3：组合链首必须是探测到的项目根"
    );
    assert_eq!(
        composed.last().map(PathBuf::as_path),
        Some(cwd.as_path()),
        "C3-3：组合链尾必须是 cwd（最高优先级）"
    );
    assert_eq!(
        composed.len(),
        3,
        "C3-3：链上必须逐级覆盖 root/a/b 三层，实际 {composed:?}"
    );
}

// ============================================================
// C3-4 local overlay 晚于同目录 AGENTS.md
// ============================================================

#[test]
fn c3_local_overlay_loads_after_base_in_same_dir() {
    let workspace = TempDir::new("overlay-ws");
    workspace.write("AGENTS.md", "# BASE-RULES-MARKER\nteam defaults\n");
    workspace.write("AGENTS.local.md", "# LOCAL-OVERLAY-MARKER\nmy overrides\n");
    let global = TempDir::new("overlay-global");
    global.write("AGENTS.md", "# GLOBAL-LAYER-MARKER\nuser defaults\n");
    let homes = vec![global.path().to_path_buf()];

    let rendered = budgeted_constitution(
        &WorkspaceHotloader::new(),
        workspace.path(),
        &homes,
        InstructionBudget::default(),
    );

    assert!(
        rendered.contains("# BASE-RULES-MARKER"),
        "C3-4：同目录 AGENTS.md 必须被加载，实际：{rendered:?}"
    );
    assert!(
        rendered.contains("# LOCAL-OVERLAY-MARKER"),
        "C3-4：AGENTS.local.md 不得被忽略（local overlay 必须加载），实际：{rendered:?}"
    );
    let base_at = rendered
        .find("# BASE-RULES-MARKER")
        .expect("C3-4：前置条件：基础层标记缺失");
    let overlay_at = rendered
        .find("# LOCAL-OVERLAY-MARKER")
        .expect("C3-4：前置条件：overlay 标记缺失");
    assert!(
        base_at < overlay_at,
        "C3-4：AGENTS.local.md 必须排在同目录 AGENTS.md 之后（后加载=更高优先级），实际 base@{base_at} overlay@{overlay_at}"
    );
}

// ============================================================
// C3-5 跨层去重：trim 后相同内容只出现一次（保留高优先级）
// ============================================================

#[test]
fn c3_trim_equal_content_is_injected_once() {
    let workspace = TempDir::new("dedupe-ws");
    let global = TempDir::new("dedupe-global");
    // 内容 trim 后相同，仅首尾空白不同。
    global.write(
        "AGENTS.md",
        "\n\n   # SHARED-RULES-MARKER   \nsame body line\n\n",
    );
    workspace.write(
        "AGENTS.md",
        "# SHARED-RULES-MARKER\nsame body line\n",
    );
    let homes = vec![global.path().to_path_buf()];

    let rendered = budgeted_constitution(
        &WorkspaceHotloader::new(),
        workspace.path(),
        &homes,
        InstructionBudget::default(),
    );

    assert_eq!(
        count_occurrences(&rendered, "# SHARED-RULES-MARKER"),
        1,
        "C3-5：trim 后相同的内容只允许出现一次（相同内容不得跨层重复注入），实际：{rendered:?}"
    );
    assert!(
        rendered.contains("same body line"),
        "C3-5：去重不得把唯一保留的那一份内容丢掉，实际：{rendered:?}"
    );
}

// ============================================================
// C3-6 技能 rank：项目层永远最高优先级（不论 rank 数值如何，项目级压过全局级）
// ============================================================
//
// 裁决依据（PA-148 / ADR 0045 冻结不变量）：`$PONY_AGENT_HOME` 只是**重定位产品全局根**，
// 仍属全局层，不得压过项目级技能。rank 表遵循 Lead 冻结值：
// 项目 `.dsh/skills`=100、项目 `.agents/skills`=200、custom=300、`$PONY_AGENT_HOME/skills`=400
// （全局带内最高）、`~/.dsh/skills`=500 / `~/.agents/skills`=510 / `~/.claude/skills`=520、
// 无法识别身份的注入根按 450+index 兜底。数值越小越优先，但项目带恒在全局带之上。

#[test]
fn c3_project_outranks_global_regardless_of_rank_values() {
    let _lock = env_lock();
    let home = TempDir::new("rank-home");
    let workspace = TempDir::new("rank-ws");
    home.mkdir("skills");
    let probe_name = "wave42-rank-probe";
    // 同名技能：全局根 `$PONY_AGENT_HOME/skills`（rank 400）与项目 `.agents/skills`（rank 200）
    home.write(
        &format!("skills/{probe_name}/SKILL.md"),
        &format!("---\nname: {probe_name}\ndescription: GLOBAL-RANK-400\n---\n\nglobal body\n"),
    );
    workspace.write(
        &format!(".agents/skills/{probe_name}/SKILL.md"),
        &format!("---\nname: {probe_name}\ndescription: PROJECT-RANK-200\n---\n\nproject body\n"),
    );
    // 单层技能用于锚定 rank 常量映射
    home.write(
        "skills/wave42-global-only/SKILL.md",
        "---\nname: wave42-global-only\ndescription: GLOBAL-ONLY\n---\n\nbody\n",
    );
    workspace.write(
        ".agents/skills/wave42-project-only/SKILL.md",
        "---\nname: wave42-project-only\ndescription: PROJECT-ONLY\n---\n\nbody\n",
    );
    let _env = EnvGuard::set(PONY_AGENT_HOME_ENV, home.path());

    let (skills, _) = WorkspaceHotloader::new().scan_skills_with_budget(
        workspace.path(),
        &[home.path().to_path_buf()],
        InstructionBudget::default(),
    );

    let by_name = |target: &str| -> Vec<&pony_agent_core::agent::workspace_hotloader::HotloadedSkill> {
        skills.iter().filter(|skill| skill.name == target).collect()
    };

    // 1) 同名冲突：项目级胜出（scope/description/rank 三重证据 + 只保留一条）
    let probe = by_name(probe_name);
    assert_eq!(
        probe.len(),
        1,
        "C3-6：同名技能只允许保留一条（冲突必须显式裁决），实际 {} 条",
        probe.len()
    );
    assert_eq!(
        probe[0].scope,
        AgentScope::Project,
        "C3-6：同名冲突必须由项目层胜出（项目层永远最高优先级，$PONY_AGENT_HOME 仍属全局层）"
    );
    assert_eq!(
        probe[0].description, "PROJECT-RANK-200",
        "C3-6：胜出者必须是项目 `.agents/skills` 里的那一份正文/描述"
    );
    assert_eq!(
        probe[0].rank, 200,
        "C3-6：项目 `.agents/skills` 的 rank 必须是 200"
    );

    // 2) 全局独有技能：$PONY_AGENT_HOME/skills = 400（全局带内最高）
    let global_only = by_name("wave42-global-only");
    assert_eq!(global_only.len(), 1, "C3-6：全局独有技能必须可见");
    assert_eq!(
        global_only[0].rank, 400,
        "C3-6：$PONY_AGENT_HOME/skills 的 rank 必须是 400"
    );

    // 3) 项目独有技能：项目 `.agents/skills` = 200
    let project_only = by_name("wave42-project-only");
    assert_eq!(project_only.len(), 1, "C3-6：项目独有技能必须可见");
    assert_eq!(
        project_only[0].rank, 200,
        "C3-6：项目 `.agents/skills` 的 rank 必须是 200"
    );

    // 4) 用数值暴露可读不变量：全局带 rank 恒大于项目带 rank
    assert!(
        global_only[0].rank > project_only[0].rank,
        "C3-6：全局层 rank({}) 必须大于项目层 rank({})，否则'项目层永远最高优先级'不变量被破坏",
        global_only[0].rank,
        project_only[0].rank
    );
}

// ============================================================
// C4-1 深层触达：浅→深多条目
// ============================================================

#[test]
fn c4_deep_touch_returns_instructions_shallow_to_deep() {
    let root = TempDir::new("touch-root");
    root.write("AGENTS.md", "# ROOT-RULE-MARKER\nroot rules\n");
    root.write("a/AGENTS.md", "# A-RULE-MARKER\na rules\n");
    root.write("a/b/AGENTS.md", "# B-RULE-MARKER\nb rules\n");
    root.write("a/b/deep.rs", "fn deep() {}\n");

    let entries = instructions_for_touched_path(
        root.path(),
        &root.path().join("a/b/deep.rs"),
        &BTreeSet::new(),
    );

    assert_eq!(
        entries.len(),
        3,
        "C4-1：root/a/b 三级各有一份 AGENTS.md，必须返回 3 条，实际 {entries:?}",
        entries = entries
            .iter()
            .map(|entry| entry.display_path.clone())
            .collect::<Vec<_>>()
    );
    assert!(
        entries[0].content.contains("# ROOT-RULE-MARKER"),
        "C4-1：第 1 条必须是根层指令，实际：{:?}",
        entries[0].content
    );
    assert!(
        entries[1].content.contains("# A-RULE-MARKER"),
        "C4-1：第 2 条必须是 mid 层指令（浅→深），实际：{:?}",
        entries[1].content
    );
    assert!(
        entries[2].content.contains("# B-RULE-MARKER"),
        "C4-1：第 3 条必须是文件所在层指令（最深），实际：{:?}",
        entries[2].content
    );
}

// ============================================================
// C4-2 已注入集合可跳过（自洽推导 key 形态，避免依赖未冻结的 display_path 格式）
// ============================================================

#[test]
fn c4_already_injected_paths_are_skipped() {
    let root = TempDir::new("touch-dedupe-root");
    root.write("AGENTS.md", "# ROOT-RULE-MARKER\nroot rules\n");
    root.write("a/AGENTS.md", "# A-RULE-MARKER\na rules\n");
    root.write("a/b/AGENTS.md", "# B-RULE-MARKER\nb rules\n");
    let touched = root.path().join("a/b/deep.rs");

    let first = instructions_for_touched_path(root.path(), &touched, &BTreeSet::new());
    assert_eq!(first.len(), 3, "C4-2：前置条件：首轮必须返回 3 条");

    // 用实现自己产出的 display_path 作为 key，再回灌 -> 只跳过该层。
    let mut already: BTreeSet<String> = BTreeSet::new();
    already.insert(first[1].display_path.clone());
    let second = instructions_for_touched_path(root.path(), &touched, &already);

    assert_eq!(
        second.len(),
        2,
        "C4-2：已注入的层必须被跳过，实际剩余 {second:?}",
        second = second
            .iter()
            .map(|entry| entry.display_path.clone())
            .collect::<Vec<_>>()
    );
    assert!(
        second
            .iter()
            .all(|entry| !entry.content.contains("# A-RULE-MARKER")),
        "C4-2：已注入的 `# A-RULE-MARKER` 层不得再次出现在注入结果里"
    );
    assert!(
        second
            .iter()
            .any(|entry| entry.content.contains("# ROOT-RULE-MARKER")),
        "C4-2：未注入的浅层必须仍然返回"
    );
    assert!(
        second
            .iter()
            .any(|entry| entry.content.contains("# B-RULE-MARKER")),
        "C4-2：未注入的深层必须仍然返回"
    );
}

// ============================================================
// C4-3 root 外路径不注入
// ============================================================

#[test]
fn c4_outside_root_paths_produce_no_instructions() {
    let root = TempDir::new("touch-inside");
    root.write("AGENTS.md", "# ROOT-RULE-MARKER\nroot rules\n");
    let outside = TempDir::new("touch-outside");
    outside.write("AGENTS.md", "# HOST-HOME-RULE-MARKER\nmust not inject\n");
    outside.write("x/deep.rs", "fn deep() {}\n");

    let outside_entries = instructions_for_touched_path(
        root.path(),
        &outside.path().join("x/deep.rs"),
        &BTreeSet::new(),
    );
    assert!(
        outside_entries.is_empty(),
        "C4-3：workspace 外路径不得产生任何指令段落，实际 {:?}",
        outside_entries
            .iter()
            .map(|entry| entry.display_path.clone())
            .collect::<Vec<_>>()
    );

    // 词法逃逸形态：root/../<outside>/x/deep.rs
    let escaped = root.path().join("../").join(
        outside
            .path()
            .file_name()
            .expect("C4-3：临时目录必须有文件名"),
    );
    let escaped_entries =
        instructions_for_touched_path(root.path(), &escaped.join("x/deep.rs"), &BTreeSet::new());
    assert!(
        escaped_entries.is_empty(),
        "C4-3：`..` 词法逃逸路径不得注入 workspace 外指令，实际 {:?}",
        escaped_entries
            .iter()
            .map(|entry| entry.display_path.clone())
            .collect::<Vec<_>>()
    );
}

// ============================================================
// C4-4 超 MAX_TOUCH_STEP_BYTES 截断并带标记
// ============================================================

#[test]
fn c4_oversize_instruction_is_truncated_with_marker() {
    let root = TempDir::new("touch-truncate");
    let tail_marker = "TAIL-MUST-BE-CUT-9f3a";
    let big_content = format!("{}{tail_marker}\n", "Q".repeat(MAX_TOUCH_STEP_BYTES + 4_096));
    root.write("big/AGENTS.md", &big_content);
    root.write("big/file.rs", "fn f() {}\n");

    let entries =
        instructions_for_touched_path(root.path(), &root.path().join("big/file.rs"), &BTreeSet::new());
    assert_eq!(entries.len(), 1, "C4-4：big 层只有一份指令");
    let content = &entries[0].content;

    assert!(
        content.len() <= MAX_TOUCH_STEP_BYTES,
        "C4-4：单步注入不得超过 MAX_TOUCH_STEP_BYTES({MAX_TOUCH_STEP_BYTES})，实际 {} 字节",
        content.len()
    );
    assert!(
        content.to_lowercase().contains("truncated"),
        "C4-4：截断必须携带 `truncated` 标记文案，实际尾部：{:?}",
        &content[content.len().saturating_sub(120)..]
    );
    assert!(
        !content.contains(tail_marker),
        "C4-4：截断后不得保留原文尾部（否则等于原文全量注入）"
    );
}

// ============================================================
// C4-5 workspace_read_file 前置 `=== Additional instructions` 段落
// ============================================================

#[test]
fn c4_workspace_read_file_prepends_additional_instructions() {
    let root = TempDir::new("touch-tool");
    root.write("AGENTS.md", "# ROOT-RULE-MARKER\nroot rules\n");
    root.write("a/AGENTS.md", "# A-RULE-MARKER\na rules\n");
    root.write("a/b/AGENTS.md", "# B-RULE-MARKER\nb rules\n");
    root.write("a/b/deep.rs", "fn deep() {}\n");

    let router = ToolRouter::with_workspace_root(root.path().to_path_buf());
    let call = ToolCall {
        call_id: None,
        name: "workspace_read_file".to_string(),
        arguments: json!({ "path": "a/b/deep.rs" }),
        plan: None,
    };
    let result = router.execute_with_workspace_root(&call, Some(root.path()));

    assert_eq!(
        result.status, "ok",
        "C4-5：前置条件：workspace_read_file 必须成功，实际 status={} output={}",
        result.status, result.output
    );
    assert!(
        result.output.contains("读取成功"),
        "C4-5：既有结果关键字段不得被破坏，实际：{}",
        result.output
    );
    assert!(
        result.output.contains("=== Additional instructions"),
        "C4-5：触碰深层文件后结果必须前置 `=== Additional instructions` 段落，实际：{}",
        result.output
    );
    assert!(
        result.output.contains("# ROOT-RULE-MARKER")
            && result.output.contains("# A-RULE-MARKER")
            && result.output.contains("# B-RULE-MARKER"),
        "C4-5：root/a/b 三级指令都必须注入，实际：{}",
        result.output
    );
    let section_at = result
        .output
        .find("=== Additional instructions")
        .expect("C4-5：前置条件：段落缺失");
    let content_at = result
        .output
        .find("读取成功")
        .expect("C4-5：前置条件：结果字段缺失");
    assert!(
        section_at < content_at,
        "C4-5：附加指令必须是**前置**追加（不得改变既有结果文本顺序之外的关键字段）"
    );
}
