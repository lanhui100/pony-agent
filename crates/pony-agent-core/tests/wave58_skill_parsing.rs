//! Wave-58 验收测试（Test Agent 冻结，只读）：技能 frontmatter 解析与扫描/加载一致性。
//!
//! 契约依据：`.dev-team/contract-matrix-wave58.md`
//!   - F2-1 块标量（`|` / `>` / `|-` / `>-`）必须解析为真实描述文本
//!   - F2-2 描述长度超过 1024 字符时按 UTF-8 边界截断
//!   - F2-3 扫描与加载共用同一信任谓词（D1 分域：全局根符号链接可加载，
//!         项目根越界符号链接不列出且留 `outside_root`）
//!   - F2-4 去重键 = `frontmatter.name`（缺省回落目录名），败者留 `duplicate_name`
//!   - F2-5 超过 `MAX_SKILL_BODY_BYTES` 的技能被跳过并留 `too_large`
//!   - F2-6 `SkillScanReport` / `scan_skills_report*` 报告通道，既有 `scan_skills*` 语义不变
//!
//! 观测面（既有公开导出，未臆造 API）：
//! - `WorkspaceHotloader::scan_skills_with_globals` / `scan_skills_report_with_globals`；
//! - `skill_body::{is_trusted_skill_file, load_skill_body_for_workspace, resolve_skill_by_name,
//!   MAX_SKILL_BODY_BYTES}`。
//!
//! 隔离纪律（test-expert §4.1）：
//! - 所有 fixture 一律临时目录（`std::env::temp_dir()` + PID/纳秒/序号唯一名），`TempDir`
//!   在 Drop（含 panic 展开）时递归清理；
//! - 全局层一律走**显式注入**（`global_homes` 形参）；只有必须让 `skill_load` 的受信根
//!   包含临时全局根（F2-3 全局符号链接用例）时才用 `$PONY_AGENT_HOME`，并在 `ENV_LOCK`
//!   内串行 + `EnvGuard` 恢复原值；
//! - 符号链接用例是 Unix 专属（`#[cfg(unix)]`），Windows 上不伪造 link 语义。

/// 仅 Unix 符号链接用例的 EnvGuard 使用 ⇒ Windows 上不编译。
#[cfg(unix)]
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(unix)]
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use pony_agent_core::agent::skill_body::MAX_SKILL_BODY_BYTES;
#[cfg(unix)]
use pony_agent_core::agent::skill_body::{load_skill_body_for_workspace, resolve_skill_by_name};
use pony_agent_core::agent::workspace_hotloader::{
    AgentScope, SkillOmissionReason, SkillScanReport, WorkspaceHotloader,
};

/// 仅 Unix 专属符号链接用例会注入该环境变量；Windows 上不编译，避免 dead_code。
#[cfg(unix)]
const PONY_AGENT_HOME_ENV: &str = "PONY_AGENT_HOME";

/// 契约 F2-2 冻结的描述上限（单位：字符）。
const MAX_SKILL_DESCRIPTION_CHARS: usize = 1024;

// ============================================================
// 夹具
// ============================================================

/// 唯一临时目录，Drop（含 panic 展开）时递归清理。
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
            "wave58-f2-{tag}-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("创建 wave-58 F2 测试临时目录失败");
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    #[cfg(unix)]
    fn canonical(&self) -> PathBuf {
        self.root
            .canonicalize()
            .expect("临时目录 canonicalize 失败")
    }

    fn mkdir(&self, relative: &str) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(&path).expect("创建临时子目录失败");
        path
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

/// 环境变量守卫：Drop 时恢复修改前的取值（panic 路径同样恢复）。
///
/// 唯一消费者是 Unix 专属的符号链接用例（`#[cfg(unix)]`），故守卫本身同样按 unix 编译，
/// 以免在 Windows 上成为 dead_code（CI 以 `-D warnings` 编译测试目标）。
#[cfg(unix)]
struct EnvGuard {
    key: &'static str,
    previous: Option<OsString>,
}

#[cfg(unix)]
impl EnvGuard {
    fn set(key: &'static str, value: &Path) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

#[cfg(unix)]
impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

/// 环境变量串行锁：`$PONY_AGENT_HOME` 是进程级状态，读写它的用例统一在锁内串行。
#[cfg(unix)]
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[cfg(unix)]
fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// frontmatter 技能文件内容。
fn skill_file(frontmatter: &str, body: &str) -> String {
    format!("---\n{frontmatter}\n---\n\n{body}\n")
}

fn scan(workspace: &Path) -> Vec<pony_agent_core::agent::workspace_hotloader::HotloadedSkill> {
    // 显式空全局层：不读宿主 ~/.agents，也不断言宿主技能数量。
    let (skills, _hit) = WorkspaceHotloader::new().scan_skills_with_globals(workspace, &[]);
    skills
}

fn description_of(
    workspace: &Path,
    directory_name: &str,
) -> (String, AgentScope, u16) {
    let skills = scan(workspace);
    let skill = skills
        .iter()
        .find(|skill| {
            skill
                .path
                .parent()
                .and_then(|dir| dir.file_name())
                .and_then(|name| name.to_str())
                == Some(directory_name)
        })
        .unwrap_or_else(|| {
            panic!(
                "前置：技能目录 `{directory_name}` 必须被扫描到，实际：{:?}",
                skills
                    .iter()
                    .map(|skill| skill.name.clone())
                    .collect::<Vec<_>>()
            )
        });
    (skill.description.clone(), skill.scope, skill.rank)
}

/// 技能清单里「同一 name」的条目（用于 D3 去重断言）。
fn skills_named(
    workspace: &Path,
    name: &str,
) -> Vec<pony_agent_core::agent::workspace_hotloader::HotloadedSkill> {
    scan(workspace)
        .into_iter()
        .filter(|skill| skill.name == name)
        .collect()
}

/// D2 的稳定标记字面量：`omitted skill:<name> (<reason>)`。
fn omission_marker(name: &str, reason: SkillOmissionReason) -> String {
    format!("omitted skill:{name} ({})", reason.as_str())
}

// ============================================================
// F2-1：块标量（`|` / `>` / `|-` / `>-`）
// ============================================================

#[test]
fn f2_1_block_scalars_parse_to_real_description() {
    // Arrange：四种块标量 + 缩进续行
    let workspace = TempDir::new("f2-1");
    workspace.write(
        ".agents/skills/w58-literal/SKILL.md",
        &skill_file(
            "name: w58-literal\ndescription: |\n  w58 literal line one\n  w58 literal line two",
            "# body",
        ),
    );
    workspace.write(
        ".agents/skills/w58-folded/SKILL.md",
        &skill_file(
            "name: w58-folded\ndescription: >\n  w58 folded line one\n  w58 folded line two",
            "# body",
        ),
    );
    workspace.write(
        ".agents/skills/w58-literal-strip/SKILL.md",
        &skill_file(
            "name: w58-literal-strip\ndescription: |-\n  w58 strip line one\n  w58 strip line two",
            "# body",
        ),
    );
    workspace.write(
        ".agents/skills/w58-folded-strip/SKILL.md",
        &skill_file(
            "name: w58-folded-strip\ndescription: >-\n  w58 foldstrip line one\n  w58 foldstrip line two",
            "# body",
        ),
    );

    // Act
    let (literal, _, _) = description_of(workspace.path(), "w58-literal");
    let (folded, _, _) = description_of(workspace.path(), "w58-folded");
    let (literal_strip, _, _) = description_of(workspace.path(), "w58-literal-strip");
    let (folded_strip, _, _) = description_of(workspace.path(), "w58-folded-strip");

    // Assert：描述不得退化为块标量指示符本身
    for (label, description) in [
        ("w58-literal", &literal),
        ("w58-folded", &folded),
        ("w58-literal-strip", &literal_strip),
        ("w58-folded-strip", &folded_strip),
    ] {
        assert!(
            !matches!(description.trim(), "|" | ">" | "|-" | ">-"),
            "F2-1：`{label}` 的 description 不得是块标量指示符本身，实际：{description:?}"
        );
    }

    // Assert：字面标量以换行连接续行（`|` 与 `|-` 同连接语义）
    assert_eq!(
        literal.trim(),
        "w58 literal line one\nw58 literal line two",
        "F2-1：`description: |` 必须以换行连接续行，实际：{literal:?}"
    );
    assert_eq!(
        literal_strip.trim(),
        "w58 strip line one\nw58 strip line two",
        "F2-1：`description: |-` 必须以换行连接续行，实际：{literal_strip:?}"
    );

    // Assert：折叠标量以空格连接续行（`>` 与 `>-` 同连接语义）
    assert_eq!(
        folded.trim(),
        "w58 folded line one w58 folded line two",
        "F2-1：`description: >` 必须按 YAML 语义以空格连接续行，实际：{folded:?}"
    );
    assert_eq!(
        folded_strip.trim(),
        "w58 foldstrip line one w58 foldstrip line two",
        "F2-1：`description: >-` 必须按 YAML 语义以空格连接续行，实际：{folded_strip:?}"
    );
}

// ============================================================
// F2-2：描述超 1024 字符按 UTF-8 边界截断
// ============================================================

#[test]
fn f2_2_long_description_truncated_at_char_boundary() {
    // Arrange：ASCII 2000 字符 + 多字节 1200 个汉字（每字 3 字节）
    let workspace = TempDir::new("f2-2");
    let ascii_description = "a".repeat(2000);
    let wide_description = "技".repeat(1200);
    workspace.write(
        ".agents/skills/w58-long-ascii/SKILL.md",
        &skill_file(
            &format!("name: w58-long-ascii\ndescription: {ascii_description}"),
            "# body",
        ),
    );
    workspace.write(
        ".agents/skills/w58-long-wide/SKILL.md",
        &skill_file(
            &format!("name: w58-long-wide\ndescription: {wide_description}"),
            "# body",
        ),
    );

    // Act
    let (ascii, _, _) = description_of(workspace.path(), "w58-long-ascii");
    let (wide, _, _) = description_of(workspace.path(), "w58-long-wide");

    // Assert：截断到 1024 字符（不是 1024 字节，也不得切裂多字节字符）
    assert_eq!(
        ascii.chars().count(),
        MAX_SKILL_DESCRIPTION_CHARS,
        "F2-2：超长描述必须截断到 {MAX_SKILL_DESCRIPTION_CHARS} 字符，实际 {} 字符",
        ascii.chars().count()
    );
    assert!(
        ascii.chars().all(|ch| ch == 'a'),
        "F2-2：ASCII 截断结果不得掺入替换字符，实际尾部：{:?}",
        ascii.chars().rev().take(8).collect::<String>()
    );
    assert_eq!(
        wide.chars().count(),
        MAX_SKILL_DESCRIPTION_CHARS,
        "F2-2：多字节描述必须按 UTF-8 边界、以「字符」为单位截断到 \
         {MAX_SKILL_DESCRIPTION_CHARS} 字符（契约矩阵用词为「字符」），实际 {} 字符 / {} 字节",
        wide.chars().count(),
        wide.len()
    );
    assert!(
        wide.chars().all(|ch| ch == '技'),
        "F2-2：截断必须落在 UTF-8 边界上（不得产生半字符/替换字符），实际末尾字符：{:?}",
        wide.chars().rev().take(4).collect::<String>()
    );
}

// ============================================================
// F2-3：扫描与加载共用同一信任谓词（D1 分域）
// ============================================================

#[cfg(unix)]
#[test]
fn f2_3a_project_root_symlink_escape_is_not_listed() {
    // Arrange：项目技能根的 SKILL.md 符号链接指向工作区外文件（不可信输入）
    let workspace = TempDir::new("f2-3a-workspace");
    let outside = TempDir::new("f2-3a-outside");
    let secret = outside.write("secret.md", "# W58-OUTSIDE-SECRET\nhost 文件，绝不可读入提示词。\n");
    let escape_dir = workspace.mkdir(".agents/skills/w58-escape");
    std::os::unix::fs::symlink(&secret, escape_dir.join("SKILL.md"))
        .expect("创建越界符号链接失败");

    // Act
    let report = WorkspaceHotloader::new().scan_skills_report_with_globals(workspace.path(), &[]);
    let legacy_listed = scan(workspace.path());

    // Assert：项目根越界符号链接不得被列出（scan 与 load 同一谓词）
    assert!(
        !report
            .skills
            .iter()
            .any(|skill| skill.name == "w58-escape"),
        "F2-3：项目级根内的越界符号链接技能不得出现在可加载清单中，实际：{:?}",
        report
            .skills
            .iter()
            .map(|skill| skill.name.clone())
            .collect::<Vec<_>>()
    );
    assert!(
        !legacy_listed.iter().any(|skill| skill.name == "w58-escape"),
        "F2-3：既有 `scan_skills*`（委托新报告）也不得列出越界符号链接技能"
    );

    // Assert：拒绝必须可观测（D2 不允许静默丢弃）
    let omission = report
        .omitted
        .iter()
        .find(|omission| omission.name == "w58-escape")
        .unwrap_or_else(|| {
            panic!(
                "F2-3：越界符号链接技能必须产生 omitted 记录，实际 omitted：{:?}",
                report.omitted
            )
        });
    assert_eq!(
        omission.reason,
        SkillOmissionReason::OutsideRoot,
        "F2-3：越界符号链接的理由必须是 outside_root，实际：{:?}",
        omission.reason
    );
    assert_eq!(
        omission_marker("w58-escape", omission.reason),
        "omitted skill:w58-escape (outside_root)",
        "F2-3：省略标记必须形如 `omitted skill:<name> (<reason>)`"
    );

    // Assert：加载侧同步 fail-closed（列表没有 ⇒ 加载必须 unknown）
    assert!(
        resolve_skill_by_name(workspace.path(), "w58-escape").is_none(),
        "F2-3：越界符号链接技能不得被 `resolve_skill_by_name` 解析到"
    );
}

#[cfg(unix)]
#[test]
fn f2_3b_global_root_symlink_skill_listed_implies_loadable() {
    // Arrange：全局根下的技能目录是符号链接并指向根外（`npx skills` / Claude Code 标准形态）
    let workspace = TempDir::new("f2-3b-workspace");
    let global_home = TempDir::new("f2-3b-global-home");
    let installed = TempDir::new("f2-3b-installed");
    let body_marker = "W58-LINKED-GLOBAL-BODY";
    installed.write(
        "real-skill/SKILL.md",
        &skill_file(
            "name: w58-linked-global\ndescription: W58 linked global skill",
            &format!("# {body_marker}"),
        ),
    );
    let skills_dir = global_home.mkdir("skills");
    std::os::unix::fs::symlink(installed.path().join("real-skill"), skills_dir.join("w58-linked-global"))
        .expect("创建全局技能目录符号链接失败");

    // Act（加载侧受信根来自 $PONY_AGENT_HOME，故临时注入）
    let _env_serial = env_lock();
    let _env = EnvGuard::set(PONY_AGENT_HOME_ENV, global_home.path());
    let (skills, _hit) =
        WorkspaceHotloader::new().scan_skills_with_globals(workspace.path(), &[global_home.canonical()]);
    let listed = skills
        .iter()
        .find(|skill| skill.name == "w58-linked-global")
        .unwrap_or_else(|| {
            panic!(
                "F2-3 前置：全局根下的符号链接技能必须被列出，实际：{:?}",
                skills.iter().map(|skill| skill.name.clone()).collect::<Vec<_>>()
            )
        });
    let loaded = load_skill_body_for_workspace(
        listed,
        MAX_SKILL_BODY_BYTES,
        Some(workspace.path()),
    );

    // Assert：列出 ⇒ 必须可加载（消除「列表里有、加载被拒」）
    let body = loaded.unwrap_or_else(|error| {
        panic!(
            "F2-3：全局根符号链接技能既然被列出就必须可加载（D1 允许全局根符号链接逃逸），\
             实际加载失败：{error:?}"
        )
    });
    assert_eq!(body.name, "w58-linked-global", "F2-3：加载到的技能名必须一致");
    assert!(
        body.content.contains(body_marker),
        "F2-3：加载到的正文必须来自符号链接目标，实际正文：{:?}",
        body.content
    );
}

// ============================================================
// F2-4：去重键 = frontmatter name（D3）
// ============================================================

#[test]
fn f2_4_duplicate_name_same_rank_keeps_one_and_reports_omission() {
    // Arrange：同一技能根下两个目录声明同一 name（同 rank ⇒ 由 canonical 路径字典序裁决）
    let workspace = TempDir::new("f2-4a");
    workspace.write(
        ".agents/skills/w58-aaa/SKILL.md",
        &skill_file(
            "name: w58-dup-skill\ndescription: W58 duplicate from aaa",
            "# body",
        ),
    );
    workspace.write(
        ".agents/skills/w58-bbb/SKILL.md",
        &skill_file(
            "name: w58-dup-skill\ndescription: W58 duplicate from bbb",
            "# body",
        ),
    );

    // Act
    let report = WorkspaceHotloader::new().scan_skills_report_with_globals(workspace.path(), &[]);

    // Assert：只保留一条，且是 canonical 路径字典序更小者（确定性，不依赖遍历顺序）
    let kept: Vec<_> = report
        .skills
        .iter()
        .filter(|skill| skill.name == "w58-dup-skill")
        .collect();
    assert_eq!(
        kept.len(),
        1,
        "F2-4：同名技能必须去重为一条（去重键 = frontmatter name），实际 {} 条：{:?}",
        kept.len(),
        kept.iter().map(|skill| &skill.description).collect::<Vec<_>>()
    );
    assert_eq!(
        kept[0].description, "W58 duplicate from aaa",
        "F2-4：同 rank 同名冲突必须取 canonical 路径字典序更小者（w58-aaa），实际胜出：{:?}",
        kept[0].description
    );

    // Assert：败者必须产生 duplicate_name 省略记录
    let omission = report
        .omitted
        .iter()
        .find(|omission| omission.reason == SkillOmissionReason::DuplicateName)
        .unwrap_or_else(|| {
            panic!(
                "F2-4：被去重淘汰的候选必须产生 duplicate_name 记录，实际 omitted：{:?}",
                report.omitted
            )
        });
    assert_eq!(
        omission.name, "w58-dup-skill",
        "F2-4：省略记录的名字必须是去重键（frontmatter name），实际：{:?}",
        omission.name
    );
    assert_eq!(
        omission_marker(&omission.name, omission.reason),
        "omitted skill:w58-dup-skill (duplicate_name)",
        "F2-4：省略标记必须形如 `omitted skill:<name> (duplicate_name)`"
    );

    // Assert：既有入口（内部委托新报告）与报告结论一致
    assert_eq!(
        skills_named(workspace.path(), "w58-dup-skill").len(),
        1,
        "F2-4：既有 `scan_skills*` 也必须按 D3 去重"
    );
}

#[test]
fn f2_4_duplicate_name_rank_wins_over_path_order() {
    // Arrange：两个不同作用域根声明同名技能（.dsh rank=100 优于 .claude rank=300）
    let workspace = TempDir::new("f2-4b");
    workspace.write(
        ".dsh/skills/w58-zzz/SKILL.md",
        &skill_file(
            "name: w58-rank-skill\ndescription: W58 rank winner from dsh",
            "# body",
        ),
    );
    workspace.write(
        ".claude/skills/w58-aaa/SKILL.md",
        &skill_file(
            "name: w58-rank-skill\ndescription: W58 rank loser from claude",
            "# body",
        ),
    );

    // Act
    let kept = skills_named(workspace.path(), "w58-rank-skill");

    // Assert：同名冲突先比 rank（小者胜），路径字典序只是同 rank 的兜底
    assert_eq!(
        kept.len(),
        1,
        "F2-4：同名技能必须去重为一条，实际 {} 条",
        kept.len()
    );
    assert_eq!(
        kept[0].description, "W58 rank winner from dsh",
        "F2-4：同名冲突必须先比 rank（.dsh=100 < .claude=300），实际胜出：{:?}（rank={}）",
        kept[0].description, kept[0].rank
    );
}

// ============================================================
// F2-5：超过 MAX_SKILL_BODY_BYTES 的技能被跳过并留 too_large
// ============================================================

#[test]
fn f2_5_oversized_skill_body_skipped_with_marker() {
    // Arrange：SKILL.md 超过正文上限（frontmatter 合法，只是体积超限）
    let workspace = TempDir::new("f2-5");
    let oversized_body = "x".repeat(MAX_SKILL_BODY_BYTES + 1_024);
    workspace.write(
        ".agents/skills/w58-huge/SKILL.md",
        &skill_file(
            "name: w58-huge\ndescription: W58 oversized skill",
            &oversized_body,
        ),
    );
    workspace.write(
        ".agents/skills/w58-small/SKILL.md",
        &skill_file("name: w58-small\ndescription: W58 small skill", "# body"),
    );

    // Act
    let report = WorkspaceHotloader::new().scan_skills_report_with_globals(workspace.path(), &[]);

    // Assert：超限技能不得进入可加载清单，正常技能照旧
    assert!(
        !report.skills.iter().any(|skill| skill.name == "w58-huge"),
        "F2-5：超过 MAX_SKILL_BODY_BYTES({MAX_SKILL_BODY_BYTES}) 的技能不得进入可加载清单"
    );
    assert!(
        report.skills.iter().any(|skill| skill.name == "w58-small"),
        "F2-5 对照：同批次正常技能必须仍在清单内，实际：{:?}",
        report
            .skills
            .iter()
            .map(|skill| skill.name.clone())
            .collect::<Vec<_>>()
    );

    // Assert：超限必须可观测（too_large）
    let omission = report
        .omitted
        .iter()
        .find(|omission| omission.name == "w58-huge")
        .unwrap_or_else(|| {
            panic!(
                "F2-5：超限技能必须产生 omitted 记录，实际 omitted：{:?}",
                report.omitted
            )
        });
    assert_eq!(
        omission.reason,
        SkillOmissionReason::TooLarge,
        "F2-5：超限理由必须是 too_large，实际：{:?}",
        omission.reason
    );
    assert_eq!(
        omission_marker("w58-huge", omission.reason),
        "omitted skill:w58-huge (too_large)",
        "F2-5：省略标记必须形如 `omitted skill:<name> (too_large)`"
    );
}

// ============================================================
// F2-6：报告通道（SkillScanReport / scan_skills_report*）与既有入口一致性
// ============================================================

#[test]
fn f2_6_scan_report_channel_and_legacy_delegation() {
    // Arrange：一个正常技能 + 一组同名重复 + 一个非文件候选
    let workspace = TempDir::new("f2-6");
    workspace.write(
        ".agents/skills/w58-ok/SKILL.md",
        &skill_file("name: w58-ok\ndescription: W58 ok skill", "# body"),
    );
    workspace.write(
        ".agents/skills/w58-dup-a/SKILL.md",
        &skill_file("name: w58-report-dup\ndescription: W58 dup from a", "# body"),
    );
    workspace.write(
        ".agents/skills/w58-dup-b/SKILL.md",
        &skill_file("name: w58-report-dup\ndescription: W58 dup from b", "# body"),
    );
    // SKILL.md 是目录 ⇒ 非普通文件候选（not_a_file）
    workspace.mkdir(".agents/skills/w58-not-file/SKILL.md");

    // Act 1：报告结构契约
    let report: SkillScanReport =
        WorkspaceHotloader::new().scan_skills_report_with_globals(workspace.path(), &[]);

    // Assert：结构字段可直接消费（skills + omitted），且语义正确
    assert!(
        report.skills.iter().any(|skill| skill.name == "w58-ok"),
        "F2-6：报告 `skills` 必须包含可加载技能，实际：{:?}",
        report
            .skills
            .iter()
            .map(|skill| skill.name.clone())
            .collect::<Vec<_>>()
    );
    assert!(
        report
            .omitted
            .iter()
            .any(|omission| omission.name == "w58-report-dup"
                && omission.reason == SkillOmissionReason::DuplicateName),
        "F2-6：报告 `omitted` 必须记录同名去重败者，实际：{:?}",
        report.omitted
    );
    assert!(
        report
            .omitted
            .iter()
            .any(|omission| omission.name == "w58-not-file"
                && omission.reason == SkillOmissionReason::NotAFile),
        "F2-6：报告 `omitted` 必须记录非文件候选（not_a_file），实际：{:?}",
        report.omitted
    );
    assert!(
        report.skills.iter().all(|skill| skill
            .path
            .file_name()
            .and_then(|name| name.to_str())
            == Some("SKILL.md")),
        "F2-6：`skills` 里的每一项都必须指向可加载的 SKILL.md，实际：{:?}",
        report
            .skills
            .iter()
            .map(|skill| skill.path.clone())
            .collect::<Vec<_>>()
    );

    // Assert：D2 理由枚举的稳定字面量（测试与提示词直接消费，禁止改名）
    assert_eq!(SkillOmissionReason::OutsideRoot.as_str(), "outside_root");
    assert_eq!(SkillOmissionReason::TooLarge.as_str(), "too_large");
    assert_eq!(
        SkillOmissionReason::DuplicateName.as_str(),
        "duplicate_name"
    );
    assert_eq!(SkillOmissionReason::NotAFile.as_str(), "not_a_file");

    // Assert：空报告是合法初值
    let empty = SkillScanReport::default();
    assert!(
        empty.skills.is_empty() && empty.omitted.is_empty(),
        "F2-6：`SkillScanReport::default()` 必须是空报告"
    );

    // Act 2：既有入口（签名不变，内部委托新报告）
    let (legacy_skills, _hit) =
        WorkspaceHotloader::new().scan_skills_with_globals(workspace.path(), &[]);

    // Assert：委托一致性 —— 两个入口必须给出同一份可加载清单
    let report_view: Vec<_> = report
        .skills
        .iter()
        .map(|skill| (skill.name.clone(), skill.description.clone(), skill.scope, skill.rank))
        .collect();
    let legacy_view: Vec<_> = legacy_skills
        .iter()
        .map(|skill| (skill.name.clone(), skill.description.clone(), skill.scope, skill.rank))
        .collect();
    assert_eq!(
        report_view, legacy_view,
        "F2-6：既有 `scan_skills_with_globals` 必须委托 `scan_skills_report_with_globals`，\
         两者给出的技能集合（name/description/scope/rank）必须一致"
    );
}

// ============================================================
// D1 信任谓词单点（F2-3 的显式 API 契约）
// ============================================================

#[cfg(unix)]
#[test]
fn f2_3_trust_predicate_allows_global_escape_and_rejects_project_escape() {
    use pony_agent_core::agent::skill_body::is_trusted_skill_file;

    // Arrange：同一个越界目标，分别从「全局根」与「项目根」两种授权域观察
    let outside = TempDir::new("f2-3-predicate-outside");
    let secret = outside.write("SKILL.md", "# W58-OUTSIDE-SECRET\n");
    let global_root = TempDir::new("f2-3-predicate-global");
    let project_root = TempDir::new("f2-3-predicate-project");
    let global_link = global_root.mkdir("skills/w58-link");
    let project_link = project_root.mkdir(".agents/skills/w58-link");
    std::os::unix::fs::symlink(&secret, global_link.join("SKILL.md"))
        .expect("创建全局符号链接失败");
    std::os::unix::fs::symlink(&secret, project_link.join("SKILL.md"))
        .expect("创建项目符号链接失败");

    // Act + Assert：D1 分域授权
    assert!(
        is_trusted_skill_file(&global_link.join("SKILL.md"), &global_root.path().join("skills"), true),
        "F2-3：`allow_symlink_escape = true` 时，全局根下的符号链接技能必须被信任\
         （npx skills / Claude Code 标准安装形态）"
    );
    assert!(
        !is_trusted_skill_file(
            &project_link.join("SKILL.md"),
            &project_root.path().join(".agents/skills"),
            false
        ),
        "F2-3：`allow_symlink_escape = false` 时，项目级根的越界符号链接必须被拒绝（fail-closed）"
    );

    // Assert：即使允许逃逸，`..` 组件与「非 SKILL.md 文件名」仍必须拒绝
    assert!(
        !is_trusted_skill_file(
            &global_link.join("..").join("w58-link").join("SKILL.md"),
            &global_root.path().join("skills"),
            true
        ),
        "F2-3：含 `..` 组件的路径必须 fail-closed，即使 allow_symlink_escape = true"
    );
    let not_skill_file = outside.write("other.md", "# W58-NOT-SKILL-FILE\n");
    assert!(
        !is_trusted_skill_file(&not_skill_file, &global_root.path().join("skills"), true),
        "F2-3：文件名不是 SKILL.md 的路径必须 fail-closed（受信边界只针对技能正文）"
    );
}
