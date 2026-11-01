//! 技能正文按需加载（PA-149）：只读、受白名单约束、fail-closed。

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use super::path_permission::{trusted_skill_roots_with_policy, SkillTrustRoot};
use super::workspace_hotloader::{AgentScope, HotloadedSkill, WorkspaceHotloader};

/// 加载成功的技能正文。
pub struct SkillBody {
    pub name: String,
    pub scope: AgentScope,
    pub path: PathBuf,
    pub content: String,
    pub truncated: bool,
}

/// 技能正文加载失败原因（fail-closed 语义，禁止静默降级）。
#[derive(Debug, PartialEq, Eq)]
pub enum SkillBodyError {
    Unreadable(String),
    OutOfAllowedRoot(PathBuf),
    TooLarge { size: usize, limit: usize },
    UnknownSkill(String),
}

impl SkillBodyError {
    /// 结构化错误码（工具面直接消费，保持既有 `^[a-z][a-z0-9_]*$` 形状）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unreadable(_) => "skill_unreadable",
            Self::OutOfAllowedRoot(_) => "skill_out_of_allowed_root",
            Self::TooLarge { .. } => "skill_too_large",
            Self::UnknownSkill(_) => "unknown_skill",
        }
    }

    /// 面向调用方的中文说明。
    pub fn message(&self) -> String {
        match self {
            Self::Unreadable(reason) => format!("读取技能正文失败：{reason}。"),
            Self::OutOfAllowedRoot(path) => {
                format!("技能正文不在受信只读根内，已拒绝读取：{}。", path.display())
            }
            Self::TooLarge { size, limit } => {
                format!("技能正文 {size} 字节，超过单次加载上限 {limit} 字节。")
            }
            Self::UnknownSkill(name) => format!("未找到技能 `{name}`。"),
        }
    }
}

/// 单次加载正文的字节上限。
pub const MAX_SKILL_BODY_BYTES: usize = 262_144;

/// 技能正文文件名（canonical 目标必须仍是技能正文本身）。
const SKILL_BODY_FILE_NAME: &str = "SKILL.md";

/// 按 HotloadedSkill 记录加载全局层技能正文（workspace 内项目层请用
/// [`load_skill_body_for_workspace`] 显式传入 workspace root）。
pub fn load_skill_body(
    skill: &HotloadedSkill,
    max_bytes: usize,
) -> Result<SkillBody, SkillBodyError> {
    load_skill_body_for_workspace(skill, max_bytes, None)
}

/// 按 HotloadedSkill 记录加载正文；路径必须落在受信技能只读根内。
///
/// L3-B F2（TOCTOU / 符号链接 / FIFO）：
/// 1. `..` 组件直接拒绝；
/// 2. 用**分域信任策略根**（Wave-58 F2-3 / D1）选出该路径所属的受信根：全局根允许符号链接
///    解析到根外，项目级根保持严格 canonical 包含；
/// 3. **根归属先行**（L3 审查 B3）：先经与扫描**同一个** [`trust_skill_file_with_limit`]
///    （[`is_trusted_skill_file`] 的实现本体，仅尺寸参数化）判根归属 / 文件名 / 普通文件 / 尺寸，
///    越界目标一律
///    `OutOfAllowedRoot`，不得以 `TooLarge` 回显体积；
/// 4. `open` 后用**已打开句柄** `metadata()`（fstat）判普通文件与长度，再读内容 —— 判定与读取
///    作用于同一个 inode，避免 check-then-act 期间路径被替换；
/// 5. FIFO / 设备文件 / 其他非普通文件一律 `Unreadable`（不做阻塞式 open）。
///
/// 与扫描的一致性（F2-3）：授权判定收敛到**同一个**谓词；路径别名（symlink 前缀 / verbatim
/// 前缀 / 卷大小写）下用 canonical 形态复核同一谓词，严格根仍会重做 canonical 包含判定，
/// 因此不放宽任何安全约束（L3 审查 B2）。
pub fn load_skill_body_for_workspace(
    skill: &HotloadedSkill,
    max_bytes: usize,
    workspace_root: Option<&Path>,
) -> Result<SkillBody, SkillBodyError> {
    if skill
        .path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(SkillBodyError::OutOfAllowedRoot(skill.path.clone()));
    }

    // Wave-58 F2-3：分域策略受信根。无显式 workspace root 时只允许全局根（项目层必须显式
    // 传 root），杜绝 `*/.agents/skills/*/SKILL.md` 这类任意路径被结构判定放行（A7 fail-closed）。
    let trusted_roots = trusted_skill_roots_with_policy(workspace_root);
    let Some(trusted_root) = trusted_roots
        .iter()
        .find(|root| matches_trust_root(&skill.path, root))
    else {
        return Err(SkillBodyError::OutOfAllowedRoot(skill.path.clone()));
    };

    let canonical_path = skill
        .path
        .canonicalize()
        .map_err(|_| SkillBodyError::OutOfAllowedRoot(skill.path.clone()))?;
    // 3) 根归属 + 文件名 + 普通文件 + 尺寸（同一谓词；别名路径用 canonical 形态复核）。
    let trusted = match authorize_skill_path(
        &skill.path,
        &canonical_path,
        trusted_root,
        max_bytes,
    ) {
        Ok(trusted) => trusted,
        Err(SkillTrustRejection::TooLarge) => {
            // 授权（根归属 / 文件名 / 普通文件）已通过，才允许读取尺寸回显。
            let size = fs::metadata(&canonical_path)
                .map(|metadata| usize::try_from(metadata.len()).unwrap_or(usize::MAX))
                .unwrap_or(usize::MAX);
            return Err(SkillBodyError::TooLarge {
                size,
                limit: max_bytes,
            });
        }
        Err(_) => return Err(SkillBodyError::OutOfAllowedRoot(skill.path.clone())),
    };
    let canonical_path = trusted.canonical;
    let target_metadata = match fs::metadata(&canonical_path) {
        Ok(metadata) if metadata.file_type().is_file() => metadata,
        Ok(_) => {
            return Err(SkillBodyError::Unreadable(
                "技能正文不是普通文件".to_string(),
            ))
        }
        Err(error) => {
            return Err(SkillBodyError::Unreadable(format!(
                "元信息读取失败：{error}"
            )))
        }
    };
    let size = usize::try_from(target_metadata.len()).unwrap_or(usize::MAX);
    if size > max_bytes {
        // 超限一律拒绝，禁止截断后返回"成功"（open 前再确认一次，防竞态放大）。
        return Err(SkillBodyError::TooLarge {
            size,
            limit: max_bytes,
        });
    }

    let file = fs::File::open(&canonical_path)
        .map_err(|error| SkillBodyError::Unreadable(format!("打开失败：{error}")))?;
    // L3 r3 A9②：canonical→open 之间目标可能被替换；open 后比较句柄与 canonical 目标的
    // 文件身份（unix dev+ino），不一致即拒绝（判定与读取必须同一 inode）。
    let handle_metadata = file
        .metadata()
        .map_err(|error| SkillBodyError::Unreadable(format!("元信息读取失败：{error}")))?;
    if !handle_metadata.file_type().is_file() {
        return Err(SkillBodyError::Unreadable(
            "技能正文不是普通文件".to_string(),
        ));
    }
    if !same_file_identity(&handle_metadata, &target_metadata) {
        return Err(SkillBodyError::OutOfAllowedRoot(skill.path.clone()));
    }
    let handle_size = usize::try_from(handle_metadata.len()).unwrap_or(usize::MAX);
    if handle_size > max_bytes {
        return Err(SkillBodyError::TooLarge {
            size: handle_size,
            limit: max_bytes,
        });
    }

    // L3 r3 A4/A8：有界读取（take(max_bytes + 1)），绝不整文件入内存。
    let mut buffer: Vec<u8> = Vec::with_capacity(size.min(max_bytes).saturating_add(1));
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut buffer)
        .map_err(|error| SkillBodyError::Unreadable(format!("正文读取失败：{error}")))?;
    if buffer.len() > max_bytes {
        // fstat 与实际可读长度不一致（写入竞态）时同样按超限拒绝。
        return Err(SkillBodyError::TooLarge {
            size: buffer.len(),
            limit: max_bytes,
        });
    }
    let content = String::from_utf8(buffer)
        .map_err(|_| SkillBodyError::Unreadable("正文不是 UTF-8 文本".to_string()))?;

    Ok(SkillBody {
        name: skill.name.clone(),
        scope: skill.scope,
        path: skill.path.clone(),
        content,
        truncated: false,
    })
}

/// 文件身份比较（L3 r3 A9②）：unix 用 dev+ino；其他平台退化为长度+修改时间。
#[cfg(unix)]
fn same_file_identity(handle: &fs::Metadata, target: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    handle.dev() == target.dev() && handle.ino() == target.ino()
}

#[cfg(not(unix))]
fn same_file_identity(handle: &fs::Metadata, target: &fs::Metadata) -> bool {
    handle.len() == target.len() && handle.modified().ok() == target.modified().ok()
}

/// 扫描阶段有界预检（L3 r3 A8）：供 `WorkspaceHotloader::scan_skills*` 在
/// `FileFingerprint::compute`（整文件读）之前调用；返回 false 表示该 `SKILL.md`
/// 超过 [`MAX_SKILL_BODY_BYTES`]，应跳过并记 [`omitted_skill_marker`]。
pub fn skill_metadata_within_scan_budget(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(metadata) => {
            metadata.file_type().is_file()
                && usize::try_from(metadata.len())
                    .map(|len| len <= MAX_SKILL_BODY_BYTES)
                    .unwrap_or(false)
        }
        Err(_) => false,
    }
}

/// 扫描跳过标记（L3 r3 A8）：`omitted skill:<name>`。
pub fn omitted_skill_marker(name: &str) -> String {
    format!("omitted skill:{name}")
}

/// 技能文件信任判定失败原因（Wave-58 F2-3 / 契约 D1、D2）。
///
/// 扫描侧（`WorkspaceHotloader::scan_skills_report*`）把本枚举逐项映射为
/// [`crate::agent::workspace_hotloader::SkillOmissionReason`]，从而把「静默跳过」变成
/// `omitted skill:<name> (<reason>)` 可观测标记。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SkillTrustRejection {
    /// 路径 / 符号链接逃逸出受信根，或 canonical 目标不是技能正文。
    OutsideRoot,
    /// `SKILL.md` 超过 [`MAX_SKILL_BODY_BYTES`]。
    TooLarge,
    /// 候选入口不是普通文件（悬空符号链接、目录、FIFO、设备等）。
    NotAFile,
}

/// 通过信任判定的技能文件（canonical 目标 + 真实大小）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TrustedSkillFile {
    /// canonical 形态的 `SKILL.md`（判定与去重排序都以它为准）。
    pub canonical: PathBuf,
    /// 目标文件字节数（已保证 ≤ [`MAX_SKILL_BODY_BYTES`]）。
    pub file_size: u64,
}

/// [`is_trusted_skill_file`] 的**唯一实现**（返回精细原因，供扫描侧映射 omitted 理由）。
///
/// 判定顺序（任一步失败即 fail-closed 返回 `Err`，禁止 `unwrap` 后放行）：
/// 1. 路径不得含 `..` 组件；
/// 2. 词法路径必须属于 `trusted_root`（防止「任意路径 + 任意根」组合被放行）；
/// 3. 词法文件名必须是 `SKILL.md`；
/// 4. canonicalize（悬空符号链接 / 不存在 → `NotAFile`）；
/// 5. canonical 文件名必须是 `SKILL.md`（`SKILL.md -> /etc/shadow` 这类改名逃逸 → `OutsideRoot`）；
/// 6. `allow_symlink_escape = false` 时，canonical 目标必须严格包含在 canonical 根内；
/// 7. canonical 目标必须是普通文件；
/// 8. 大小必须 ≤ `max_bytes`（扫描侧传 [`MAX_SKILL_BODY_BYTES`]，加载侧传本次加载上限）。
///
/// **根归属（1-7）先于尺寸（8）**：越界目标不得以 `TooLarge` 形式回显体积（L3 审查 B3）。
pub(crate) fn trust_skill_file_with_limit(
    path: &Path,
    trusted_root: &Path,
    allow_symlink_escape: bool,
    max_bytes: usize,
) -> Result<TrustedSkillFile, SkillTrustRejection> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(SkillTrustRejection::OutsideRoot);
    }
    if !path.starts_with(trusted_root) {
        return Err(SkillTrustRejection::OutsideRoot);
    }
    if path.file_name().and_then(|name| name.to_str()) != Some(SKILL_BODY_FILE_NAME) {
        return Err(SkillTrustRejection::OutsideRoot);
    }
    let Ok(canonical) = path.canonicalize() else {
        // 悬空符号链接 / 目标不存在：不可解析一律不是可加载的技能正文。
        return Err(SkillTrustRejection::NotAFile);
    };
    if canonical.file_name().and_then(|name| name.to_str()) != Some(SKILL_BODY_FILE_NAME) {
        return Err(SkillTrustRejection::OutsideRoot);
    }
    if !allow_symlink_escape {
        let Ok(canonical_root) = trusted_root.canonicalize() else {
            return Err(SkillTrustRejection::OutsideRoot);
        };
        if !canonical.starts_with(&canonical_root) {
            return Err(SkillTrustRejection::OutsideRoot);
        }
    }
    let Ok(metadata) = fs::metadata(&canonical) else {
        return Err(SkillTrustRejection::NotAFile);
    };
    if !metadata.file_type().is_file() {
        return Err(SkillTrustRejection::NotAFile);
    }
    let file_size = metadata.len();
    if usize::try_from(file_size)
        .map(|size| size > max_bytes)
        .unwrap_or(true)
    {
        return Err(SkillTrustRejection::TooLarge);
    }
    Ok(TrustedSkillFile {
        canonical,
        file_size,
    })
}

/// 扫描侧入口：固定按 [`MAX_SKILL_BODY_BYTES`] 判尺寸。
pub(crate) fn trust_skill_file(
    path: &Path,
    trusted_root: &Path,
    allow_symlink_escape: bool,
) -> Result<TrustedSkillFile, SkillTrustRejection> {
    trust_skill_file_with_limit(
        path,
        trusted_root,
        allow_symlink_escape,
        MAX_SKILL_BODY_BYTES,
    )
}

/// 技能文件信任谓词 —— **扫描与加载共用的单一实现入口**（wave-58 契约 D1 / F2-3）。
///
/// 背景：审查缺陷 P1-6「列表里有、加载被拒」。扫描（`WorkspaceHotloader::scan_skills*`）
/// 与加载（[`load_skill_body_for_workspace`] / `skill_load` 工具）必须调用同一个谓词，
/// 否则全局根下的符号链接技能会「列得出、载不进」。
///
/// 分域授权语义（Lead 决策 D1，**不可由下游自行变更**）：
/// - `allow_symlink_escape = false`（项目级根：`<ws>/.agents/skills`、`<ws>/.claude/skills`、
///   `<ws>/.dsh/skills`）：严格 canonical 包含判定，canonical 目标必须落在 `trusted_root` 内。
///   workspace 内容是不可信输入，`SKILL.md -> /etc/shadow` 这类仓库内逃逸必须拒绝。
/// - `allow_symlink_escape = true`（用户级全局根：`~/.agents`、`~/.claude`、`$PONY_AGENT_HOME`、
///   `config_dir/pony-agent`）：允许技能目录 / 文件为符号链接并解析到根外
///   （`npx skills` 与 Claude Code 的标准安装形态），但**仍须**同时满足：
///   路径无 `..` 组件、canonical 目标是普通文件、文件名为 `SKILL.md`、
///   大小 ≤ [`MAX_SKILL_BODY_BYTES`]；任一不可读 / 无法解析即 fail-closed 返回 `false`。
///
/// 实现委托 [`trust_skill_file`]（同一实现，扫描侧复用其精细原因）；加载侧按本次加载上限
/// 调用 [`is_trusted_skill_file_with_limit`]，仍是**同一个**判定的尺寸参数化投影。
pub fn is_trusted_skill_file(path: &Path, trusted_root: &Path, allow_symlink_escape: bool) -> bool {
    trust_skill_file(path, trusted_root, allow_symlink_escape).is_ok()
}

/// 加载侧「该路径是否属于某个策略技能根」判定（Wave-58 F2-3）。
///
/// 先做词法包含（扫描结果里的 `path` 就是词法 `SKILL.md` 路径），再兜底比较 canonical
/// 形态（调用方传入已 canonicalize 的路径时仍能落到正确策略根）。
fn matches_trust_root(path: &Path, root: &SkillTrustRoot) -> bool {
    if path.starts_with(&root.root) {
        return true;
    }
    match (path.canonicalize(), root.canonical_root.as_ref()) {
        (Ok(canonical), Some(canonical_root)) => canonical.starts_with(canonical_root),
        _ => false,
    }
}

/// 加载侧授权复核（Wave-58 F2-3 / D1，L3 审查 B2）：语义上只调用**同一个**谓词
/// [`trust_skill_file_with_limit`]（= [`is_trusted_skill_file`] 的尺寸参数化投影）。
///
/// 先按词法路径复核；路径别名（symlink 前缀 / verbatim 前缀 / 卷大小写）导致词法前缀不成立时，
/// 再按 canonical 形态复核同一谓词。严格根（`allow_symlink_escape = false`）每次复核都会
/// 重做 canonical 包含判定，因此别名兜底不放宽边界；全局根本就允许逃逸，兜底不引入新权限。
fn authorize_skill_path(
    path: &Path,
    canonical: &Path,
    root: &SkillTrustRoot,
    max_bytes: usize,
) -> Result<TrustedSkillFile, SkillTrustRejection> {
    let lexical = trust_skill_file_with_limit(path, &root.root, root.allow_symlink_escape, max_bytes);
    if lexical.is_ok() {
        return lexical;
    }
    if canonical != path {
        if let Ok(trusted) = trust_skill_file_with_limit(
            canonical,
            &root.root,
            root.allow_symlink_escape,
            max_bytes,
        ) {
            return Ok(trusted);
        }
        if let Some(canonical_root) = root.canonical_root.as_deref() {
            if let Ok(trusted) = trust_skill_file_with_limit(
                canonical,
                canonical_root,
                root.allow_symlink_escape,
                max_bytes,
            ) {
                return Ok(trusted);
            }
        }
    }
    lexical
}

/// 从当前工作区扫描结果里按名字解析技能（含全局层）。
///
/// 未知名返回 `None`（调用方据此返回结构化失败，禁止以空正文冒充成功）。
pub fn resolve_skill_by_name(workspace_root: &Path, name: &str) -> Option<HotloadedSkill> {
    let requested = name.trim();
    if requested.is_empty() {
        return None;
    }

    let hotloader = WorkspaceHotloader::new();
    let (skills, _) = hotloader.scan_skills(workspace_root);

    skills.into_iter().find(|skill| {
        skill.name == requested
            || skill
                .path
                .parent()
                .and_then(|directory| directory.file_name())
                .and_then(|folder| folder.to_str())
                == Some(requested)
    })
}
