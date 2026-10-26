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
use std::sync::{Arc, Barrier, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
        // 注意：这里**刻意不做 canonicalize**。夹具根保持"与生产 workspace root 相同的形态"——
        // 生产 root 来自会话注册（`workspace::normalize_workspace_root`：canonicalize 后**剥掉**
        // Windows `\\?\` 前缀），即普通绝对路径。保持这一形态才能如实暴露产品侧的
        // "plain root vs verbatim 解析结果"不一致（见 C4-5 用例上方的缺陷说明）。
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
    // Arrange：项目层 ~58B、高优先级全局层 ~57B、低优先级全局层 ~2KB
    //
    // 为什么把两层小内容从 150B 缩到 ~58B（**预算 600 与"保留/省略"期望均未改**）：
    // 产品预算按**渲染后字节**计，含每层 `## <标题> (<绝对路径>)` 标题行与末尾 `omitted <路径>`
    // 行（`workspace_hotloader.rs:1254-1259` 明确冻结此口径）。本地实测渲染代价为
    //   total ≈ 105 + 3*len(根路径) + 项目内容 + 高层内容
    // Linux 上根≈`/tmp/wave42-c2c4-budget-high-<pid>-<nanos>-<seq>`（≈55）→ total≈570，
    // 600 预算只剩 **30B** 余量；而 Windows 根含 `\\?\C:\Users\runneradmin\AppData\Local\Temp\`
    // （≈96）→ total≈681 > 600 ⇒ 连高优先级层都被 omit（CI 原样复现）。
    // 路径前缀无法缩短（OS 既定），故把**夹具内容**压到与平台无关的量级，恢复原有语义：
    // 「项目层 + 高优先级全局层都放得下，低优先级 ~2KB 必须被省略」。
    //
    // **夹具根长度上限 ≈140 字符**（实测：根 135 绿、167 红）。Windows 实际根 ≈96（余量 ~44）。
    // 若将来 CI runner 的 TEMP 根异常长而本用例转红：说明该口径下"路径也吃用户预算"已经
    // 实质影响可用性 —— 那属于**产品预算口径议题**（要把标题行/omitted 行排除出预算），
    // 不要靠继续缩夹具内容来凑，按议题走。
    // 目录标签也刻意取短：两条全局层路径 + 项目层路径都会被渲染进预算（标题行 / omitted 行），
    // 标签每多 1 字符就多消耗 3 字节预算。
    let workspace = TempDir::new("c2ws");
    let low = TempDir::new("c2lo");
    let high = TempDir::new("c2hi");
    let project_line = format!("# PROJECT-RULES-MARKER\n{}\n", "p".repeat(24));
    let high_line = format!("# HIGH-GLOBAL-MARKER\n{}\n", "h".repeat(24));
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
//
// 【已知产品缺陷（Windows 专属，已经上报 Lead，产品侧未改）】
// 本用例与 C4-7 在 windows-latest 上会红，原因**不是**并发、也不是预算，而是路径前缀形态不一致：
//   1) 产品 `resolve_workspace_path`（`tools.rs:4752`）返回的是 `candidate.canonicalize()` 的
//      **原始**结果——Windows 上带 verbatim 前缀 `\\?\C:\...`；
//   2) 而 `with_touched_instructions`（`tools.rs:3360-3390`）把**会话 root**（已剥掉 `\\?\`
//      前缀的普通绝对路径，见 `workspace::normalize_workspace_root`）传给
//      `instructions_for_touched_path`，后者用 `lexical_touched.starts_with(lexical_root)`
//      做边界闸门（`instruction_touch.rs:60-62`）——`Prefix(VerbatimDisk)` 与 `Prefix(Disk)`
//      不是同一组件 ⇒ 直接返回空 ⇒ 触达注入整段为空 ⇒ 段落缺失 / `with_section=0`。
// 本机（Linux）100% 复现方式：`TMPDIR=<符号链接目录>`，让 canonical 与给定路径不一致，
// 即可得到与 CI 逐字相同的 "0/8"。修法应落在产品侧（两处统一 `path_permission::normalize_canonical`
// 或改用 `canonical_resolved_root`），测试侧故意不 canonicalize 夹具根，以免把该缺陷掩盖成绿色。
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

// ============================================================
// Wave-42 追加红相（L3 双审查 FAIL 后追加；不修改上方已冻结的 20 条断言）
// ============================================================
// 追加契约点：
// - C2-5：项目层是最高优先级 —— 单层超 max_bytes 必须**截断**而非整层丢弃；
// - C3-7：跨层去重不得污染来源缓存 —— 被去重掉的全局层变更必须触发缓存失效；
// - C2-6：默认预算黄金字节（单一项目层原始字节语义）逐字节锁定；
// - C4-6：符号链接逃逸（workspace 内指令文件指向 workspace 外真实文件）不得注入。

// ============================================================
// C2-5 超大项目层：截断而非丢弃（禁止 omitted 掉最高优先级项目层）
// ============================================================

#[test]
fn c2_huge_project_layer_is_truncated_not_dropped() {
    // Arrange：单一项目 AGENTS.md ≈ 70KB，无全局层，默认预算 64KiB
    let workspace = TempDir::new("huge-project");
    let content = format!("# HUGE-PROJECT-RULE\n{}\n", "p".repeat(70_000));
    workspace.write("AGENTS.md", &content);
    let budget = InstructionBudget::default();

    // Act
    let rendered = budgeted_constitution(&WorkspaceHotloader::new(), workspace.path(), &[], budget);

    // Assert：项目层必须"截断保留"而不是"整层丢弃"
    assert!(
        rendered.contains("# HUGE-PROJECT-RULE"),
        "C2-5：项目层是最高优先级，超预算时必须截断保留首部，实际渲染 {} 字节：{:?}",
        rendered.len(),
        rendered.chars().take(200).collect::<String>()
    );
    assert!(
        rendered.to_lowercase().contains("truncated"),
        "C2-5：截断必须带 `truncated` 标记，实际渲染 {} 字节：{:?}",
        rendered.len(),
        rendered.chars().take(200).collect::<String>()
    );
    assert!(
        rendered.as_bytes().len() <= budget.max_bytes + 128,
        "C2-5：截断结果必须落在 max_bytes(65536)+128 标记留白内，实际 {} 字节",
        rendered.as_bytes().len()
    );
    assert!(
        !rendered.contains("omitted"),
        "C2-5：项目层禁止被整层丢弃（不得出现 `omitted`），实际渲染 {} 字节：{:?}",
        rendered.len(),
        rendered.chars().take(200).collect::<String>()
    );
}

// ============================================================
// C3-7 去重不得吞掉全局层变更：全局来源变更必须使缓存失效
// ============================================================

#[test]
fn c3_deduped_global_source_change_invalidates_cache() {
    // Arrange：全局 home 与 workspace 写**完全相同**内容的 AGENTS.md
    let workspace = TempDir::new("dedupe-cache-ws");
    let global = TempDir::new("dedupe-cache-global");
    let shared = "# SHARED-DEDUPE-RULE\nidentical copy\n";
    workspace.write("AGENTS.md", shared);
    let global_path = global.write("AGENTS.md", shared);
    let homes = vec![global.path().to_path_buf()];
    let hotloader = WorkspaceHotloader::new();
    let budget = InstructionBudget::default();

    // Act 1：首次解析
    let (first_text, first_hit) = hotloader
        .resolve_constitution_with_budget(workspace.path(), &homes, budget)
        .expect("C3-7：前置条件：首次解析必须返回内容");
    assert!(
        !first_hit,
        "C3-7：首次解析不得命中缓存（否则后续失效断言无意义）"
    );
    assert_eq!(
        count_occurrences(&first_text, "# SHARED-DEDUPE-RULE"),
        1,
        "C3-7：前置条件：trim 相同内容只保留一份，实际：{first_text:?}"
    );

    // Act 2：20ms 后改写被去重掉的全局层
    std::thread::sleep(Duration::from_millis(20));
    fs::write(&global_path, "# GLOBAL-UPDATED\nonly in global layer\n")
        .expect("改写全局 AGENTS.md 失败");

    // Assert：缓存必须失效，且新内容真正进入渲染结果
    let (second_text, second_hit) = hotloader
        .resolve_constitution_with_budget(workspace.path(), &homes, budget)
        .expect("C3-7：第二次解析必须返回内容");
    assert!(
        !second_hit,
        "C3-7：被去重的全局层内容变更必须使缓存失效（禁止被去重缓存吞掉），实际 hit=true"
    );
    assert!(
        second_text.contains("# GLOBAL-UPDATED"),
        "C3-7：变更后的全局层内容必须出现在渲染结果里，实际：{second_text:?}"
    );
}

// ============================================================
// C2-6 默认预算黄金字节：单一项目层必须逐字节等于原始文件内容
// ============================================================

#[test]
fn c2_default_budget_golden_bytes() {
    // Arrange：固定 fixture
    let workspace = TempDir::new("golden-ws");
    workspace.write("AGENTS.md", "# GOLDEN-PROJECT\nrules\n");

    // Act
    let (rendered, _hit) = WorkspaceHotloader::new()
        .resolve_constitution_with_budget(workspace.path(), &[], InstructionBudget::default())
        .expect("C2-6：前置条件：单一项目层必须返回内容");

    // Assert：黄金字节（禁止同函数自比，禁止只做包含断言）
    let expected = "# GOLDEN-PROJECT\nrules\n";
    assert_eq!(
        rendered.as_bytes(),
        expected.as_bytes(),
        "C2-6：默认预算下单一项目层必须逐字节等于原始 AGENTS.md 内容，实际：{rendered:?}"
    );
}

// ============================================================
// C4-6 符号链接逃逸：workspace 内指令文件指向 workspace 外文件不得注入
// ============================================================

#[test]
fn c4_symlinked_instruction_outside_workspace_is_not_injected() {
    let workspace = TempDir::new("symlink-ws");
    let outside = TempDir::new("symlink-outside");
    let secret = "# SECRET-OUTSIDE-CONTENT\nhost secrets\n";
    let outside_file = outside.write("AGENTS.md", secret);
    workspace.mkdir("sub");
    workspace.write("sub/main.rs", "fn main() {}\n");
    let touched = workspace.path().join("sub/main.rs");

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside_file, workspace.path().join("sub/AGENTS.md"))
            .expect("C4-6：创建符号链接失败");

        // 断言 1：纯函数层不注入 workspace 外内容
        let entries =
            instructions_for_touched_path(workspace.path(), &touched, &BTreeSet::new());
        assert!(
            entries
                .iter()
                .all(|entry| !entry.content.contains("SECRET-OUTSIDE-CONTENT")),
            "C4-6：符号链接指向 workspace 外时不得注入其内容，实际 {:?}",
            entries
                .iter()
                .map(|entry| entry.display_path.clone())
                .collect::<Vec<_>>()
        );

        // 断言 2：工具结果文本同样不得泄漏
        let router = ToolRouter::with_workspace_root(workspace.path().to_path_buf());
        let call = ToolCall {
            call_id: None,
            name: "workspace_read_file".to_string(),
            arguments: json!({ "path": "sub/main.rs" }),
            plan: None,
        };
        let result = router.execute_with_workspace_root(&call, Some(workspace.path()));
        assert_eq!(
            result.status, "ok",
            "C4-6：前置条件：workspace_read_file 必须成功，实际 status={} output={}",
            result.status, result.output
        );
        assert!(
            !result.output.contains("SECRET-OUTSIDE-CONTENT"),
            "C4-6：工具结果不得前置注入符号链接逃逸后的 workspace 外内容，实际：{}",
            result.output
        );
    }

    #[cfg(not(unix))]
    {
        // 非 unix 平台无法构造稳定符号链接逃逸：显式跳过（不计红相失败）。
        let _ = (&outside_file, &touched);
    }
}

// ============================================================
// Wave-42 L2-AT 对抗/并发破坏注入（末条追加；不修改上方任何既有断言）
// ============================================================
// 契约点 C4-7：共享同一 `Arc<ToolRouter>` 的 8 个并发 `workspace_read_file` 触碰同一深层文件时，
// 触达注入必须**每份结果最多一次**（并发下不得重复注入），且不得因互斥/竞态而全部丢失。
// 同步原语：`Barrier(8)` 让 8 线程同一时刻起跑（确定性，禁止 sleep 竞态）。

#[test]
fn c4_concurrent_touch_does_not_duplicate_injection() {
    const THREADS: usize = 8;

    // Arrange：root 与 mid 两层指令，触碰 a/b/main.rs
    let workspace = TempDir::new("race-touch");
    workspace.write("AGENTS.md", "# ROOT-RACE-MARKER\nroot rules\n");
    workspace.write("a/AGENTS.md", "# A-RACE-MARKER\na rules\n");
    workspace.write("a/b/main.rs", "fn main() {}\n");
    let root_agents = workspace.path().join("AGENTS.md");
    let a_agents = workspace.path().join("a/AGENTS.md");

    let router = Arc::new(ToolRouter::with_workspace_root(
        workspace.path().to_path_buf(),
    ));
    let root_path = workspace.path().to_path_buf();
    let barrier = Barrier::new(THREADS);

    // Act：8 线程同一 Barrier 起跑，各自独立构造同一调用并执行
    let results: Vec<(String, String)> = std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(THREADS);
        for _ in 0..THREADS {
            let router = Arc::clone(&router);
            let root_path = root_path.clone();
            let barrier = &barrier;
            handles.push(scope.spawn(move || {
                let call = ToolCall {
                    call_id: None,
                    name: "workspace_read_file".to_string(),
                    arguments: json!({ "path": "a/b/main.rs" }),
                    plan: None,
                };
                barrier.wait();
                let result = router.execute_with_workspace_root(&call, Some(root_path.as_path()));
                (result.status, result.output)
            }));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().expect("C4-7：并发线程发生 panic"))
            .collect()
    });

    // Assert：① 全部成功；② 每层指令/每层段落标题在单份结果内至多一次；③ 至少一份携带附加段落。
    //
    // 注意（对 Lead 原始措辞的契约级修正，见汇报）：C4-1 冻结语义是"**每个**来源目录各一段"，
    // 本 fixture 有 root 与 a 两级指令，因此单份结果的段落标题基线是 **2**（不是 1）。
    // 真正要守的不变量是"并发不得让**同一层**重复注入"，故按层分别计数：
    // 根层段落标题 ≤1、mid 层段落标题 ≤1、总标题数 ≤ 层数(2)，两级标记各 ≤1。
    let root_header = format!(
        "=== Additional instructions ({})",
        root_agents.display()
    );
    let a_header = format!("=== Additional instructions ({})", a_agents.display());
    let instruction_layers = 2usize;
    let mut with_section = 0usize;
    for (index, (status, output)) in results.iter().enumerate() {
        assert_eq!(
            status, "ok",
            "C4-7：第 {index} 个并发结果必须成功，实际 output={output}"
        );
        assert!(
            count_occurrences(output, &root_header) <= 1,
            "C4-7：第 {index} 个结果里根层指令段落最多出现 1 次（并发不得重复注入），实际：{output}"
        );
        assert!(
            count_occurrences(output, &a_header) <= 1,
            "C4-7：第 {index} 个结果里 mid 层指令段落最多出现 1 次（并发不得重复注入），实际：{output}"
        );
        assert!(
            count_occurrences(output, "=== Additional instructions") <= instruction_layers,
            "C4-7：第 {index} 个结果的附加段落标题数不得超过指令层数({instruction_layers})，实际：{output}"
        );
        assert!(
            count_occurrences(output, "# ROOT-RACE-MARKER") <= 1,
            "C4-7：第 {index} 个结果里根层指令最多出现 1 次，实际：{output}"
        );
        assert!(
            count_occurrences(output, "# A-RACE-MARKER") <= 1,
            "C4-7：第 {index} 个结果里 mid 层指令最多出现 1 次，实际：{output}"
        );
        if output.contains("=== Additional instructions") {
            with_section += 1;
        }
    }
    assert!(
        with_section >= 1,
        "C4-7：并发下必须至少有一份结果携带附加指令段落（不得因互斥/竞态全部丢失），实际 0/{THREADS}"
    );
    eprintln!(
        "C4-7 并发触达收据：threads={THREADS} ok={} with_section={with_section}",
        results.iter().filter(|(status, _)| status == "ok").count()
    );
}

// ============================================================
// Wave-42 L3-r2 复审反例追加（只加用例；上方 21 条既有断言一字未改）
// ============================================================
// 契约点：
// - C2-7：渲染结果字节数必须严格受 `max_bytes` 约束（标题/分隔/标记不得白送预算）；
// - C2-8：最高优先级层即便位于全局带，也必须"截断保留"而不是"整层省略"；
// - C2-9：来源文件不可读后恢复可读，必须重试读取（缓存不得把临时读失败永久固化）；
// - C4-8：触达 prelude 不得把成功读取顶成 `output_budget_exceeded`。

// ============================================================
// C2-7 渲染字节严格不超预算
// ============================================================

#[test]
fn c2_rendered_bytes_never_exceed_budget() {
    // 子例 A：3 个全局层各 ~78B，max_bytes=300
    let workspace = TempDir::new("render-budget-ws");
    let global_one = TempDir::new("render-budget-g1");
    let global_two = TempDir::new("render-budget-g2");
    let global_three = TempDir::new("render-budget-g3");
    let layer = |tag: char| format!("# LAYER-{tag}-MARKER\n{}\n", tag.to_string().repeat(60));
    global_one.write("AGENTS.md", &layer('a'));
    global_two.write("AGENTS.md", &layer('b'));
    global_three.write("AGENTS.md", &layer('c'));
    let homes = vec![
        global_one.path().to_path_buf(),
        global_two.path().to_path_buf(),
        global_three.path().to_path_buf(),
    ];
    let budget = InstructionBudget {
        max_bytes: 300,
        max_source_bytes: 1_048_576,
    };
    let rendered = budgeted_constitution(
        &WorkspaceHotloader::new(),
        workspace.path(),
        &homes,
        budget,
    );
    assert!(
        rendered.as_bytes().len() <= budget.max_bytes,
        "C2-7：渲染字节 {} 超过 max_bytes({})（标题/分隔/标记不得占用额外预算），实际：{rendered:?}",
        rendered.as_bytes().len(),
        budget.max_bytes
    );

    // 子例 B：max_bytes=10 的极紧预算仍须严格不超且非空
    let single = TempDir::new("render-budget-tiny");
    single.write("AGENTS.md", "# SMALL-PROJECT\nrules\n");
    let tiny_budget = InstructionBudget {
        max_bytes: 10,
        max_source_bytes: 1_048_576,
    };
    let tiny = budgeted_constitution(
        &WorkspaceHotloader::new(),
        single.path(),
        &[],
        tiny_budget,
    );
    assert!(
        !tiny.is_empty(),
        "C2-7：极紧预算下结果不得为空（必须保留可判读的截断信号）"
    );
    assert!(
        tiny.as_bytes().len() <= tiny_budget.max_bytes,
        "C2-7：极紧预算下渲染字节 {} 超过 max_bytes({})，实际：{tiny:?}",
        tiny.as_bytes().len(),
        tiny_budget.max_bytes
    );
}

// ============================================================
// C2-8 全局带最高优先级层：截断保留，不得整层省略
// ============================================================

#[test]
fn c2_oversize_global_top_layer_is_truncated_not_omitted() {
    let workspace = TempDir::new("huge-global-ws"); // 无 AGENTS.md
    let home = TempDir::new("huge-global-home");
    home.write(
        "AGENTS.md",
        &format!("# HUGE-GLOBAL-TOP\n{}\n", "h".repeat(70_000)),
    );
    let homes = vec![home.path().to_path_buf()];
    let budget = InstructionBudget::default();

    let rendered = budgeted_constitution(
        &WorkspaceHotloader::new(),
        workspace.path(),
        &homes,
        budget,
    );

    assert!(
        rendered.contains("# HUGE-GLOBAL-TOP"),
        "C2-8：全局带唯一/最高优先级层超预算时必须截断保留首部，实际渲染 {} 字节：{:?}",
        rendered.as_bytes().len(),
        rendered.chars().take(200).collect::<String>()
    );
    assert!(
        rendered.to_lowercase().contains("truncated"),
        "C2-8：截断必须带 `truncated` 标记，实际：{:?}",
        rendered.chars().take(200).collect::<String>()
    );
    assert!(
        !rendered.contains("omitted"),
        "C2-8：最高优先级层不得被整层省略（不得出现 `omitted`），实际：{:?}",
        rendered.chars().take(200).collect::<String>()
    );
    assert!(
        rendered.as_bytes().len() <= budget.max_bytes,
        "C2-8：截断结果必须落在 max_bytes({}) 内，实际 {} 字节",
        budget.max_bytes,
        rendered.as_bytes().len()
    );
}

// ============================================================
// C2-9 不可读来源在权限恢复后必须重试读取
// ============================================================

#[cfg(unix)]
#[test]
fn c2_unreadable_source_is_retried_after_recovery() {
    use std::os::unix::fs::PermissionsExt;

    /// 权限守卫：Drop（含 panic）时恢复可读，避免污染后续用例。
    struct PermGuard {
        path: PathBuf,
    }

    impl Drop for PermGuard {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(0o644));
        }
    }

    let workspace = TempDir::new("recovery-ws");
    let home = TempDir::new("recovery-home");
    let global_path = home.write("AGENTS.md", "# RECOVERY-LAYER-MARKER\nrecovered rules\n");
    let homes = vec![home.path().to_path_buf()];
    let hotloader = WorkspaceHotloader::new();
    let budget = InstructionBudget::default();

    let _guard = PermGuard {
        path: global_path.clone(),
    };
    fs::set_permissions(&global_path, fs::Permissions::from_mode(0o000))
        .expect("C2-9：设置 0o000 失败");

    // 第 1 次解析：层不可读 → 不得凭空注入
    let first_text = hotloader
        .resolve_constitution_with_budget(workspace.path(), &homes, budget)
        .map(|(text, _)| text)
        .unwrap_or_default();
    assert!(
        !first_text.contains("# RECOVERY-LAYER-MARKER"),
        "C2-9：不可读的来源不得被注入，实际：{first_text:?}"
    );

    // 恢复可读后第 2 次解析：必须重新读到该层
    fs::set_permissions(&global_path, fs::Permissions::from_mode(0o644))
        .expect("C2-9：恢复 0o644 失败");
    let (second_text, _hit) = hotloader
        .resolve_constitution_with_budget(workspace.path(), &homes, budget)
        .expect("C2-9：权限恢复后必须重新解析出内容（禁止返回 None）");
    assert!(
        second_text.contains("# RECOVERY-LAYER-MARKER"),
        "C2-9：来源恢复可读后必须重试读取（禁止把临时读失败固化进缓存），实际：{second_text:?}"
    );
}

// ============================================================
// C4-8 触达 prelude 不得破坏大文件读取的成功语义
// ============================================================
//
// [RED DEGRADED: PUBLIC-SEAM-ONLY] `GovernedToolExecutor` 仅有 `new`/`dispatcher`/`context`
// 公开方法，`DispatchBudgetConfig.max_result_bytes` 没有对外 execute 注入缝，集成测试无法
// 构造 governed 预算路径；故此处仅经 `ToolRouter` 公开入口断言
// 「16KiB 触达 prelude 不破坏 110KB 读取的 status=ok 语义」（legacy 路径无 tool 级字节上限）。
// 待公开 governed 执行缝落地后，应改为真实 governed 路径复测。

#[test]
fn c4_governed_read_stays_ok_with_touch_prelude() {
    let workspace = TempDir::new("governed-touch");
    workspace.write(
        "AGENTS.md",
        &format!("# BIG-TOUCH-RULE\n{}\n", "t".repeat(16 * 1024)),
    );
    workspace.write("a/big.txt", &"x".repeat(110 * 1024));

    let router = ToolRouter::with_workspace_root(workspace.path().to_path_buf());
    let call = ToolCall {
        call_id: None,
        name: "workspace_read_file".to_string(),
        arguments: json!({ "path": "a/big.txt" }),
        plan: None,
    };
    let result = router.execute_with_workspace_root(&call, Some(workspace.path()));

    assert_eq!(
        result.status, "ok",
        "C4-8：触达 prelude 后大文件读取必须仍然成功，实际 status={} output={}",
        result.status, result.output
    );
    assert!(
        !result.output.contains("output_budget_exceeded"),
        "C4-8：触达 prelude 不得把成功读取顶成输出预算超限，实际：{}",
        result.output.chars().take(300).collect::<String>()
    );
}

// ============================================================
// Wave-42 L3-r3 安全回归追加（只加用例；上方 24 条既有断言一字未改）
// ============================================================
// 契约点 C3-8：符号链接不得把 workspace 外文件伪装成宪法层注入；
// 同时正向钉子：真实普通文件必须仍被加载（防"一刀切收紧"导致功能回退）。

#[cfg(unix)]
#[test]
fn c3_symlinked_constitution_outside_workspace_is_not_injected() {
    let workspace = TempDir::new("symlink-constitution-ws");
    let outside = TempDir::new("symlink-constitution-outside");
    let outside_file = outside.write("secret.md", "# SECRET-CONSTITUTION\nhost secrets\n");
    let link = workspace.path().join("AGENTS.md");
    std::os::unix::fs::symlink(&outside_file, &link).expect("C3-8：创建宪法符号链接失败");

    // Act：无全局层，workspace 的 AGENTS.md 是指向 workspace 外的符号链接
    let rendered = WorkspaceHotloader::new()
        .resolve_constitution_with_budget(workspace.path(), &[], InstructionBudget::default())
        .map(|(text, _)| text)
        .unwrap_or_default();

    // Assert 1：符号链接逃逸的层必须被跳过（不得注入 workspace 外内容）
    assert!(
        !rendered.contains("SECRET-CONSTITUTION"),
        "C3-8：符号链接指向 workspace 外时不得注入其内容，实际渲染 {} 字节：{:?}",
        rendered.as_bytes().len(),
        rendered.chars().take(200).collect::<String>()
    );

    // Assert 2（正向钉子）：真实普通文件仍必须被加载
    let normal = TempDir::new("symlink-constitution-normal");
    normal.write("AGENTS.md", "# NORMAL-CONSTITUTION-MARKER\nteam rules\n");
    let normal_text =
        budgeted_constitution(&WorkspaceHotloader::new(), normal.path(), &[], InstructionBudget::default());
    assert!(
        normal_text.contains("# NORMAL-CONSTITUTION-MARKER"),
        "C3-8：真实普通文件 AGENTS.md 必须仍被加载（防过度收紧），实际：{normal_text:?}"
    );
}
