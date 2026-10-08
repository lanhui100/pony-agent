use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::SystemTime;

/// 文件元数据与指纹（用于判断是否需要重新读取与编译）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileFingerprint {
    pub path: PathBuf,
    pub mtime: Option<SystemTime>,
    pub file_size: u64,
    pub content_hash: u64, // FNV-1a 64-bit 快速内容校验
}

impl FileFingerprint {
    pub fn compute<P: AsRef<Path>>(path: P) -> Option<(Self, String)> {
        let path = path.as_ref();
        let metadata = fs::metadata(path).ok()?;
        let mtime = metadata.modified().ok();
        let file_size = metadata.len();
        let content = fs::read_to_string(path).ok()?;
        let content_hash = fnv1a_hash(content.as_bytes());

        Some((
            Self {
                path: path.to_path_buf(),
                mtime,
                file_size,
                content_hash,
            },
            content,
        ))
    }
}

pub fn fnv1a_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3u64);
    }
    hash
}

/// 热加载解析后的 Skill 摘要（支持 YAML Frontmatter 标准字段）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HotloadedSkill {
    pub name: String,
    pub description: String,
    pub user_invocable: bool,
    pub allowed_tools: Vec<String>,
    pub path: PathBuf,
    pub fingerprint: FileFingerprint,
}

/// 宪法文件内容缓存
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstitutionCache {
    pub source_path: PathBuf,
    pub content: String,
    pub fingerprint: FileFingerprint,
}

/// 会话级已固化的静态基线（用于保障历史 KV Cache 不被截断冲刷）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionBaselineSnapshot {
    pub constitution_hash: u64,
    pub constitution_content: String,
    pub skills_hash: u64,
    pub skills: Vec<HotloadedSkill>,
}

/// 针对会话的增量规则变更记录（DSH 风格的 Append-Only 变动事件）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionDeltaUpdate {
    pub constitution_changed: bool,
    pub updated_constitution_summary: Option<String>,
    pub added_or_updated_skills: Vec<HotloadedSkill>,
    pub removed_skill_names: Vec<String>,
}

/// 统一热加载与 KV-Cache 保护管理器
#[derive(Debug, Default)]
pub struct WorkspaceHotloader {
    constitution: RwLock<Option<ConstitutionCache>>,
    skills: RwLock<BTreeMap<String, HotloadedSkill>>,
    /// 会话基线注册表：session_id -> SessionBaselineSnapshot
    session_baselines: RwLock<HashMap<String, SessionBaselineSnapshot>>,
}

impl WorkspaceHotloader {
    pub fn new() -> Self {
        Self::default()
    }

    /// 热重载/缓存命中感知：加载工作区宪法文档（遵循 AGENTS.md / CLAUDE.md 标准）
    /// 返回值：(内容, 是否命中全局文件内存缓存)
    pub fn resolve_constitution<P: AsRef<Path>>(&self, workspace_root: P) -> Option<(String, bool)> {
        let root = workspace_root.as_ref();
        let candidates = ["AGENTS.md", "CLAUDE.md", ".claude/CLAUDE.md"];
        let mut target_path = None;
        for c in &candidates {
            let p = root.join(c);
            if p.exists() {
                target_path = Some(p);
                break;
            }
        }

        let target_path = target_path?;
        let metadata = fs::metadata(&target_path).ok()?;
        let mtime = metadata.modified().ok();
        let file_size = metadata.len();

        {
            let read_guard = self.constitution.read().unwrap();
            if let Some(cached) = read_guard.as_ref() {
                if cached.source_path == target_path
                    && cached.fingerprint.mtime == mtime
                    && cached.fingerprint.file_size == file_size
                {
                    return Some((cached.content.clone(), true));
                }
            }
        }

        let (fingerprint, content) = FileFingerprint::compute(&target_path)?;
        let mut write_guard = self.constitution.write().unwrap();
        *write_guard = Some(ConstitutionCache {
            source_path: target_path,
            content: content.clone(),
            fingerprint,
        });

        Some((content, false))
    }

    /// 热重载/缓存命中感知：多层级扫描并解析项目内 Skills（支持 .agents/skills 与 .claude/skills）
    /// 返回值：(Skills 列表有序且稳定输出, 是否完全命中缓存未变动)
    pub fn scan_skills<P: AsRef<Path>>(&self, workspace_root: P) -> (Vec<HotloadedSkill>, bool) {
        let root = workspace_root.as_ref();
        let mut skill_dirs = Vec::new();

        let agents_skills = root.join(".agents").join("skills");
        if agents_skills.is_dir() {
            skill_dirs.push(agents_skills);
        }
        let claude_skills = root.join(".claude").join("skills");
        if claude_skills.is_dir() && !skill_dirs.contains(&claude_skills) {
            skill_dirs.push(claude_skills);
        }

        let mut discovered = BTreeMap::new();
        let mut any_changed = false;

        for base_dir in skill_dirs {
            let entries = match fs::read_dir(&base_dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let folder_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                    let skill_file = path.join("SKILL.md");
                    if !skill_file.is_file() {
                        continue;
                    }

                    let metadata = match fs::metadata(&skill_file) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    let mtime = metadata.modified().ok();
                    let file_size = metadata.len();

                    let hit_cache = {
                        let read_guard = self.skills.read().unwrap();
                        if let Some(existing) = read_guard.get(&folder_name) {
                            if existing.fingerprint.mtime == mtime && existing.fingerprint.file_size == file_size {
                                discovered.insert(folder_name.clone(), existing.clone());
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    };

                    if !hit_cache {
                        any_changed = true;
                        if let Some((fingerprint, content)) = FileFingerprint::compute(&skill_file) {
                            let (parsed_name, description, user_invocable, allowed_tools) =
                                parse_skill_metadata(&content, &folder_name);
                            let skill = HotloadedSkill {
                                name: parsed_name,
                                description,
                                user_invocable,
                                allowed_tools,
                                path: skill_file,
                                fingerprint,
                            };
                            discovered.insert(folder_name, skill);
                        }
                    }
                }
            }
        }

        {
            let mut write_guard = self.skills.write().unwrap();
            if !any_changed && write_guard.len() == discovered.len() {
                let result = write_guard.values().cloned().collect();
                return (result, true);
            }
            *write_guard = discovered.clone();
        }

        let result = discovered.into_values().collect();
        (result, false)
    }

    /// 会话 KV-Cache 保护机制（Append-Only Delta Update）：
    /// 1. 若会话为首次轮次（turn_count <= 1 或无基线）：将当前宪法与 Skills 固化为该会话的永久静态前缀基线；
    /// 2. 若会话已有基线且文件发生变动：**绝对不篡改头部静态基线**（防止全量历史 KV Cache 失效），而是计算出增量 Delta，生成一条追加在队尾的 Notice 提示。
    pub fn resolve_session_prompt_context<P: AsRef<Path>>(
        &self,
        session_id: &str,
        turn_count: usize,
        workspace_root: P,
    ) -> (String, Option<String>) {
        let (current_constitution, _) = self.resolve_constitution(&workspace_root)
            .unwrap_or_else(|| (String::new(), true));
        let (current_skills, _) = self.scan_skills(&workspace_root);

        let const_hash = fnv1a_hash(current_constitution.as_bytes());
        let mut skill_hash_input = Vec::new();
        for s in &current_skills {
            skill_hash_input.extend_from_slice(s.name.as_bytes());
            skill_hash_input.extend_from_slice(s.description.as_bytes());
            skill_hash_input.extend_from_slice(&s.fingerprint.content_hash.to_le_bytes());
        }
        let skills_hash = fnv1a_hash(&skill_hash_input);

        let mut baselines = self.session_baselines.write().unwrap();

        if turn_count <= 1 || !baselines.contains_key(session_id) {
            // 首轮：固化静态 Baseline
            let baseline = SessionBaselineSnapshot {
                constitution_hash: const_hash,
                constitution_content: current_constitution.clone(),
                skills_hash,
                skills: current_skills.clone(),
            };
            let static_prefix = self.build_cached_prompt_prefix(
                Some(&current_constitution),
                &current_skills,
            );
            baselines.insert(session_id.to_string(), baseline);
            return (static_prefix, None);
        }

        // 非首轮：取出会话固化的基线，保持最头部静态 Prompt 前缀字节级完全相同（KV-Cache Hit 100%）
        let baseline = baselines.get(session_id).unwrap().clone();
        let stable_prefix = self.build_cached_prompt_prefix(
            Some(&baseline.constitution_content),
            &baseline.skills,
        );

        // 检测是否存在中途变更
        let const_changed = baseline.constitution_hash != const_hash;
        let skills_changed = baseline.skills_hash != skills_hash;

        if !const_changed && !skills_changed {
            return (stable_prefix, None);
        }

        // 计算增量并生成追加消息（Append-Only Notification）
        let mut delta_notice = String::from("Notice: Workspace configuration was updated mid-session:\n");
        if const_changed {
            delta_notice.push_str("- Workspace constitution/rules updated. Latest guidelines are active.\n");
        }
        if skills_changed {
            let old_names: Vec<_> = baseline.skills.iter().map(|s| &s.name).collect();
            let new_names: Vec<_> = current_skills.iter().map(|s| &s.name).collect();
            let added: Vec<_> = new_names.iter().filter(|n| !old_names.contains(n)).collect();
            let removed: Vec<_> = old_names.iter().filter(|n| !new_names.contains(n)).collect();
            if !added.is_empty() {
                delta_notice.push_str(&format!("- Newly installed skills: {:?}\n", added));
            }
            if !removed.is_empty() {
                delta_notice.push_str(&format!("- Removed skills: {:?}\n", removed));
            }
        }

        (stable_prefix, Some(delta_notice))
    }

    /// 构造静态保真的 Prompt 前缀块，确保 Prompt Cache 最佳命中
    pub fn build_cached_prompt_prefix(&self, constitution_content: Option<&str>, skills: &[HotloadedSkill]) -> String {
        let mut prefix = String::with_capacity(4096);

        if let Some(const_text) = constitution_content {
            if !const_text.trim().is_empty() {
                prefix.push_str("=== WORKSPACE CONSTITUTION & RULES ===\n");
                prefix.push_str(const_text.trim());
                prefix.push_str("\n\n");
            }
        }

        if !skills.is_empty() {
            prefix.push_str("=== AVAILABLE REUSABLE SKILLS ===\n");
            prefix.push_str("The following project-specific skills are available and loadable on demand:\n");
            for skill in skills {
                prefix.push_str(&format!("- `{}`: {}\n", skill.name, skill.description));
            }
            prefix.push_str("\n");
        }

        prefix
    }
}

/// 解析 SKILL.md 的标准 YAML Frontmatter
fn parse_skill_metadata(content: &str, fallback_name: &str) -> (String, String, bool, Vec<String>) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (
            fallback_name.to_string(),
            extract_fallback_description(content),
            true,
            Vec::new(),
        );
    }

    let mut name = fallback_name.to_string();
    let mut description = String::new();
    let mut user_invocable = true;
    let mut allowed_tools = Vec::new();

    let rest = &trimmed[3..];
    if let Some(end_idx) = rest.find("\n---") {
        let frontmatter = &rest[..end_idx];
        for line in frontmatter.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(colon_idx) = line.find(':') {
                let key = line[..colon_idx].trim();
                let val = line[colon_idx + 1..].trim().trim_matches('"').trim_matches('\'');
                match key {
                    "name" => {
                        if !val.is_empty() {
                            name = val.to_string();
                        }
                    }
                    "description" => {
                        description = val.to_string();
                    }
                    "user-invocable" => {
                        user_invocable = val.parse().unwrap_or(true);
                    }
                    "allowed-tools" => {
                        allowed_tools = val
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                    }
                    _ => {}
                }
            }
        }
    }

    if description.is_empty() {
        description = extract_fallback_description(content);
    }

    (name, description, user_invocable, allowed_tools)
}

fn extract_fallback_description(content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("---") {
            continue;
        }
        return trimmed.chars().take(200).collect();
    }
    "No description provided.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hotloader_cache_hit_and_reload() {
        let temp_dir = std::env::temp_dir().join(format!("pony_test_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&temp_dir).unwrap();

        let agents_md = temp_dir.join("AGENTS.md");
        fs::write(&agents_md, "# Test Rules v1\nDo not break.").unwrap();

        let hotloader = WorkspaceHotloader::new();

        let (content, hit) = hotloader.resolve_constitution(&temp_dir).unwrap();
        assert_eq!(content, "# Test Rules v1\nDo not break.");
        assert!(!hit);

        let (content2, hit2) = hotloader.resolve_constitution(&temp_dir).unwrap();
        assert_eq!(content2, content);
        assert!(hit2);

        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&agents_md, "# Test Rules v2\nUpdated.").unwrap();
        let (content3, hit3) = hotloader.resolve_constitution(&temp_dir).unwrap();
        assert_eq!(content3, "# Test Rules v2\nUpdated.");
        assert!(!hit3);

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_yaml_frontmatter_parsing() {
        let sample = r#"---
name: awesome-skill
description: Comprehensive auditing tool for code.
user-invocable: false
allowed-tools: Bash, Read
---

# Details
"#;
        let (name, desc, user_invocable, tools) = parse_skill_metadata(sample, "fallback");
        assert_eq!(name, "awesome-skill");
        assert_eq!(desc, "Comprehensive auditing tool for code.");
        assert!(!user_invocable);
        assert_eq!(tools, vec!["Bash", "Read"]);
    }

    #[test]
    fn test_session_kv_cache_preservation_on_mid_turn_edit() {
        let temp_dir = std::env::temp_dir().join(format!("pony_session_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&temp_dir).unwrap();

        let agents_md = temp_dir.join("AGENTS.md");
        fs::write(&agents_md, "# Baseline Rules").unwrap();

        let hotloader = WorkspaceHotloader::new();
        let session_id = "session_123";

        // 第 1 轮交互：创建基线
        let (prefix_turn_1, notice_turn_1) = hotloader.resolve_session_prompt_context(session_id, 1, &temp_dir);
        assert!(prefix_turn_1.contains("# Baseline Rules"));
        assert!(notice_turn_1.is_none());

        // 用户在第 2 轮中途修改了 AGENTS.md 宪法文件
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&agents_md, "# Modified Mid-Session Rules").unwrap();

        // 第 2 轮交互：静态前缀必须保持不变（KV-Cache 命中保护！），变动以增量 notice 输出
        let (prefix_turn_2, notice_turn_2) = hotloader.resolve_session_prompt_context(session_id, 2, &temp_dir);
        assert_eq!(prefix_turn_1, prefix_turn_2, "头部静态前缀必须严格相等，确保 KV Cache 不被冲刷！");
        assert!(notice_turn_2.is_some());
        assert!(notice_turn_2.unwrap().contains("constitution/rules updated"));

        fs::remove_dir_all(&temp_dir).unwrap();
    }
}
