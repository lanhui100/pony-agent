//! 技能正文按需加载（PA-149）：只读、受白名单约束、fail-closed。

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use super::path_permission::trusted_skill_roots;
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
/// 2. 先 canonicalize，再用 **canonical 路径**做受信根包含判定（阻断 symlink 指向白名单外）；
/// 3. `open` 后用**已打开句柄** `metadata()`（fstat）判普通文件与长度，再读内容 —— 判定与读取
///    作用于同一个 inode，避免 check-then-act 期间路径被替换；
/// 4. FIFO / 设备文件 / 其他非普通文件一律 `Unreadable`（不做阻塞式 open）。
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

    let canonical_path = skill
        .path
        .canonicalize()
        .map_err(|_| SkillBodyError::OutOfAllowedRoot(skill.path.clone()))?;
    if canonical_path.file_name().and_then(|name| name.to_str()) != Some(SKILL_BODY_FILE_NAME) {
        return Err(SkillBodyError::OutOfAllowedRoot(skill.path.clone()));
    }
    let within_allowed_roots = trusted_skill_roots(workspace_root)
        .iter()
        .any(|root| canonical_path.starts_with(root));
    // L3 r3 A7：无显式 workspace root 时只允许全局根（项目层必须显式传 root），
    // 杜绝 `*/.agents/skills/*/SKILL.md` 这类任意路径被结构判定放行的 fail-open。
    if !within_allowed_roots {
        return Err(SkillBodyError::OutOfAllowedRoot(skill.path.clone()));
    }

    // L3 r3 A8/A4：metadata 预检（canonical 目标）+ 非普通文件拒绝；超限在 open 前即拒绝。
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
        // 超限一律拒绝，禁止截断后返回"成功"。
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
