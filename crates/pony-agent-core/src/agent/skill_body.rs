//! 技能正文按需加载（PA-149）：只读、受白名单约束、fail-closed。

use std::path::PathBuf;

use super::workspace_hotloader::{AgentScope, HotloadedSkill};

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

/// 单次加载正文的字节上限。
pub const MAX_SKILL_BODY_BYTES: usize = 262_144;

/// 按 HotloadedSkill 记录加载正文；路径必须落在受信只读根内。
#[allow(unused_variables)]
pub fn load_skill_body(
    skill: &HotloadedSkill,
    max_bytes: usize,
) -> Result<SkillBody, SkillBodyError> {
    unimplemented!("wave42 stub: load_skill_body")
}

/// 从当前工作区扫描结果里按名字解析技能（含全局层）。
#[allow(unused_variables)]
pub fn resolve_skill_by_name(
    workspace_root: &std::path::Path,
    name: &str,
) -> Option<HotloadedSkill> {
    unimplemented!("wave42 stub: resolve_skill_by_name")
}
