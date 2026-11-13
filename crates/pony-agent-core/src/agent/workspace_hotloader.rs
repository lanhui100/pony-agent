// ponylint:file-size-exempt [until 2026-12-31] 需保留作为 hotloader 核心逻辑单文件以便维护
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::SystemTime;

use crate::agent::skill_body::{trust_skill_file, SkillTrustRejection};

/// 指令文档候选文件名的**单一来源**（wave-58 契约 F3-3 / 审查缺陷 P1-4）。
///
/// 根层（宪法注入：`resolve_constitution*`）与触达层（`instruction_touch`）必须
/// 共用本常量，禁止任何模块再自带第二份字面量，否则同一目录在不同入口加载的
/// 文件会不一致。
///
/// **顺序语义（严格分段，不得重排）**：
/// - 前 3 项 = 基础候选（base），见 `CONSTITUTION_CANDIDATES`，按优先级从高到低；
/// - 后 2 项 = 同目录个人覆盖层（local overlay），见 `LOCAL_OVERLAY_CANDIDATES`；
///   覆盖层排在基础候选之后加载，因此在同一目录内具有更高优先级。
///
/// 本波**不改变**「同目录多候选首命中」语义（契约 D6）：每级目录先取基础候选
/// 首命中，再取覆盖层首命中；`AGENTS.md` 与 `CLAUDE.md` 共存的语义变更留给 wave-59 + ADR。
pub const INSTRUCTION_CANDIDATES: [&str; 5] = [
    // —— 基础候选（base），高 → 低 ——
    "AGENTS.md",
    "CLAUDE.md",
    // 项目级同时兼容 Claude Code 的 `.claude/CLAUDE.md` 布局。
    ".claude/CLAUDE.md",
    // —— local overlay 候选，高 → 低 ——
    "AGENTS.local.md",
    "CLAUDE.local.md",
];

/// 基础候选：引用 [`INSTRUCTION_CANDIDATES`] 前 3 项（保持既有常量名 / 类型 / 语义可用）。
///
/// 说明：const 上下文不支持范围切片（`Index` 尚未 const 稳定），故逐项引用；
/// 这样仍是「单一来源」，改动候选表只需改 [`INSTRUCTION_CANDIDATES`]。
const CONSTITUTION_CANDIDATES: [&str; 3] = [
    INSTRUCTION_CANDIDATES[0],
    INSTRUCTION_CANDIDATES[1],
    INSTRUCTION_CANDIDATES[2],
];

/// local overlay 候选：引用 [`INSTRUCTION_CANDIDATES`] 后 2 项。
const LOCAL_OVERLAY_CANDIDATES: [&str; 2] = [INSTRUCTION_CANDIDATES[3], INSTRUCTION_CANDIDATES[4]];

/// 技能元数据解析所需的正文前缀上限：YAML frontmatter 足够，禁止整文件入内存。
const SKILL_FRONTMATTER_READ_LIMIT: usize = 65_536;

/// 用户级（全局）根目录的环境变量覆盖入口。
pub const PONY_AGENT_HOME_ENV: &str = "PONY_AGENT_HOME";

/// 用户级「自定义技能根」列表的环境变量入口（路径分隔符分隔：unix `:` / windows `;`）。
///
/// 每项都是**技能根目录本身**（其下直接放各技能子目录），不会自动再拼 `skills/`。
/// 只扩充技能发现面，**不**参与宪法（`AGENTS.md` / `CLAUDE.md`）分层——宪法分层契约仍由
/// ADR 0045 的四个全局根冻结，避免新增全局指令层导致提示词基线漂移。
pub const PONY_AGENT_SKILL_DIRS_ENV: &str = "PONY_AGENT_SKILL_DIRS";

/// 指令与技能的作用域（用户级全局 / 项目级）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AgentScope {
    Global,
    Project,
}

impl AgentScope {
    /// 稳定短标签，用于提示词标注与缓存键。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
        }
    }
}

/// 文件元数据与指纹（用于判断是否需要重新读取与编译）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileFingerprint {
    pub path: PathBuf,
    pub mtime: Option<SystemTime>,
    pub file_size: u64,
    pub content_hash: u64, // FNV-1a 64-bit 快速内容校验
}

impl FileFingerprint {
    /// 完整读取（既有语义；仅用于已知小文件的调用方）。
    pub fn compute<P: AsRef<Path>>(path: P) -> Option<(Self, String)> {
        Self::compute_bounded(path, usize::MAX)
    }

    /// **有界读取**：最多读取 `limit` 字节，避免超大文件整份进入内存。
    ///
    /// 指纹里的 `file_size` 仍是文件真实大小，`content_hash` 只覆盖有界前缀；
    /// 因此超出前缀的变更由 `mtime + file_size` 兜底检测（与既有缓存语义一致）。
    pub fn compute_bounded<P: AsRef<Path>>(path: P, limit: usize) -> Option<(Self, String)> {
        let path = path.as_ref();
        let metadata = fs::metadata(path).ok()?;
        let mtime = metadata.modified().ok();
        let file_size = metadata.len();
        let file = fs::File::open(path).ok()?;
        let mut bytes: Vec<u8> = Vec::new();
        file.take(limit as u64).read_to_end(&mut bytes).ok()?;
        if (file_size as usize) > limit {
            let keep = utf8_safe_prefix_len(&bytes, limit.min(bytes.len()));
            bytes.truncate(keep);
        }
        let content = String::from_utf8(bytes).ok()?;
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
    pub disable_model_invocation: bool,
    pub model_invocable: bool,
    pub path: PathBuf,
    pub fingerprint: FileFingerprint,
    /// 该 Skill 来自用户级全局根目录还是当前工作区。
    pub scope: AgentScope,
    /// 注入排序权重（wave-42 契约字段，数值越小越先注入）。
    pub rank: u16,
}

/// 技能扫描被跳过（omitted）的原因枚举（wave-58 契约 D2 / F2-6）。
///
/// 存在的意义：把「静默消失」变成可观测标记 `omitted skill:<name> (<reason>)`，
/// 让模型与用户都能知道哪些技能存在但不可用、以及为什么不可用。
/// `as_str` 的输出是稳定契约字面量（测试与提示词直接消费），禁止随意改名。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SkillOmissionReason {
    /// 符号链接 / 路径逃逸出受信根。项目级根保持严格 canonical 包含判定（D1）。
    OutsideRoot,
    /// `SKILL.md` 超过 [`crate::agent::skill_body::MAX_SKILL_BODY_BYTES`]（F2-5）。
    TooLarge,
    /// 同名技能在去重中败出（去重键见 D3：`frontmatter.name`，缺省回落目录名）。
    DuplicateName,
    /// 候选入口不是普通文件（悬空符号链接、目录、FIFO 等）。
    NotAFile,
    /// 技能明确配置为禁止模型直接调用（disable-model-invocation: true）。
    ModelInvocationDisabled,
}

impl SkillOmissionReason {
    /// 稳定 snake_case 标记（写入 `omitted skill:<name> (<reason>)` 与测试断言）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OutsideRoot => "outside_root",
            Self::TooLarge => "too_large",
            Self::DuplicateName => "duplicate_name",
            Self::NotAFile => "not_a_file",
            Self::ModelInvocationDisabled => "model_invocation_disabled",
        }
    }
}

/// 单条技能扫描跳过记录（wave-58 契约 F2-6）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillOmission {
    /// 技能名（D3 去重键：`frontmatter.name`，缺省回落目录名）。
    pub name: String,
    /// 跳过原因。
    pub reason: SkillOmissionReason,
    /// 触发跳过的候选路径（技能目录或其 `SKILL.md`）。
    pub path: PathBuf,
}

impl SkillOmission {
    /// 稳定可观测标记 `omitted skill:<name> (<reason>)`（契约 D2 / F2-6）。
    ///
    /// wave-58 L3-B F-1：`name` 来自不可信技能内容 / 目录名，必须转义后才可进入模型可见文本，
    /// 否则可伪造诊断行或段落标题。
    pub fn marker(&self) -> String {
        let name = sanitize_manifest_token(&self.name);
        let name = if name.is_empty() {
            sanitize_manifest_token(
                &self
                    .path
                    .parent()
                    .and_then(|directory| directory.file_name())
                    .and_then(|folder| folder.to_str())
                    .unwrap_or_default()
                    .to_string(),
            )
        } else {
            name
        };
        // 额外硬化：名字里若内嵌诊断前缀（`omitted skill:`）会制造歧义诊断串，直接剔除。
        let name = name.replace("omitted skill:", "");
        let name = sanitize_manifest_token(&name);
        let name = if name.is_empty() {
            "unnamed-skill".to_string()
        } else {
            truncate_description(&name, MAX_SKILL_NAME_CHARS)
        };
        format!("omitted skill:{} ({})", name, self.reason.as_str())
    }
}

/// 技能扫描报告（wave-58 契约 F2-6 / D2）。
///
/// `skills` 是**可加载**的技能清单（必须已过 [`crate::agent::skill_body::is_trusted_skill_file`]
/// 同一信任谓词），`omitted` 是被跳过的候选及其原因。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillScanReport {
    pub skills: Vec<HotloadedSkill>,
    pub omitted: Vec<SkillOmission>,
}

impl SkillScanReport {
    /// 全部 `omitted skill:<name> (<reason>)` 标记（供提示词 / 可观测面直接消费）。
    pub fn omission_markers(&self) -> Vec<String> {
        self.omitted.iter().map(SkillOmission::marker).collect()
    }
}

/// 指令注入预算上限（wave-42 契约桩）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstructionBudget {
    pub max_bytes: usize,
    pub max_source_bytes: usize,
}

impl Default for InstructionBudget {
    fn default() -> Self {
        Self {
            max_bytes: 65_536,
            max_source_bytes: 1_048_576,
        }
    }
}

/// 单层指令文档（用户级或项目级宪法文件）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstitutionLayer {
    pub scope: AgentScope,
    pub source_path: PathBuf,
    pub content: String,
    pub fingerprint: FileFingerprint,
}

/// 宪法文件内容缓存（可能由多个层级组成：全局层在前，项目层在后）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstitutionCache {
    /// 有界读取得到的层（去重前、读取顺序）。渲染阶段的去重/预算决策不写回缓存。
    pub layers: Vec<ConstitutionLayer>,
    /// **去重前全部来源**的读取状态快照。命中校验必须覆盖每个来源，
    /// 否则被去重丢弃的同文来源、以及读取失败后恢复的来源都将永远不可见。
    sources: Vec<ConstitutionSourceSnapshot>,
    /// 生成缓存时的单文件读取上限；上限变化意味着已读正文可能被提前截短，必须重读。
    max_source_bytes: usize,
}

/// 单个宪法来源的读取状态快照（缓存身份 + 预算判定输入）。
///
/// 命中语义（诚实假设）：成功读取且 `mtime + file_size` 均未变化的来源视为未变更；
/// 「同尺寸 + 同 mtime 的原地成功改写」不在本波检测范围（需逐次内容哈希比对）。
/// 反之，任一来源**读取失败**都会把 `read_ok` 记为 false，并使整份缓存 miss：
/// 即使权限/文件恢复后 mtime 与 size 完全不变，下次调用也必然重试读盘。
#[derive(Clone, Debug, PartialEq, Eq)]
struct ConstitutionSourceSnapshot {
    scope: AgentScope,
    path: PathBuf,
    mtime: Option<SystemTime>,
    file_size: u64,
    /// 是否成功读取并解码正文（false → 缓存永不命中，强制重试）。
    read_ok: bool,
    /// 单文件是否超过 `max_source_bytes`（正文只保留首部，最高优先级层截断保留）。
    oversized: bool,
    /// 有界读取到的正文（oversized 时为文件首部，按 UTF-8 边界安全截断）。
    content: String,
    content_hash: u64,
}

impl ConstitutionSourceSnapshot {
    /// 有界读取来源：最多读 `max_source_bytes + 1` 字节以判定是否超限，
    /// 避免超大文件整份进入内存。
    fn capture(scope: AgentScope, path: &Path, max_source_bytes: usize) -> Self {
        let metadata = fs::metadata(path).ok();
        let mtime = metadata
            .as_ref()
            .and_then(|metadata| metadata.modified().ok());
        let file_size = metadata.map(|metadata| metadata.len()).unwrap_or(0);

        let mut read_ok = false;
        let mut oversized = false;
        let mut content = String::new();
        if let Ok(file) = fs::File::open(path) {
            let mut bytes: Vec<u8> = Vec::new();
            let limit = (max_source_bytes as u64).saturating_add(1);
            if file.take(limit).read_to_end(&mut bytes).is_ok() {
                oversized = bytes.len() > max_source_bytes;
                if oversized {
                    let keep = utf8_safe_prefix_len(&bytes, max_source_bytes);
                    bytes.truncate(keep);
                }
                if let Ok(decoded) = String::from_utf8(bytes) {
                    read_ok = true;
                    content = decoded;
                }
            }
        }

        Self {
            scope,
            path: path.to_path_buf(),
            mtime,
            file_size,
            read_ok,
            oversized,
            content_hash: fnv1a_hash(content.as_bytes()),
            content,
        }
    }

    fn to_layer(&self) -> ConstitutionLayer {
        ConstitutionLayer {
            scope: self.scope,
            source_path: self.path.clone(),
            content: self.content.clone(),
            fingerprint: FileFingerprint {
                path: self.path.clone(),
                mtime: self.mtime,
                file_size: self.file_size,
                content_hash: self.content_hash,
            },
        }
    }

    /// 当前磁盘元数据是否与快照一致（来源被删除也视为不一致）。
    fn metadata_matches(&self) -> bool {
        match fs::metadata(&self.path) {
            Ok(metadata) => {
                self.file_size == metadata.len() && self.mtime == metadata.modified().ok()
            }
            Err(_) => false,
        }
    }
}

/// 取字节序列中不超过 `max_bytes` 的 UTF-8 安全前缀长度（不切裂多字节字符）。
fn utf8_safe_prefix_len(bytes: &[u8], max_bytes: usize) -> usize {
    if bytes.len() <= max_bytes {
        return bytes.len();
    }
    let mut end = max_bytes;
    while end > 0 && (bytes[end] & 0xC0) == 0x80 {
        end -= 1;
    }
    end
}

/// 会话级已固化的静态基线（用于保障历史 KV Cache 不被截断冲刷）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionBaselineSnapshot {
    pub constitution_hash: u64,
    pub constitution_content: String,
    pub skills_hash: u64,
    pub skills: Vec<HotloadedSkill>,
    /// 固化基线时使用的注入预算（wave-42 契约字段）。
    pub budget: InstructionBudget,
}

/// 针对会话的增量规则变更记录（DSH 风格的 Append-Only 变动事件）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionDeltaUpdate {
    pub constitution_changed: bool,
    pub updated_constitution_summary: Option<String>,
    pub added_or_updated_skills: Vec<HotloadedSkill>,
    pub removed_skill_names: Vec<String>,
}

/// 会话基线注册表的容量上限（Wave-58 F2-7，Lead 裁决 1024）。
///
/// 背景：`session_baselines` 原为无界 `HashMap`，子代理会话 ID 每次新建，长驻进程会无界增长。
/// 超限后按 LRU 淘汰；淘汰一律输出 `[pony-agent][hotloader]` 结构化诊断，**可观测而非静默**，
/// 且被淘汰会话下一轮会以 notice 明确告知「前缀已 re-baseline」（不静默改写）。
pub const MAX_SESSION_BASELINES: usize = 1024;

/// 会话基线表（Wave-58 F2-7）：`session_id -> (快照, 已固化前缀字节, 最近使用序号)`。
///
/// LRU 写法与 `tools.rs` 的 `TouchedInstructionMemory` 对齐（单调 `sequence` + `last_used`
/// 最小者先淘汰），避免引入额外依赖。
/// 会话基线复合键（wave-58 L3-B F-2）：`(canonical workspace root, session_id)`。
///
/// 只按 `session_id` 分桶会让「同一 session 换绑工作区」整体回放旧工作区前缀（跨工作区串味），
/// 因此 workspace 必须参与键；同一 session 同一 workspace 时行为与既有实现完全一致。
type SessionBaselineKey = (String, String);

#[derive(Debug, Default)]
struct SessionBaselineRegistry {
    entries: HashMap<SessionBaselineKey, SessionBaselineEntry>,
    /// 单调递增访问序号（LRU 淘汰依据）。
    sequence: u64,
}

#[derive(Debug, Clone)]
struct SessionBaselineEntry {
    snapshot: SessionBaselineSnapshot,
    /// 首轮固化时使用的技能清单预算（wave-58 D5' / L3-裁决 A）。
    ///
    /// 回放时必须与本次传入的预算比对：预算变化（如 provider 窗口切换）不能用旧清单字节，
    /// 而要用**基线内容**按新预算重渲染。
    catalog_budget: SkillCatalogBudget,
    /// 首轮扫描期的 omitted 诊断（wave-58 L3-B F-3）：预算变化触发重渲染时必须回放同一份标记，
    /// 否则该轮 `outside_root` / `too_large` / `duplicate_name` 会静默消失。
    omitted: Vec<SkillOmission>,
    /// 首轮固化的**渲染结果**字节（回放即返回它，保证第 2 轮起前缀字节级不变）。
    prefix: String,
    /// 最近一次访问（读或写）时的访问序号。
    last_used: u64,
}

impl SessionBaselineRegistry {
    /// 触发一次访问并返回当前序号（饱和递增，永不 panic）。
    fn tick(&mut self) -> u64 {
        self.sequence = self.sequence.saturating_add(1);
        self.sequence
    }

    /// 读取基线条目并标记该会话为最近使用；不存在时返回 `None`（复合键：workspace + session）。
    fn touch_entry(&mut self, key: &SessionBaselineKey) -> Option<SessionBaselineEntry> {
        let now = self.tick();
        let entry = self.entries.get_mut(key)?;
        entry.last_used = now;
        Some(entry.clone())
    }

    /// 写入基线并做 LRU 淘汰（刚写入的条目永不作为淘汰对象）。
    fn insert_entry(
        &mut self,
        key: SessionBaselineKey,
        snapshot: SessionBaselineSnapshot,
        catalog_budget: SkillCatalogBudget,
        omitted: Vec<SkillOmission>,
        prefix: String,
    ) {
        let now = self.tick();
        let protected = key.clone();
        self.entries.insert(
            key,
            SessionBaselineEntry {
                snapshot,
                catalog_budget,
                omitted,
                prefix,
                last_used: now,
            },
        );
        self.evict_overflow(&protected);
    }

    /// 容量超限时淘汰最久未使用的条目（Wave-58 F2-7）：逐条输出结构化诊断。
    fn evict_overflow(&mut self, protected: &SessionBaselineKey) {
        while self.entries.len() > MAX_SESSION_BASELINES {
            let Some(oldest) = self
                .entries
                .iter()
                .filter(|(key, _)| *key != protected)
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            else {
                return;
            };
            let evicted = self.entries.remove(&oldest);
            eprintln!(
                "[pony-agent][hotloader] session baseline evicted: workspace={} session={} reason=lru_capacity cap={MAX_SESSION_BASELINES} last_used={}",
                oldest.0,
                oldest.1,
                evicted.map(|entry| entry.last_used).unwrap_or_default()
            );
        }
    }
}

/// 技能缓存可驻留的 workspace 桶上限（wave-58 L3 L-1）。
///
/// 长驻桌面进程每打开一个工作区就会驻留一份技能缓存（技能多时每份数百 KB），
/// 因此分桶表必须有界：超过上限按 LRU 淘汰最久未用工作区的桶，并同步清理其逐文件缓存。
pub const MAX_SKILL_CACHE_WORKSPACES: usize = 16;

/// 技能缓存分桶表（wave-58 L3 L-1）：`canonical workspace root -> (合并技能清单, 最近使用序号)`。
///
/// LRU 写法与 [`SessionBaselineRegistry`] 一致（单调 `sequence` + `last_used` 最小者先淘汰）。
#[derive(Debug, Default)]
struct SkillCacheRegistry {
    entries: BTreeMap<PathBuf, SkillCacheEntry>,
    /// 单调递增访问序号（LRU 淘汰依据）。
    sequence: u64,
}

#[derive(Debug, Clone)]
struct SkillCacheEntry {
    skills: Vec<HotloadedSkill>,
    /// 最近一次访问（读或写）时的访问序号。
    last_used: u64,
}

impl SkillCacheRegistry {
    /// 触发一次访问并返回当前序号（饱和递增，永不 panic）。
    fn tick(&mut self) -> u64 {
        self.sequence = self.sequence.saturating_add(1);
        self.sequence
    }

    /// 读取桶内容并刷新 LRU 活跃度；未命中返回 `None`。
    fn touch(&mut self, workspace: &Path) -> Option<Vec<HotloadedSkill>> {
        let now = self.tick();
        let entry = self.entries.get_mut(workspace)?;
        entry.last_used = now;
        Some(entry.skills.clone())
    }

    /// 写入（或更新）桶并做 LRU 淘汰，返回被淘汰的 workspace 列表（供调用方清理逐文件缓存）。
    fn store(&mut self, workspace: PathBuf, skills: Vec<HotloadedSkill>) -> Vec<PathBuf> {
        let now = self.tick();
        self.entries.insert(
            workspace.clone(),
            SkillCacheEntry {
                skills,
                last_used: now,
            },
        );
        let mut evicted: Vec<PathBuf> = Vec::new();
        while self.entries.len() > MAX_SKILL_CACHE_WORKSPACES {
            let Some(oldest) = self
                .entries
                .iter()
                .filter(|(candidate, _)| **candidate != workspace)
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(candidate, _)| candidate.clone())
            else {
                break;
            };
            self.entries.remove(&oldest);
            evicted.push(oldest);
        }
        evicted
    }
}

/// 统一热加载与 KV-Cache 保护管理器
#[derive(Debug, Default)]
pub struct WorkspaceHotloader {
    constitution: RwLock<Option<ConstitutionCache>>,
    /// 逐文件 Skill 缓存，键为 `{canonical_workspace_root}:{scope}:{rank}:{folder}`。
    ///
    /// Wave-58 BLOCKING-1：进程级单例跨工作区复用，键必须含 workspace 身份，且命中判定必须
    /// 比对 `fingerprint.path`（同尺寸 / 同 mtime 的不同文件不得互相冒充）。
    skill_files: RwLock<BTreeMap<String, HotloadedSkill>>,
    /// 上次扫描的合并结果，按 canonical workspace root 分桶（禁止跨工作区共享），
    /// 并受 [`MAX_SKILL_CACHE_WORKSPACES`] 有界 LRU 约束（wave-58 L3 L-1）。
    skills: RwLock<SkillCacheRegistry>,
    /// 会话基线注册表（Wave-58 F2-7：有界 LRU，容量 [`MAX_SESSION_BASELINES`]）。
    session_baselines: RwLock<SessionBaselineRegistry>,
}

static GLOBAL_HOTLOADER: std::sync::OnceLock<WorkspaceHotloader> = std::sync::OnceLock::new();

/// 获取进程级全局持久化热加载管理器，确保跨轮次持有基线
pub fn get_global_workspace_hotloader() -> &'static WorkspaceHotloader {
    GLOBAL_HOTLOADER.get_or_init(WorkspaceHotloader::new)
}

/// 纯逻辑：解析用户级（全局）根目录候选，按优先级从低到高返回。
///
/// 顺序（后者优先级更高，项目级永远在其之上）：
/// 1. `~/.agents`：跨 agent 共享根（与项目级 `.agents` 约定对称）
/// 2. `~/.claude`：Claude Code 兼容根（与项目级 `.claude` 约定对称）
/// 3. `config_dir()/pony-agent`：产品自有全局根
/// 4. `$PONY_AGENT_HOME`：显式覆盖（最高）
///
/// 结果去重且保留优先级顺序；任何输入缺失只跳过该来源，不产生兜底目录。
pub fn resolve_global_agent_homes_from(
    env_home: Option<&Path>,
    config_dir: Option<&Path>,
    home_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(home) = home_dir {
        candidates.push(home.join(".agents"));
        candidates.push(home.join(".claude"));
    }
    if let Some(config) = config_dir {
        candidates.push(config.join("pony-agent"));
    }
    if let Some(env) = env_home {
        candidates.push(env.to_path_buf());
    }

    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut resolved: Vec<PathBuf> = Vec::new();
    for candidate in candidates {
        if candidate.as_os_str().is_empty() {
            continue;
        }
        if seen.insert(candidate.clone()) {
            resolved.push(candidate);
        }
    }
    resolved
}

/// 解析当前进程的用户级（全局）根目录候选（读环境变量与系统目录）。
pub fn resolve_global_agent_homes() -> Vec<PathBuf> {
    let env_home = std::env::var_os(PONY_AGENT_HOME_ENV)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty());
    resolve_global_agent_homes_from(
        env_home.as_deref(),
        dirs::config_dir().as_deref(),
        dirs::home_dir().as_deref(),
    )
}

/// 解析 [`PONY_AGENT_SKILL_DIRS_ENV`] 的纯逻辑入口（不读环境，便于用例注入）。
///
/// - 空项跳过、重复项按首次出现去重，保持用户书写顺序；
/// - 不做 `~` 展开、不做存在性检查（不存在的根在枚举期自然缺席）。
pub fn resolve_custom_skill_dirs_from(raw: Option<&std::ffi::OsStr>) -> Vec<PathBuf> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = Vec::new();
    for path in std::env::split_paths(raw) {
        if path.as_os_str().is_empty() {
            continue;
        }
        if dirs.iter().any(|existing| existing == &path) {
            continue;
        }
        dirs.push(path);
    }
    dirs
}

/// 解析自定义技能根的纯逻辑入口（合并 settings.json 的 skillDirs 与环境变量）。
///
/// 顺序语义（ISSUE-20261111-009）：
/// - `store_dirs`（来自 settings.json 的 `skillDirs` 配置项）按声明顺序排前；
/// - `env_raw`（来自环境变量 `PONY_AGENT_SKILL_DIRS`）按声明顺序兜底追加；
/// - 空项跳过、按首次出现去重，保持用户书写顺序；
/// - 不做 `~` 展开、不做存在性检查（不存在的根在枚举期自然缺席）。
pub fn resolve_custom_skill_dirs_merged(
    store_dirs: &[PathBuf],
    env_raw: Option<&std::ffi::OsStr>,
) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for path in store_dirs {
        if path.as_os_str().is_empty() {
            continue;
        }
        if dirs.iter().any(|existing| existing == path) {
            continue;
        }
        dirs.push(path.clone());
    }
    for path in resolve_custom_skill_dirs_from(env_raw) {
        if dirs.iter().any(|existing| existing == &path) {
            continue;
        }
        dirs.push(path);
    }
    dirs
}

/// 解析当前进程的自定义技能根（合并 settings.json 的 `skillDirs` 与 [`PONY_AGENT_SKILL_DIRS_ENV`]）。
pub fn resolve_custom_skill_dirs() -> Vec<PathBuf> {
    let store_dirs = crate::agent::app_settings::AppSettingsStore::new()
        .load_view()
        .skill_dirs;
    let env_raw = std::env::var_os(PONY_AGENT_SKILL_DIRS_ENV);
    resolve_custom_skill_dirs_merged(&store_dirs, env_raw.as_deref())
}

/// 返回「当前会参与注入」的指令文件 **canonical 路径**清单（wave-58 契约 F3-4）。
///
/// 用途：会话首轮把根层已注入的来源种子化到 `TouchedInstructionMemory.seen`，
/// 使触达层（`instruction_touch`）只注入真正的子目录指令，不再重复注入根层
/// `AGENTS.md`（审查缺陷 P1-3 / 契约 F3-1）。触达层与 semistable 段必须共用本函数
/// 的结果，不得各自计算来源集合（契约第 5 节自洽性检查）。
///
/// 顺序语义：与 [`WorkspaceHotloader::resolve_constitution_with_globals`] 的层级顺序一致
/// ——全局根按 `global_homes` 顺序在前，项目祖先链（浅 → 深）在后；每级目录内按
/// [`INSTRUCTION_CANDIDATES`] 分段语义先基础候选、再 local overlay。
///
/// **空桩（Protocol 冻结，执行者实现业务逻辑）**：只做纯路径计算，不读正文、不入预算；
/// 不可读 / 不可采纳 / 越界符号链接的来源直接不出现（fail-closed）。
pub fn constitution_source_paths(workspace_root: &Path, global_homes: &[PathBuf]) -> Vec<PathBuf> {
    // 与宪法注入共用同一发现函数，保证「会注入什么」与「种子化什么」是同一来源集合，
    // 不各自计算（契约第 5 节自洽性检查）。
    let mut paths: Vec<PathBuf> = Vec::new();
    for source in discover_constitution_sources(workspace_root, global_homes) {
        // fail-closed：不可 canonicalize（悬空符号链接 / 不可读 / 无权限）的来源本就不会被
        // 注入，因此也不进入路径清单。
        let Ok(canonical) = fs::canonicalize(&source.path) else {
            continue;
        };
        // 同一物理文件经不同根到达（符号链接归一）时只记一次。
        if !paths.iter().any(|existing| existing == &canonical) {
            paths.push(canonical);
        }
    }
    paths
}

impl WorkspaceHotloader {
    pub fn new() -> Self {
        Self::default()
    }

    /// 热重载/缓存命中感知：加载工作区宪法文档（遵循 AGENTS.md / CLAUDE.md 标准）
    /// 返回值：(内容, 是否命中全局文件内存缓存)
    pub fn resolve_constitution<P: AsRef<Path>>(
        &self,
        workspace_root: P,
    ) -> Option<(String, bool)> {
        let global_homes = resolve_global_agent_homes();
        self.resolve_constitution_with_globals(workspace_root, &global_homes)
    }

    /// 可注入全局根目录的宪法解析变体（测试与宿主显式装配用）。
    ///
    /// 层级顺序：全局层（按 `global_homes` 顺序）在前，项目层在最后（最高优先级）。
    /// 每层在其根目录内取首个存在的候选文件（`AGENTS.md` → `CLAUDE.md` → `.claude/CLAUDE.md`）。
    /// 仅存在单一项目层时返回原始字节，保持既有提示词形态不变。
    pub fn resolve_constitution_with_globals<P: AsRef<Path>>(
        &self,
        workspace_root: P,
        global_homes: &[PathBuf],
    ) -> Option<(String, bool)> {
        self.resolve_constitution_with_budget(
            workspace_root,
            global_homes,
            InstructionBudget::default(),
        )
    }

    /// 加载（并缓存）给定来源清单的宪法层。
    /// 返回 (来源读取快照, 有界层, 是否命中内存缓存)。
    ///
    /// 缓存以「来源清单 + 单文件读取上限 + **去重前每个来源**的读取状态」为命中条件；
    /// 锁纪律：读锁内只克隆快照，stat / 读盘一律在锁外执行。
    fn load_constitution_layers(
        &self,
        sources: &[ConstitutionSource],
        max_source_bytes: usize,
    ) -> (
        Vec<ConstitutionSourceSnapshot>,
        Vec<ConstitutionLayer>,
        bool,
    ) {
        let cached_snapshot = {
            let guard = self
                .constitution
                .read()
                .unwrap_or_else(|poison| poison.into_inner());
            guard
                .as_ref()
                .filter(|cached| {
                    cached.max_source_bytes == max_source_bytes
                        && cached.sources.len() == sources.len()
                        && cached
                            .sources
                            .iter()
                            .zip(sources.iter())
                            .all(|(cached, source)| {
                                cached.scope == source.scope && cached.path == source.path
                            })
                })
                .cloned()
        };

        if let Some(cached) = cached_snapshot {
            // 任一来源上次读取失败 → 强制 miss 重试（权限恢复时 mtime/size 可能不变）。
            let unchanged = cached
                .sources
                .iter()
                .all(|source| source.read_ok && source.metadata_matches());
            if unchanged {
                return (cached.sources, cached.layers, true);
            }
        }

        let snapshots: Vec<ConstitutionSourceSnapshot> = sources
            .iter()
            .map(|source| {
                ConstitutionSourceSnapshot::capture(source.scope, &source.path, max_source_bytes)
            })
            .collect();
        let layers: Vec<ConstitutionLayer> = snapshots
            .iter()
            .filter(|snapshot| snapshot.read_ok)
            .map(|snapshot| snapshot.to_layer())
            .collect();
        if layers.is_empty() {
            return (snapshots, layers, false);
        }

        let cache = ConstitutionCache {
            layers: layers.clone(),
            sources: snapshots.clone(),
            max_source_bytes,
        };
        *self
            .constitution
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(cache);
        (snapshots, layers, false)
    }

    /// 热重载/缓存命中感知：多层级扫描并解析 Skills
    /// （用户级全局根 + 项目内 `.agents/skills`、`.claude/skills`）
    /// 返回值：(Skills 列表有序且稳定输出, 是否完全命中缓存未变动)
    pub fn scan_skills<P: AsRef<Path>>(&self, workspace_root: P) -> (Vec<HotloadedSkill>, bool) {
        let global_homes = resolve_global_agent_homes();
        self.scan_skills_with_globals(workspace_root, &global_homes)
    }

    /// 可注入全局根目录的 Skills 扫描变体。
    ///
    /// **签名与语义保持不变**（wave-58 F2-6）：仍返回 `(技能清单, 是否完全命中缓存未变动)`，
    /// 内部委托 [`Self::scan_skills_internal`]。扫描与加载共用同一信任谓词（D1），
    /// 去重键为 frontmatter `name`（D3）；被跳过的候选只体现在报告的 `omitted` 里，
    /// 不改变本方法的返回值形状。
    pub fn scan_skills_with_globals<P: AsRef<Path>>(
        &self,
        workspace_root: P,
        global_homes: &[PathBuf],
    ) -> (Vec<HotloadedSkill>, bool) {
        let custom_skill_dirs = resolve_custom_skill_dirs();
        self.scan_skills_with_roots(workspace_root, global_homes, &custom_skill_dirs)
    }

    /// 可注入「全局根 + 自定义技能根」的 Skills 扫描变体（PA-173 / ISSUE-20261111-007）。
    ///
    /// 自定义根来自 [`PONY_AGENT_SKILL_DIRS_ENV`]；显式传参入口让用例不依赖进程环境，
    /// 语义与 [`Self::scan_skills_with_globals`] 完全一致。
    pub fn scan_skills_with_roots<P: AsRef<Path>>(
        &self,
        workspace_root: P,
        global_homes: &[PathBuf],
        custom_skill_dirs: &[PathBuf],
    ) -> (Vec<HotloadedSkill>, bool) {
        let (report, hit) =
            self.scan_skills_internal(workspace_root.as_ref(), global_homes, custom_skill_dirs);
        (report.skills, hit)
    }

    /// 技能扫描（带省略诊断），全局根按进程环境解析（wave-58 契约 F2-6 / D2）。
    ///
    /// 语义：`skills` 只包含**可加载**的技能（扫描与加载共用同一信任谓词，见 D1），
    /// 被跳过的候选以 [`SkillOmission`] 逐条记录，供 `omitted skill:<name> (<reason>)` 标记。
    pub fn scan_skills_report<P: AsRef<Path>>(&self, workspace_root: P) -> SkillScanReport {
        let global_homes = resolve_global_agent_homes();
        self.scan_skills_report_with_globals(workspace_root, &global_homes)
    }

    /// 技能扫描（带省略诊断），可注入全局根（wave-58 契约 F2-6 / D2）。
    ///
    /// 与 [`Self::scan_skills_with_globals`] 同源（都走 [`Self::scan_skills_internal`]），
    /// 区别只在本方法返回被跳过候选的诊断明细。
    pub fn scan_skills_report_with_globals<P: AsRef<Path>>(
        &self,
        workspace_root: P,
        global_homes: &[PathBuf],
    ) -> SkillScanReport {
        let custom_skill_dirs = resolve_custom_skill_dirs();
        self.scan_skills_report_with_roots(workspace_root, global_homes, &custom_skill_dirs)
    }

    /// 可注入「全局根 + 自定义技能根」的带诊断扫描变体（PA-173）。
    pub fn scan_skills_report_with_roots<P: AsRef<Path>>(
        &self,
        workspace_root: P,
        global_homes: &[PathBuf],
        custom_skill_dirs: &[PathBuf],
    ) -> SkillScanReport {
        self.scan_skills_internal(workspace_root.as_ref(), global_homes, custom_skill_dirs)
            .0
    }

    /// 技能扫描的**唯一实现**（wave-58 F2-3 / F2-4 / F2-5 / F2-6）。
    ///
    /// 契约落点：
    /// 1. **D1 分域信任**：每个候选的 `SKILL.md` 都经
    ///    [`crate::agent::skill_body::trust_skill_file`]（`is_trusted_skill_file` 的内部实现）判定，
    ///    与 `skill_load` 加载路径调用同一个谓词 —— 消除「清单里有、加载 404」（P1-6）。
    ///    全局根允许符号链接逃逸；项目根严格 canonical 包含，且根自身不得逃出 workspace。
    /// 2. **D2 可观测**：被跳过的候选逐条进入 `omitted`（outside_root / too_large /
    ///    duplicate_name / not_a_file），不再静默消失（P2-1 / P2-8）。
    /// 3. **D3 去重**：键 = frontmatter `name`（缺省回落目录名），同名先比 `rank`（小者胜），
    ///    同 rank 比 canonical 路径字典序（确定性，不依赖遍历顺序），败者记 `duplicate_name`。
    /// 4. **确定性**：目录项按名字排序后遍历；`omitted` 按 (name, reason, path) 排序去重。
    /// 5. **P2-3**：同一物理技能根经不同路径到达（`<ws>/.claude/skills -> .agents/skills`）
    ///    只扫描一次（canonical 根去重），避免重复 IO。
    fn scan_skills_internal(
        &self,
        workspace_root: &Path,
        global_homes: &[PathBuf],
        custom_skill_dirs: &[PathBuf],
    ) -> (SkillScanReport, bool) {
        let canonical_workspace = workspace_root.canonicalize().ok();
        // Wave-58 BLOCKING-1：workspace 身份标签（canonical 优先），用于缓存键与合并结果分桶。
        let workspace_label = canonical_workspace
            .clone()
            .unwrap_or_else(|| workspace_root.to_path_buf())
            .display()
            .to_string();
        let mut file_cache: BTreeMap<String, HotloadedSkill> = BTreeMap::new();
        let mut any_changed = false;
        let mut omitted: Vec<SkillOmission> = Vec::new();
        // (技能, canonical 的 SKILL.md 路径)：canonical 路径只用于 D3 同 rank tie-break。
        let mut candidates: Vec<(HotloadedSkill, PathBuf)> = Vec::new();

        // —— 阶段 1：技能根筛选（分域信任 + canonical 去重）——
        let mut scan_roots: Vec<(AgentScope, PathBuf, u16, bool, bool)> = Vec::new();
        let mut seen_canonical_roots: HashSet<PathBuf> = HashSet::new();
        for spec in enumerate_skill_roots(workspace_root, global_homes, custom_skill_dirs) {
            let SkillRootSpec {
                scope,
                dir: base_dir,
                rank,
                containment_base,
            } = spec;
            // D1：全局根（用户自有域）允许符号链接解析到根外；项目根保持严格包含。
            let allow_symlink_escape = scope == AgentScope::Global;
            let canonical_root = base_dir.canonicalize().ok();
            let root_trusted = if allow_symlink_escape {
                true
            } else {
                // 项目根自身不得经符号链接逃出**所属项目域**：仓库根技能以仓库根为上界，
                // 会话根技能以会话根为上界（workspace 内容是不可信输入）。
                let base = containment_base
                    .as_deref()
                    .unwrap_or(workspace_root)
                    .canonicalize()
                    .ok();
                match (base, canonical_root.as_ref()) {
                    (Some(base), Some(canonical_root)) => canonical_root.starts_with(&base),
                    _ => false,
                }
            };
            // P2-3：同一物理根只扫描一次（保留先出现的 rank，即更高优先级那个）。
            if let Some(canonical_root) = canonical_root {
                if !seen_canonical_roots.insert(canonical_root) {
                    continue;
                }
            }
            scan_roots.push((scope, base_dir, rank, allow_symlink_escape, root_trusted));
        }

        // —— 阶段 2：逐候选判定 + 解析 ——
        for (scope, base_dir, rank, allow_symlink_escape, root_trusted) in scan_roots {
            let Ok(read_dir) = fs::read_dir(&base_dir) else {
                continue;
            };
            let mut entries: Vec<fs::DirEntry> = read_dir.flatten().collect();
            // 确定性：不依赖 read_dir 顺序。
            entries.sort_by_key(|entry| entry.file_name());

            for entry in entries {
                let path = entry.path();
                let folder_name = entry.file_name().to_string_lossy().to_string();
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                // 技能候选 = 目录 / 指向目录的符号链接；普通文件条目不是技能，静默忽略。
                let is_dir_candidate = if file_type.is_dir() {
                    true
                } else if file_type.is_symlink() {
                    match fs::metadata(&path) {
                        Ok(metadata) if metadata.is_dir() => true,
                        Ok(_) => false,
                        Err(_) => {
                            // 悬空目录符号链接（P2-8）：可观测标记，不得静默消失。
                            omitted.push(SkillOmission {
                                name: folder_name,
                                reason: SkillOmissionReason::NotAFile,
                                path,
                            });
                            continue;
                        }
                    }
                } else {
                    false
                };
                if !is_dir_candidate {
                    continue;
                }
                let skill_file = path.join("SKILL.md");
                // 目录内没有 SKILL.md：不是技能目录（既有语义：静默忽略，不产生噪音）。
                if fs::symlink_metadata(&skill_file).is_err() {
                    continue;
                }

                // D1 / F2-3：扫描与加载共用的同一信任谓词。
                let trust = if root_trusted {
                    trust_skill_file(&skill_file, &base_dir, allow_symlink_escape)
                } else {
                    Err(SkillTrustRejection::OutsideRoot)
                };
                let canonical = match trust {
                    Ok(trusted) => trusted.canonical,
                    Err(rejection) => {
                        let reason = match rejection {
                            SkillTrustRejection::OutsideRoot => SkillOmissionReason::OutsideRoot,
                            SkillTrustRejection::TooLarge => SkillOmissionReason::TooLarge,
                            SkillTrustRejection::NotAFile => SkillOmissionReason::NotAFile,
                        };
                        // F2-5：超限技能仍可有界读取 frontmatter 取真实 name（D3 去重键语义一致）；
                        // 其余拒绝原因（越界 / 非文件）禁止读取目标内容，name 回落目录名。
                        let name = if reason == SkillOmissionReason::TooLarge {
                            skill_name_from_frontmatter_prefix(&skill_file).unwrap_or(folder_name)
                        } else {
                            folder_name
                        };
                        omitted.push(SkillOmission {
                            name,
                            reason,
                            path: skill_file,
                        });
                        continue;
                    }
                };

                let metadata = match fs::metadata(&skill_file) {
                    Ok(m) => m,
                    Err(_) => {
                        omitted.push(SkillOmission {
                            name: folder_name,
                            reason: SkillOmissionReason::NotAFile,
                            path: skill_file,
                        });
                        continue;
                    }
                };
                let mtime = metadata.modified().ok();
                let file_size = metadata.len();
                // Wave-58 BLOCKING-1：键必须含 workspace 身份 —— 进程级单例跨工作区复用，
                // 只带 `scope:rank:folder` 会让 A 工作区的同名技能冒充 B 工作区的技能。
                let cache_key = format!(
                    "{}:{}:{}:{}",
                    workspace_label,
                    scope.as_str(),
                    rank,
                    folder_name
                );

                let cached = {
                    let read_guard = self
                        .skill_files
                        .read()
                        .unwrap_or_else(|poison| poison.into_inner());
                    read_guard.get(&cache_key).cloned()
                };

                let skill = match cached {
                    Some(existing)
                        if existing.scope == scope
                            && existing.rank == rank
                            // BLOCKING-1：同尺寸 + 同 mtime 的不同文件不得互相冒充。
                            && existing.fingerprint.path == skill_file
                            && existing.path == skill_file
                            // 修复 B071: 增加 mtime 比对的容差（±1s），防止文件系统精度抖动导致缓存失效
                            && existing.fingerprint.mtime.map(|t| mtime.map(|mt| t.duration_since(mt).map(|d| d.as_secs() < 1).unwrap_or(false) || mt.duration_since(t).map(|d| d.as_secs() < 1).unwrap_or(false)).unwrap_or(false)).unwrap_or(false)
                            && existing.fingerprint.file_size == file_size =>
                    {
                        existing
                    }
                    _ => {
                        any_changed = true;
                        // B7：只为解析 frontmatter 读取有界前缀。
                        match FileFingerprint::compute_bounded(
                            &skill_file,
                            SKILL_FRONTMATTER_READ_LIMIT,
                        ) {
                            Some((fingerprint, content)) => {
                                let (
                                    parsed_name,
                                    description,
                                    user_invocable,
                                    allowed_tools,
                                    disable_model_invocation,
                                    model_invocable,
                                ) = parse_skill_metadata(&content, &folder_name);
                                HotloadedSkill {
                                    name: parsed_name,
                                    description,
                                    user_invocable,
                                    allowed_tools,
                                    path: skill_file.clone(),
                                    fingerprint,
                                    scope,
                                    rank,
                                    disable_model_invocation,
                                    model_invocable,
                                }
                            }
                            None => {
                                omitted.push(SkillOmission {
                                    name: folder_name,
                                    reason: SkillOmissionReason::NotAFile,
                                    path: skill_file,
                                });
                                continue;
                            }
                        }
                    }
                };

                file_cache.insert(cache_key, skill.clone());
                candidates.push((skill, canonical));
            }
        }

        // —— 阶段 3：D3 去重（键 = frontmatter name，缺省目录名）——
        let mut winners: BTreeMap<String, (HotloadedSkill, PathBuf)> = BTreeMap::new();
        for (skill, canonical) in candidates {
            match winners.entry(skill.name.clone()) {
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let challenger_wins = {
                        let (existing, existing_canonical) = entry.get();
                        skill.rank < existing.rank
                            || (skill.rank == existing.rank && canonical < *existing_canonical)
                    };
                    if challenger_wins {
                        let (loser, _) = entry.insert((skill, canonical));
                        omitted.push(SkillOmission {
                            name: loser.name,
                            reason: SkillOmissionReason::DuplicateName,
                            path: loser.path,
                        });
                    } else {
                        omitted.push(SkillOmission {
                            name: skill.name,
                            reason: SkillOmissionReason::DuplicateName,
                            path: skill.path,
                        });
                    }
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert((skill, canonical));
                }
            }
        }
        let merged: Vec<HotloadedSkill> = winners
            .into_values()
            .map(|(skill, _canonical)| skill)
            .collect();

        // 诊断项确定性排序并去掉完全重复项（同一 root 重复到达时不产生双份噪音）。
        omitted.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then(left.reason.cmp(&right.reason))
                .then(left.path.cmp(&right.path))
        });
        omitted.dedup();

        let report = SkillScanReport {
            skills: merged,
            omitted,
        };
        // 合并结果按 canonical workspace root 分桶（BLOCKING-1：单例不得跨工作区共享）。
        let workspace_bucket = canonical_workspace
            .clone()
            .unwrap_or_else(|| workspace_root.to_path_buf());
        {
            let mut guard = self
                .skills
                .write()
                .unwrap_or_else(|poison| poison.into_inner());
            if !any_changed && guard.touch(&workspace_bucket).as_ref() == Some(&report.skills) {
                return (report, true);
            }
        }

        // 写入分桶表并做 LRU 淘汰（wave-58 L3 L-1）：被淘汰工作区等同「首次访问」。
        let evicted_workspaces = self
            .skills
            .write()
            .unwrap_or_else(|poison| poison.into_inner())
            .store(workspace_bucket, report.skills.clone());

        {
            let mut guard = self
                .skill_files
                .write()
                .unwrap_or_else(|poison| poison.into_inner());
            // 只替换本工作区的缓存切片，其他工作区的条目保持不变。
            let prefix = format!("{workspace_label}:");
            guard.retain(|key, _| !key.starts_with(&prefix));
            // 被淘汰工作区的逐文件缓存同步清除，避免内存泄漏。
            for evicted in &evicted_workspaces {
                let evicted_prefix = format!("{}:", evicted.display());
                guard.retain(|key, _| !key.starts_with(&evicted_prefix));
                eprintln!(
                    "[pony-agent][hotloader] skill cache evicted: workspace={} reason=lru_capacity cap={MAX_SKILL_CACHE_WORKSPACES}",
                    evicted.display()
                );
            }
            for (key, value) in file_cache {
                guard.insert(key, value);
            }
        }
        (report, false)
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
        let global_homes = resolve_global_agent_homes();
        self.resolve_session_prompt_context_with_globals(
            session_id,
            turn_count,
            workspace_root,
            &global_homes,
        )
    }

    /// 可注入全局根目录的会话前缀变体（测试与宿主显式装配用）。
    pub fn resolve_session_prompt_context_with_globals<P: AsRef<Path>>(
        &self,
        session_id: &str,
        turn_count: usize,
        workspace_root: P,
        global_homes: &[PathBuf],
    ) -> (String, Option<String>) {
        self.resolve_session_prompt_context_with_budget(
            session_id,
            turn_count,
            workspace_root,
            global_homes,
            InstructionBudget::default(),
        )
    }

    /// 可显式指定注入预算的会话前缀变体。
    ///
    /// 静态前缀一旦固化即永不重写（KV-Cache 头保护）；预算变化与宪法/技能变化一样，
    /// 只以 Append-Only notice 表达。
    pub fn resolve_session_prompt_context_with_budget<P: AsRef<Path>>(
        &self,
        session_id: &str,
        turn_count: usize,
        workspace_root: P,
        global_homes: &[PathBuf],
        budget: InstructionBudget,
    ) -> (String, Option<String>) {
        self.resolve_session_prompt_context_with_budget_and_catalog(
            session_id,
            turn_count,
            workspace_root,
            global_homes,
            budget,
            SkillCatalogBudget::unlimited(),
        )
    }

    /// 会话前缀变体：技能清单同时受 [`SkillCatalogBudget`] 约束（wave-58 D5'）。
    ///
    /// 供 `context.rs` 按 provider 上下文窗口计算清单预算后调用；`catalog_budget` 为
    /// [`SkillCatalogBudget::unlimited`] 时与既有 [`Self::resolve_session_prompt_context_with_budget`]
    /// 字节完全一致。超预算条目从尾部丢弃并输出 `omitted skill:<name> (budget)` 标记。
    ///
    /// 注意：为保证首轮固化的前缀可复现，调用方应传入**会话内稳定**的预算
    /// （同一 provider 窗口）；中途变化只影响下一次渲染，不会被记为宪法/技能变更。
    pub fn resolve_session_prompt_context_with_budget_and_catalog<P: AsRef<Path>>(
        &self,
        session_id: &str,
        turn_count: usize,
        workspace_root: P,
        global_homes: &[PathBuf],
        budget: InstructionBudget,
        catalog_budget: SkillCatalogBudget,
    ) -> (String, Option<String>) {
        let workspace_root = workspace_root.as_ref();
        let (current_constitution, _) = self
            .resolve_constitution_with_budget(workspace_root, global_homes, budget)
            .unwrap_or_else(|| (String::new(), true));
        // Wave-58 D2 / L3-B4：走报告通道，omitted 诊断在生产路径不再被丢弃。
        let (report, _) =
            self.scan_skills_internal(workspace_root, global_homes, &resolve_custom_skill_dirs());
        let current_skills = apply_skill_injection_budget(report.skills, budget);

        let const_hash = fnv1a_hash(current_constitution.as_bytes());
        let skills_hash = skill_set_hash(&current_skills);
        // Wave-58 L3-B F-2：基线键 = (canonical workspace root, session_id)，同 session 换绑工作区
        // 不会回放旧工作区前缀（跨工作区不得串味）。
        let baseline_key: SessionBaselineKey = (
            workspace_root
                .canonicalize()
                .unwrap_or_else(|_| workspace_root.to_path_buf())
                .display()
                .to_string(),
            session_id.to_string(),
        );

        // 快路径：锁内只取快照（渲染一律在锁外，L3-A4）。
        let replayed = if turn_count <= 1 {
            None
        } else {
            let mut baselines = self
                .session_baselines
                .write()
                .unwrap_or_else(|poison| poison.into_inner());
            baselines.touch_entry(&baseline_key)
        };

        if let Some(entry) = replayed {
            // 非首轮：默认回放已固化的前缀字节（KV-Cache 头保护）。
            let baseline = entry.snapshot;
            let const_changed = baseline.constitution_hash != const_hash;
            let skills_changed = baseline.skills_hash != skills_hash;
            let budget_changed = baseline.budget != budget;
            // L3 裁决 A：清单预算不一致（如 provider 窗口切换）时不能用旧清单字节，
            // 必须用**基线内容**按**当前** catalog_budget 重渲染；宪法/技能正文仍取基线 ⇒ F1-3 不破。
            // 渲染结果不回写基线（baseline 只存首轮事实，避免预算抖动引发反复重渲染）。
            let catalog_changed = entry.catalog_budget != catalog_budget;
            let replay_prefix = if catalog_changed {
                // F-3：omitted 取首轮存档（同一份诊断标记），而不是写死为空。
                self.render_prompt_prefix_with_budget(
                    Some(&baseline.constitution_content),
                    &baseline.skills,
                    catalog_budget,
                    &entry.omitted,
                )
            } else {
                entry.prefix
            };
            if !const_changed && !skills_changed && !budget_changed && !catalog_changed {
                return (replay_prefix, None);
            }
            let delta_notice = build_session_delta_notice(
                &baseline,
                &current_skills.iter().filter(|s| s.model_invocable).cloned().collect::<Vec<_>>(),
                const_changed,
                skills_changed,
                budget_changed,
                catalog_changed,
            );
            return (replay_prefix, Some(delta_notice));
        }

        // 需要（重新）固化基线：渲染在锁外完成，避免把全量渲染压进全局写锁。
        let static_prefix = self.render_prompt_prefix_with_budget(
            Some(&current_constitution),
            &current_skills,
            catalog_budget,
            &report.omitted,
        );
        // L3 BLOCKING-2：中途未命中基线（LRU 淘汰 / 进程重启）必须可观测，不得静默改写前缀。
        //
        // wave-interrupted-turn-resume C6：可观测性**只走结构化日志**，不注入模型可见请求体。
        // 原实现额外把 `Notice: Session baseline was not retained …` 塞进 volatile 段，而"基线
        // 是否还在进程内缓存"是**进程本地状态**：同一份已落盘会话在同一进程里恢复两次时，
        // 第一次命中本分支（无基线）、第二次命中回放分支（有基线）⇒ 两次恢复的请求体不逐字
        // 相同（C6 判据）。请求体只能由会话事实决定，进程缓存状态一律不外泄到模型侧。
        if turn_count > 1 {
            eprintln!(
                "[pony-agent][hotloader] session baseline missing: session={session_id} turn={turn_count} reason=not_retained (lru_capacity={MAX_SESSION_BASELINES} or process restart); prefix re-baselined"
            );
        }
        let notice: Option<String> = None;

        let baseline = SessionBaselineSnapshot {
            constitution_hash: const_hash,
            constitution_content: current_constitution,
            skills_hash,
            skills: current_skills,
            budget,
        };
        {
            let mut baselines = self
                .session_baselines
                .write()
                .unwrap_or_else(|poison| poison.into_inner());
            baselines.insert_entry(
                baseline_key,
                baseline,
                catalog_budget,
                report.omitted.clone(),
                static_prefix.clone(),
            );
        }
        (static_prefix, notice)
    }

    /// 构造静态保真的 Prompt 前缀块，确保 Prompt Cache 最佳命中。
    ///
    /// **既有语义不变**（wave-58 D5'）：不裁剪技能清单；需要按窗口预算裁剪请调用
    /// [`Self::build_cached_prompt_prefix_with_budget`]。
    pub fn build_cached_prompt_prefix(
        &self,
        constitution_content: Option<&str>,
        skills: &[HotloadedSkill],
    ) -> String {
        self.build_cached_prompt_prefix_with_budget(
            constitution_content,
            skills,
            SkillCatalogBudget::unlimited(),
        )
    }

    /// 构造静态 Prompt 前缀块，技能清单受 [`SkillCatalogBudget`] 约束（wave-58 D5'）。
    ///
    /// 裁剪语义：
    /// - 裁剪前先按 `(rank 升序, 项目层优先, name 字典序)` 定优先级，**从低优先级**丢弃；
    /// - 被丢弃的条目最多输出 8 条 `omitted skill:<name> (budget)` 标记，其余折叠为
    ///   一行 `omitted skills: N more (catalog budget)`；
    /// - 保留条目仍按传入顺序渲染（前缀不因裁剪改变顺序）；
    /// - 宪法段不受目录预算影响（照旧完整渲染）；
    /// - 预算连标题都放不下时整段清单不渲染（fail-safe，不输出半截标题）。
    pub fn build_cached_prompt_prefix_with_budget(
        &self,
        constitution_content: Option<&str>,
        skills: &[HotloadedSkill],
        catalog_budget: SkillCatalogBudget,
    ) -> String {
        self.render_prompt_prefix_with_budget(constitution_content, skills, catalog_budget, &[])
    }

    /// 构造静态 Prompt 前缀块，技能清单受 [`SkillCatalogBudget`] 约束，并透传外部 omitted 标记。
    pub fn build_cached_prompt_prefix_with_omissions(
        &self,
        constitution_content: Option<&str>,
        skills: &[HotloadedSkill],
        catalog_budget: SkillCatalogBudget,
        omitted: &[SkillOmission],
    ) -> String {
        self.render_prompt_prefix_with_budget(constitution_content, skills, catalog_budget, omitted)
    }

    /// 前缀渲染的唯一实现（wave-58 D5' + D2 / L3-B4）：技能清单预算裁剪 + omitted 标记。
    ///
    /// `omitted` 为扫描阶段被跳过的候选（[`SkillOmission`]），其标记与预算裁剪标记共用
    /// ≤8 条具名上限与折叠行，一并计入清单段预算。
    fn render_prompt_prefix_with_budget(
        &self,
        constitution_content: Option<&str>,
        skills: &[HotloadedSkill],
        catalog_budget: SkillCatalogBudget,
        omitted: &[SkillOmission],
    ) -> String {
        let mut prefix = String::with_capacity(4096);

        if let Some(const_text) = constitution_content {
            if !const_text.trim().is_empty() {
                prefix.push_str("=== WORKSPACE CONSTITUTION & RULES ===\n");
                prefix.push_str(const_text.trim());
                prefix.push_str("\n\n");
            }
        }

        // 分离 model_invocable 与不可供模型调用的技能（PLAN-20261111-02 Step 2）
        let mut model_skills: Vec<HotloadedSkill> = Vec::new();
        let mut effective_omitted: Vec<SkillOmission> = omitted.to_vec();
        for skill in skills {
            if skill.model_invocable {
                model_skills.push(skill.clone());
            } else {
                effective_omitted.push(SkillOmission {
                    name: skill.name.clone(),
                    reason: SkillOmissionReason::ModelInvocationDisabled,
                    path: skill.path.clone(),
                });
            }
        }

        if !model_skills.is_empty() || !effective_omitted.is_empty() {
            let header = "=== AVAILABLE REUSABLE SKILLS ===\n\
                          The following project-specific skills are available and loadable on demand:\n";
            let header_bytes = header.len();
            if catalog_budget.max_bytes < header_bytes {
                // 预算连标题都放不下：整段不渲染（宪法照旧），不输出半截清单。
                return prefix;
            }
            // wave-58 F2-1 / F1-2：块标量描述可能含换行，清单必须保持「每个技能一行」，
            // 因此渲染时把描述内的换行压平为空格（`HotloadedSkill.description` 本身保持原文）。
            let lines: Vec<String> = model_skills
                .iter()
                .map(|skill| {
                    // wave-58 L3-B F-1：name / description 均为不可信文本，进入模型可见清单前转义
                    // （换行与控制字符压平、反引号与连续等号剔除），并各自受限长。
                    format!(
                        "- `{}` [{}]: {}\n",
                        manifest_display_name(skill),
                        skill.scope.as_str(),
                        manifest_description(skill)
                    )
                })
                .collect();
            let short_lines: Vec<String> = model_skills
                .iter()
                .map(|skill| {
                    format!(
                        "- `{}` [{}]: (description truncated, load with skill_load)\n",
                        manifest_display_name(skill),
                        skill.scope.as_str()
                    )
                })
                .collect();

            // 1) 裁剪优先级（与 `scan_skills_with_budget` 的 `by_priority` 同口径）：
            //    rank 小者优先 → 同 rank 项目层优先 → name 字典序 → 原返回顺序（确定性）。
            //    注意：`scan_skills_internal` 的返回顺序是 name 字典序，**不是**优先级顺序，
            //    因此绝不能用「从尾部丢弃」代替优先级裁剪，否则字典序靠后的项目技能会被
            //    全局技能挤掉（wave-58 F1-6 大窗口分支）。
            let mut priority: Vec<usize> = (0..model_skills.len()).collect();
            priority.sort_by(|&left, &right| {
                model_skills[left]
                    .rank
                    .cmp(&model_skills[right].rank)
                    .then(model_skills[right].scope.cmp(&model_skills[left].scope))
                    .then(model_skills[left].name.cmp(&model_skills[right].name))
                    .then(left.cmp(&right))
            });

            // 2) 两级保留选取（字节预算 + 条目上限，PLAN-20261111-02 Step 4）：
            //    - 先为条目分配最短行保真底座（只要预算允许，全部技能名均在目录中可见，消除「可载而不知」）
            //    - 再用剩余预算按优先级将最短行升级为完整描述行（受 max_entries 与字节上限约束）
            let max_markers_budget = 1024;
            let available_bytes = catalog_budget
                .max_bytes
                .saturating_sub(header_bytes + max_markers_budget);

            let mut full_flags = vec![false; model_skills.len()];
            let mut short_flags = vec![false; model_skills.len()];
            let mut short_cost = 0usize;

            for &index in &priority {
                let s_len = short_lines[index].len();
                if short_cost + s_len <= available_bytes {
                    short_flags[index] = true;
                    short_cost += s_len;
                }
            }

            let mut used = header_bytes + short_cost;
            let mut full_count = 0usize;
            for &index in &priority {
                if !short_flags[index] || full_count >= catalog_budget.max_entries {
                    continue;
                }
                let upgrade_delta = lines[index].len().saturating_sub(short_lines[index].len());
                if used + upgrade_delta + max_markers_budget <= catalog_budget.max_bytes {
                    used += upgrade_delta;
                    full_flags[index] = true;
                    short_flags[index] = false;
                    full_count += 1;
                }
            }

            // 3) 标记行同样吃预算：放不下时按优先级从低到高继续收缩保留集合（有界收敛）。
            loop {
                // 「被丢弃」= 既没有完整行、也没有最短行（两级渲染都不成立）。
                // 只按 `full_flags` 判定会把**已渲染短行**的技能同时计入
                // `omitted skill:<name> (budget)` 诊断行，导致目录自相矛盾
                // （同一技能既可见又被报告为预算丢弃）。
                let dropped: Vec<usize> = priority
                    .iter()
                    .copied()
                    .filter(|&index| !full_flags[index] && !short_flags[index])
                    .collect();
                let markers = catalog_marker_lines(&effective_omitted, &model_skills, &dropped);
                let markers_bytes: usize = markers.iter().map(String::len).sum();
                if used + markers_bytes <= catalog_budget.max_bytes {
                    // 4) 渲染：保留集合按**原返回顺序**输出（全行或最短行，前缀稳定），
                    //    标记按被丢弃者的同一优先级顺序输出。
                    prefix.push_str(header);
                    for index in 0..model_skills.len() {
                        if full_flags[index] {
                            prefix.push_str(&lines[index]);
                        } else if short_flags[index] {
                            prefix.push_str(&short_lines[index]);
                        }
                    }
                    for marker in &markers {
                        prefix.push_str(marker);
                    }
                    prefix.push_str("\n");
                    return prefix;
                }
                // 超限收缩：先收缩 short_line，再将 full_line 降级为 short_line
                if let Some(weakest_short) = priority.iter().rev().copied().find(|&index| short_flags[index]) {
                    short_flags[weakest_short] = false;
                    used -= short_lines[weakest_short].len();
                } else if let Some(weakest_full) = priority.iter().rev().copied().find(|&index| full_flags[index]) {
                    full_flags[weakest_full] = false;
                    let downgrade_delta = lines[weakest_full].len().saturating_sub(short_lines[weakest_full].len());
                    used -= downgrade_delta;
                    short_flags[weakest_full] = true;
                } else {
                    // 已无条目可丢：仍要尽量输出扫描 omitted 标记（预算允许时）。
                    let fallback = catalog_marker_lines(&effective_omitted, &model_skills, &[]);
                    let fallback_bytes: usize = fallback.iter().map(String::len).sum();
                    if header_bytes + fallback_bytes <= catalog_budget.max_bytes {
                        prefix.push_str(header);
                        for marker in &fallback {
                            prefix.push_str(marker);
                        }
                        prefix.push_str("\n");
                    }
                    return prefix;
                }
            }
        }

        prefix
    }
}

/// 技能清单预算（wave-58 D5'）：字节上限 + 条目上限。
///
/// `unlimited()` 表示不裁剪（既有 [`WorkspaceHotloader::build_cached_prompt_prefix`] 语义）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkillCatalogBudget {
    /// 技能清单段允许占用的最大字节数（含标题与 `omitted` 标记行）。
    pub max_bytes: usize,
    /// 最多列出的技能条数。
    pub max_entries: usize,
}

impl SkillCatalogBudget {
    /// 不裁剪（既有行为）。
    pub const fn unlimited() -> Self {
        Self {
            max_bytes: usize::MAX,
            max_entries: usize::MAX,
        }
    }

    /// 显式预算：字节上限 + 条目上限。
    pub const fn new(max_bytes: usize, max_entries: usize) -> Self {
        Self {
            max_bytes,
            max_entries,
        }
    }
}

impl Default for SkillCatalogBudget {
    fn default() -> Self {
        Self::unlimited()
    }
}

/// 清单 token 转义（wave-58 L3-B F-1，安全）：技能名 / 描述 / `omitted` 标记都是模型可见
/// 文本，必须防止恶意技能伪造条目行、伪造诊断行甚至伪造 `=== ... ===` 段落标题。
///
/// 规则（最小实现）：
/// 1. 换行 / 回车 / 制表 / 其他控制字符压成单空格；
/// 2. 连续空白折叠为一（`flatten_single_line` 已含该语义，这里统一复用）；
/// 3. 删除 `` ` ``（避免提前闭合 `- `name`` 的行格式）；
/// 4. 删除长度 ≥3 的连续等号串（避免伪造段落标题）。
pub(crate) fn sanitize_manifest_token(raw: &str) -> String {
    let flattened = flatten_single_line(raw);
    let mut cleaned = String::with_capacity(flattened.len());
    let mut equal_run = 0usize;
    for character in flattened.chars() {
        if character == '`' {
            continue;
        }
        if character == '=' {
            equal_run += 1;
            if equal_run >= 3 {
                // 三个及以上连续等号：整段丢弃（前面已写入的 1..2 个也回退掉）。
                while cleaned.ends_with('=') {
                    cleaned.pop();
                }
                continue;
            }
        } else {
            equal_run = 0;
        }
        cleaned.push(character);
    }
    cleaned.trim().to_string()
}

/// 清单显示名（wave-58 L3-B F-1）：sanitize 后为空则回落目录名，再为空才用占位名。
fn manifest_display_name(skill: &HotloadedSkill) -> String {
    let sanitized = sanitize_manifest_token(&skill.name);
    if !sanitized.is_empty() {
        return truncate_description(&sanitized, MAX_SKILL_NAME_CHARS);
    }
    let folder = skill
        .path
        .parent()
        .and_then(|directory| directory.file_name())
        .and_then(|folder| folder.to_str())
        .unwrap_or_default();
    let fallback = sanitize_manifest_token(folder);
    if fallback.is_empty() {
        "unnamed-skill".to_string()
    } else {
        truncate_description(&fallback, MAX_SKILL_NAME_CHARS)
    }
}

/// 清单里技能描述的字符上限（收敛至 500 字符，对齐 dsh catalogDescriptionMaxLength=500，PLAN-20261111-02 Step 4）。
pub const MAX_SKILL_CATALOG_DESCRIPTION_CHARS: usize = 500;

/// 清单描述（wave-58 L3-B F-1）：先 sanitize 再按 500 字符截断。
fn manifest_description(skill: &HotloadedSkill) -> String {
    truncate_description(
        &sanitize_manifest_token(&skill.description),
        MAX_SKILL_CATALOG_DESCRIPTION_CHARS,
    )
}

/// 清单里技能名的字符上限（wave-58 L3-B F-1）。
const MAX_SKILL_NAME_CHARS: usize = 64;

/// 清单裁剪标记（wave-58 D5'）：最多 [`MAX_SKILL_CATALOG_OMISSION_MARKERS`] 条具名标记，
/// 其余折叠为一行计数标记。
pub const MAX_SKILL_CATALOG_OMISSION_MARKERS: usize = 8;

/// 生成清单尾部标记行（wave-58 D5' + D2 / L3-B4）。
///
/// 顺序与格式：
/// 1. 扫描阶段被跳过的候选：`omitted skill:<name> (<reason>)`（D2 契约格式，`omitted` 顺序）；
/// 2. 预算裁剪丢弃的条目：`omitted skill:<name> (budget)`（按 `dropped` 优先级顺序）；
/// 3. 具名标记合计 ≤ [`MAX_SKILL_CATALOG_OMISSION_MARKERS`] 条，其余折叠为
///    `omitted skills: N more (catalog budget)`。
fn catalog_marker_lines(
    omitted: &[SkillOmission],
    skills: &[HotloadedSkill],
    dropped: &[usize],
) -> Vec<String> {
    let mut named: Vec<String> = omitted.iter().map(SkillOmission::marker).collect();
    named.extend(
        dropped
            .iter()
            .filter_map(|&index| skills.get(index))
            .map(|skill| format!("omitted skill:{} (budget)", skill.name)),
    );
    if named.is_empty() {
        return Vec::new();
    }
    let total = named.len();
    let mut lines: Vec<String> = named
        .into_iter()
        .take(MAX_SKILL_CATALOG_OMISSION_MARKERS)
        .map(|marker| format!("{marker}\n"))
        .collect();
    let remaining = total.saturating_sub(MAX_SKILL_CATALOG_OMISSION_MARKERS);
    if remaining > 0 {
        lines.push(format!(
            "omitted skills: {remaining} more (catalog budget)\n"
        ));
    }
    lines
}

/// 技能集合指纹（会话基线变更检测用）：name + scope + description + 内容哈希。
fn skill_set_hash(skills: &[HotloadedSkill]) -> u64 {
    let mut hash_input = Vec::new();
    for skill in skills {
        hash_input.extend_from_slice(skill.name.as_bytes());
        hash_input.extend_from_slice(skill.scope.as_str().as_bytes());
        hash_input.extend_from_slice(skill.description.as_bytes());
        hash_input.extend_from_slice(&skill.fingerprint.content_hash.to_le_bytes());
    }
    fnv1a_hash(&hash_input)
}

/// 按 [`InstructionBudget`] 丢弃超出注入预算的技能（既有 wave-42 语义，抽出供多处复用）。
///
/// 超限时从**最低优先级**开始丢弃：rank 大者先出局，同 rank 全局层先出局。
fn apply_skill_injection_budget(
    skills: Vec<HotloadedSkill>,
    budget: InstructionBudget,
) -> Vec<HotloadedSkill> {
    let costs: Vec<usize> = skills.iter().map(skill_injection_cost).collect();
    let mut total: usize = costs.iter().sum();
    if total <= budget.max_bytes {
        return skills;
    }

    // 优先级从高到低：rank 小者优先；同 rank 时项目层优先；再按既定列表顺序。
    let mut by_priority: Vec<usize> = (0..skills.len()).collect();
    by_priority.sort_by(|&left, &right| {
        skills[left]
            .rank
            .cmp(&skills[right].rank)
            .then(skills[right].scope.cmp(&skills[left].scope))
            .then(left.cmp(&right))
    });

    let mut dropped: HashSet<usize> = HashSet::new();
    for index in by_priority.into_iter().rev() {
        if total <= budget.max_bytes {
            break;
        }
        total -= costs[index];
        dropped.insert(index);
    }

    skills
        .into_iter()
        .enumerate()
        .filter(|(index, _)| !dropped.contains(index))
        .map(|(_, skill)| skill)
        .collect()
}

/// 会话中途变更 notice 的唯一拼装实现（wave-42 Append-Only 语义，逐字保持既有文案）。
fn build_session_delta_notice(
    baseline: &SessionBaselineSnapshot,
    current_skills: &[HotloadedSkill],
    const_changed: bool,
    skills_changed: bool,
    budget_changed: bool,
    catalog_changed: bool,
) -> String {
    let mut delta_notice = String::from("Notice: Workspace configuration was updated mid-session:\n");
    if const_changed {
        delta_notice
            .push_str("- Workspace constitution/rules updated. Latest guidelines are active.\n");
    }
    if budget_changed {
        delta_notice
            .push_str("- Instruction injection budget changed. Baseline prefix is retained.\n");
    }
    if catalog_changed {
        delta_notice.push_str(
            "- Skill catalog budget changed. Catalog re-rendered from the session baseline to fit the current context window.\n",
        );
    }
    if skills_changed {
        let old_names: Vec<_> = baseline.skills.iter().map(|s| &s.name).collect();
        let new_names: Vec<_> = current_skills.iter().map(|s| &s.name).collect();
        let added: Vec<_> = new_names
            .iter()
            .filter(|n| !old_names.contains(n))
            .collect();
        let removed: Vec<_> = old_names
            .iter()
            .filter(|n| !new_names.contains(n))
            .collect();
        if !added.is_empty() {
            delta_notice.push_str(&format!("- Newly installed skills: {:?}\n", added));
        }
        if !removed.is_empty() {
            delta_notice.push_str(&format!("- Removed skills: {:?}\n", removed));
        }
    }
    delta_notice
}

/// 宪法来源候选：作用域 + 被采纳的文件 + 其作用域根（canonical 包含校验基准）。
#[derive(Clone, Debug, PartialEq, Eq)]
struct ConstitutionSource {
    scope: AgentScope,
    path: PathBuf,
    root: PathBuf,
}

/// 采纳判定（fail-closed）：只有「链接本身不是目录/FIFO/设备/套接字」且「canonical 后
/// 是普通文件」且「落在作用域根内」的候选才可被读取。
///
/// 先用 `symlink_metadata` 判型，避免对 FIFO/设备文件发起读取而阻塞或注入非文件内容；
/// 符号链接一律解析后再校验归属，`ln -s /tmp/out/secret ws/AGENTS.md` 这类逃逸被拒绝。
fn is_adoptable_constitution_file(path: &Path, root: &Path) -> bool {
    let Ok(link_metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    let file_type = link_metadata.file_type();
    if file_type.is_dir() || (!file_type.is_file() && !file_type.is_symlink()) {
        return false;
    }

    let (Ok(canonical), Ok(canonical_root)) = (fs::canonicalize(path), fs::canonicalize(root))
    else {
        return false;
    };
    if !canonical.starts_with(&canonical_root) {
        return false;
    }
    match fs::metadata(&canonical) {
        Ok(metadata) => metadata.is_file(),
        Err(_) => false,
    }
}

/// 在根目录内按候选顺序取首个**可采纳**文件。
fn first_existing_constitution(root: &Path) -> Option<PathBuf> {
    for candidate in CONSTITUTION_CANDIDATES {
        let path = root.join(candidate);
        if is_adoptable_constitution_file(&path, root) {
            return Some(path);
        }
    }
    None
}

/// 探测单个目录内的 local overlay（个人覆盖层）：按候选顺序取首个可采纳文件。
fn first_existing_local_overlay(root: &Path) -> Option<PathBuf> {
    for candidate in LOCAL_OVERLAY_CANDIDATES {
        let path = root.join(candidate);
        if is_adoptable_constitution_file(&path, root) {
            return Some(path);
        }
    }
    None
}

/// 汇总宪法来源：全局根按顺序在前，祖先链（root→cwd，浅→深）在后（最高优先级）。
///
/// 每级链目录先取基础候选首命中，再取 local overlay 首命中（overlay 排在 base 之后，
/// 即在同一目录内具有更高优先级）。
fn discover_constitution_sources(
    workspace_root: &Path,
    global_homes: &[PathBuf],
) -> Vec<ConstitutionSource> {
    let mut sources: Vec<ConstitutionSource> = Vec::new();
    for home in global_homes {
        if let Some(path) = first_existing_constitution(home) {
            sources.push(ConstitutionSource {
                scope: AgentScope::Global,
                path,
                root: home.clone(),
            });
        }
    }
    // 宪法发现不做无界上探：bound 未由调用方给出前 root=cwd=workspace_root，避免载入宿主 home 层指令。
    let chain = ancestor_chain(workspace_root, workspace_root);
    for dir in chain {
        if let Some(path) = first_existing_constitution(&dir) {
            sources.push(ConstitutionSource {
                scope: AgentScope::Project,
                path,
                root: dir.clone(),
            });
        }
        if let Some(path) = first_existing_local_overlay(&dir) {
            sources.push(ConstitutionSource {
                scope: AgentScope::Project,
                path,
                root: dir.clone(),
            });
        }
    }
    sources
}

/// 单个技能根的解析结果——**扫描与加载共用的单一枚举**（PA-173 / ISSUE-20261111-007）。
///
/// rank 数值越小越优先；**项目带（100~399）恒小于全局带（400+）**，
/// 「项目层永远最高优先级」不变量不得被仓库根或自定义根破坏。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillRootSpec {
    /// 作用域：`Global` = 用户自有域（允许符号链接逃逸）；`Project` = 仓库内（严格 canonical 包含）。
    pub scope: AgentScope,
    /// 技能根目录本身（其下直接是各技能子目录）。
    pub dir: PathBuf,
    /// 注入与去重优先级。
    pub rank: u16,
    /// 项目级根的 canonical 包含上界（仓库根 / 会话根）；全局根为 `None`。
    pub containment_base: Option<PathBuf>,
}

/// 项目带 rank：会话根（cwd）三层目录，沿用 ADR 0045 冻结值（`.dsh` → `.agents` → `.claude`）。
const SKILL_RANK_WORKSPACE_DSH: u16 = 100;
const SKILL_RANK_WORKSPACE_AGENTS: u16 = 200;
const SKILL_RANK_WORKSPACE_CLAUDE: u16 = 300;
/// 项目带 rank：仓库根（`.git` 上探）三层——必须整体劣于会话根三层，
/// 才满足 dsh 的「近者覆盖远者」（exact scope 优于 ancestor）。
const SKILL_RANK_REPO_DSH: u16 = 350;
const SKILL_RANK_REPO_AGENTS: u16 = 360;
const SKILL_RANK_REPO_CLAUDE: u16 = 370;
/// 全局带 rank：自定义技能根起始值（`450 + index`，封顶 499）。
///
/// 落在全局带兜底段：劣于产品自有根（400），优于 `~/.dsh`(500) / `~/.agents`(510) /
/// `~/.claude`(520)，即 dsh 的「customSkillDirs 优于用户全局根」语义。
const SKILL_RANK_CUSTOM_BASE: u16 = 450;
/// `.git` 上探的最大层数（防御病态深路径上的无界 IO）。
const SKILL_GIT_ROOT_MAX_ANCESTORS: usize = 32;
/// 项目内技能目录（相对所属根）：顺序即同层内的优先级。
const PROJECT_SKILL_DIRS: [&str; 3] = [".dsh/skills", ".agents/skills", ".claude/skills"];

/// 汇总 Skills 根目录与注入 rank：全局根在前，项目级目录在后（更高优先级）。
///
/// 这是技能根的**唯一枚举**：`scan_skills*`（可见性）与
/// [`crate::agent::path_permission::trusted_skill_roots_with_policy`]（可加载性）都消费它，
/// 不允许任何一侧另建根清单（否则复现「列得出、载不进」的 P1-6 缺陷）。
pub fn enumerate_skill_roots(
    workspace_root: &Path,
    global_homes: &[PathBuf],
    custom_skill_dirs: &[PathBuf],
) -> Vec<SkillRootSpec> {
    let mut specs = enumerate_global_skill_roots(global_homes, custom_skill_dirs);
    specs.extend(enumerate_project_skill_roots(workspace_root));
    specs
}

/// 全局（用户级）技能根：全局根 `skills/` + `~/.dsh/skills` + 自定义技能根。
///
/// - 全局根来自调用方给出的 `global_homes`（默认入口为 [`resolve_global_agent_homes`]），
///   rank 由 [`GlobalRootIdentity::rank`] 决定；
/// - `~/.dsh/skills` 在此单独补齐：它只作为**技能根**参与发现（rank=500，兑现声明表），
///   不进入宪法分层，避免既有提示词基线因新增全局指令层而漂移；
/// - 自定义根（[`PONY_AGENT_SKILL_DIRS_ENV`]）按用户自有域处理（允许符号链接逃逸），rank 落 450+。
pub fn enumerate_global_skill_roots(
    global_homes: &[PathBuf],
    custom_skill_dirs: &[PathBuf],
) -> Vec<SkillRootSpec> {
    // 身份解析（env / home_dir / 已知全局根）在一次枚举内只采样一次，
    // 保证 rank 是本次枚举的纯函数快照，不会在中途因 env 变化而漂移。
    let identity = GlobalRootIdentity::snapshot();
    let mut specs: Vec<SkillRootSpec> = Vec::new();
    for (index, home) in global_homes.iter().enumerate() {
        push_skill_root(
            &mut specs,
            AgentScope::Global,
            home.join("skills"),
            identity.rank(home, index),
            None,
        );
    }
    if let Some(home_dir) = identity.home_dir.as_deref() {
        let dsh_home = home_dir.join(".dsh");
        push_skill_root(
            &mut specs,
            AgentScope::Global,
            dsh_home.join("skills"),
            identity.rank(&dsh_home, global_homes.len()),
            None,
        );
    }
    for (index, dir) in custom_skill_dirs.iter().enumerate() {
        let rank = SKILL_RANK_CUSTOM_BASE.saturating_add(index.min(49) as u16);
        push_skill_root(&mut specs, AgentScope::Global, dir.clone(), rank, None);
    }
    specs
}

/// 项目（仓库内）技能根：仓库根（`.git` 上探）三层 + 会话根三层。
///
/// 上探只停在**最近的含 `.git` 的目录**（文件形态同样成立，兼容 worktree / submodule），
/// 命中不到即退回会话根单层——不会把 `~` 或 `/` 当项目根去收罗系统目录。
pub fn enumerate_project_skill_roots(workspace_root: &Path) -> Vec<SkillRootSpec> {
    let mut specs: Vec<SkillRootSpec> = Vec::new();
    if let Some(repo_root) = find_git_root(workspace_root).filter(|root| root != workspace_root) {
        push_project_level(
            &mut specs,
            &repo_root,
            [
                SKILL_RANK_REPO_DSH,
                SKILL_RANK_REPO_AGENTS,
                SKILL_RANK_REPO_CLAUDE,
            ],
        );
    }
    push_project_level(
        &mut specs,
        workspace_root,
        [
            SKILL_RANK_WORKSPACE_DSH,
            SKILL_RANK_WORKSPACE_AGENTS,
            SKILL_RANK_WORKSPACE_CLAUDE,
        ],
    );
    specs
}

/// 由 `start` 向上探测最近的 `.git`（目录或文件形态）。
///
/// 上探上限 [`SKILL_GIT_ROOT_MAX_ANCESTORS`] 层；未命中返回 `None`。
pub fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);
    let mut hops = 0usize;
    while let Some(dir) = current {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        if hops >= SKILL_GIT_ROOT_MAX_ANCESTORS {
            return None;
        }
        hops += 1;
        current = dir.parent();
    }
    None
}

/// 把一个根的三层目录按给定 rank 追加进枚举（`containment_base` = 该根自身）。
fn push_project_level(specs: &mut Vec<SkillRootSpec>, base: &Path, ranks: [u16; 3]) {
    for (dir, rank) in PROJECT_SKILL_DIRS.iter().zip(ranks) {
        push_skill_root(
            specs,
            AgentScope::Project,
            base.join(dir),
            rank,
            Some(base.to_path_buf()),
        );
    }
}

/// 全局根身份快照：`$PONY_AGENT_HOME`、`home_dir` 与已知产品全局根**同一扫描内单次采样**；
/// 跨调用允许随环境变化（环境变了 rank 也随之变化，属预期行为）。
///
/// 冻结 rank 表：`$PONY_AGENT_HOME/skills`（及产品自有根）=400、`~/.dsh/skills`=500、
/// `~/.agents/skills`=510、`~/.claude/skills`=520；项目带 `.dsh`=100 / `.agents`=200 /
/// `.claude`=300。无法识别身份的注入根按 `450 + index`（`index` 上限 49）兜底，
/// 恒落在全局带内，不得越过 500/510/520。
struct GlobalRootIdentity {
    env_home: Option<PathBuf>,
    home_dir: Option<PathBuf>,
    known: Vec<PathBuf>,
}

impl GlobalRootIdentity {
    /// 单次采样：环境变量与系统目录只在本函数里读取一次，之后 rank 为纯函数。
    fn snapshot() -> Self {
        let env_home = std::env::var_os(PONY_AGENT_HOME_ENV)
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty());
        let home_dir = dirs::home_dir();
        let config_dir = dirs::config_dir();
        let known = resolve_global_agent_homes_from(
            env_home.as_deref(),
            config_dir.as_deref(),
            home_dir.as_deref(),
        );
        Self {
            env_home,
            home_dir,
            known,
        }
    }

    /// 基于本次快照计算单个全局根的注入 rank（越小越优先）；不再读取环境状态。
    fn rank(&self, home: &Path, index: usize) -> u16 {
        if let Some(env) = self.env_home.as_deref() {
            if home == env {
                return 400;
            }
        }
        if let Some(home_dir) = self.home_dir.as_deref() {
            if home == home_dir.join(".dsh") {
                return 500;
            }
            if home == home_dir.join(".agents") {
                return 510;
            }
            if home == home_dir.join(".claude") {
                return 520;
            }
        }
        // 产品自有全局根（config_dir/pony-agent）与 `$PONY_AGENT_HOME` 同属产品带。
        if self.known.iter().any(|candidate| candidate == home) {
            return 400;
        }

        450u16.saturating_add(index.min(49) as u16)
    }
}

fn push_skill_root(
    specs: &mut Vec<SkillRootSpec>,
    scope: AgentScope,
    dir: PathBuf,
    rank: u16,
    containment_base: Option<PathBuf>,
) {
    if !dir.is_dir() || specs.iter().any(|existing| existing.dir == dir) {
        return;
    }
    specs.push(SkillRootSpec {
        scope,
        dir,
        rank,
        containment_base,
    });
}

/// 按 UTF-8 字符边界截断到不超过 `max_bytes`（避免切裂多字节字符）。
///
/// 截断后的文本由调用方追加 `truncated <path>` 标记。
fn truncate_at_char_boundary(content: &str, max_bytes: usize) -> &str {
    if content.len() <= max_bytes {
        return content;
    }
    let mut end = max_bytes;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    &content[..end]
}

/// 内容去重归一化：整体 trim + 逐行 trim。
///
/// 逐行 trim 用于抹平「同文不同缩进/行尾空白」的形态差异（例如全局模板与项目
/// 副本仅空行/行尾空格不同），使 trim 后相同的层可被识别为同一份。
fn normalize_constitution_content(content: &str) -> String {
    content
        .trim()
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
}

/// 渲染多层宪法内容。
///
/// 只有一个项目层时返回原始字节（与既有单文件行为完全一致）；多层级时按
/// 全局→项目的顺序拼接，并为每层标注作用域与来源路径。
fn render_constitution(layers: &[&ConstitutionLayer]) -> String {
    if layers.len() == 1 && layers[0].scope == AgentScope::Project {
        return layers[0].content.clone();
    }

    let mut rendered = String::new();
    for layer in layers {
        if !rendered.is_empty() {
            rendered.push_str("\n\n");
        }
        let heading = match layer.scope {
            AgentScope::Global => "Global user-level instructions",
            AgentScope::Project => "Project instructions",
        };
        rendered.push_str(&format!(
            "## {} ({})\n",
            heading,
            layer.source_path.display()
        ));
        rendered.push_str(layer.content.trim_end());
    }
    rendered
}

/// 预算渲染候选层（`candidates` 顺序 = 渲染顺序 = 优先级从低到高）。
/// 宪法层被省略的原因（预算裁决的**机器账**，不进模型可见文本）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstitutionOmissionReason {
    /// trim 后与更高优先级层同文，被去重丢弃（不产生 `omitted` 文本标记，历史行为）。
    DuplicateContent,
    /// 单文件超过 `max_source_bytes` 且非最高优先级层，整层省略。
    OversizedSource,
    /// 渲染后超预算，按低→高优先级逐层省略。
    BudgetExceeded,
}

impl ConstitutionOmissionReason {
    /// 稳定诊断串（可观测面与测试断言共用）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DuplicateContent => "duplicate_content",
            Self::OversizedSource => "oversized_source",
            Self::BudgetExceeded => "budget_exceeded",
        }
    }
}

/// 一条被省略的宪法层。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OmittedConstitutionLayer {
    pub path: PathBuf,
    pub reason: ConstitutionOmissionReason,
}

/// 最高优先级层被截断的机器账。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TruncatedConstitutionLayer {
    pub path: PathBuf,
    /// 正文原始字节数（截断前）。
    pub original_bytes: usize,
    /// 实际保留的正文字节数（不含 `truncated <path>` 标记）。
    pub included_bytes: usize,
}

/// 宪法预算裁决的结构化报告。
///
/// 背景（backlog B142 / item-5）：模型可见文本里只有 `omitted <path>` / `truncated <path>`
/// 两种字符串标记，调用方与可观测面无法知道**为什么**省略、省了多少字节、去重丢掉了哪些层。
/// 本报告只做观测与测试，不改变任何模型可见字节。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConstitutionBudgetReport {
    /// 被省略的层（去重 / 超源文件上限 / 超预算），按裁决顺序记录。
    pub omitted: Vec<OmittedConstitutionLayer>,
    /// 最高优先级层的截断记录（未发生截断时为 `None`）。
    pub truncated: Option<TruncatedConstitutionLayer>,
    /// 是否走到最后一步「极限预算硬裁剪」（连标题与标记都放不下）。
    pub clamped: bool,
}

impl ConstitutionBudgetReport {
    /// 是否发生了任何非默认裁决。
    pub fn is_empty(&self) -> bool {
        self.omitted.is_empty() && self.truncated.is_none() && !self.clamped
    }
}

struct BudgetCandidate {
    source_index: usize,
    layer: ConstitutionLayer,
    oversized: bool,
    kept: bool,
}

/// 跨层去重标记：`true` 表示保留最高优先级的一份。
///
/// 逆序扫描（高优先级在前）保证「后出现者胜」；超限层的正文只是文件首部，
/// 不参与内容判等、也不会被去重丢弃。
fn dedupe_keep_flags(candidates: &[BudgetCandidate]) -> Vec<bool> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut keep = vec![true; candidates.len()];
    for (index, candidate) in candidates.iter().enumerate().rev() {
        if candidate.oversized {
            continue;
        }
        let key = normalize_constitution_content(&candidate.layer.content);
        if !seen.insert(key) {
            keep[index] = false;
        }
    }
    keep
}

/// 按既定格式拼装：保留层走 `render_constitution`，被省略层在末尾按优先级高→低
/// 追加 `omitted <path>`。
fn compose_constitution(candidates: &[BudgetCandidate]) -> String {
    let kept: Vec<&ConstitutionLayer> = candidates
        .iter()
        .filter(|candidate| candidate.kept)
        .map(|candidate| &candidate.layer)
        .collect();
    let mut rendered = render_constitution(&kept);

    let mut omitted: Vec<&BudgetCandidate> = candidates
        .iter()
        .filter(|candidate| !candidate.kept)
        .collect();
    omitted.sort_by(|left, right| right.source_index.cmp(&left.source_index));
    for candidate in omitted {
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        rendered.push_str(&format!(
            "omitted {}",
            candidate.layer.source_path.display()
        ));
    }
    rendered
}

/// 预算内渲染：先按渲染后字节定保留集合（必要时截断最高优先级层），再输出。
///
/// 返回 `(模型可见文本, 结构化报告)`；文本与历史实现逐字节一致，报告只做观测。
fn render_constitution_with_budget(
    snapshots: &[ConstitutionSourceSnapshot],
    layers: Vec<ConstitutionLayer>,
    max_bytes: usize,
) -> (String, ConstitutionBudgetReport) {
    let source_index: HashMap<&Path, usize> = snapshots
        .iter()
        .enumerate()
        .map(|(index, snapshot)| (snapshot.path.as_path(), index))
        .collect();

    let mut candidates: Vec<BudgetCandidate> = layers
        .into_iter()
        .map(|layer| {
            let index = source_index
                .get(layer.source_path.as_path())
                .copied()
                .unwrap_or(0);
            let oversized = snapshots
                .get(index)
                .map(|snapshot| snapshot.oversized)
                .unwrap_or(false);
            BudgetCandidate {
                source_index: index,
                layer,
                oversized,
                kept: true,
            }
        })
        .collect();

    let mut report = ConstitutionBudgetReport::default();

    // 1) 跨层去重：trim 后同文只保留最高优先级一份，被丢弃者不产生 omitted 文本标记，
    //    但记账到报告（`DuplicateContent`）：否则「少注入了一整份规则」在观测面上不可见。
    let keep_flags = dedupe_keep_flags(&candidates);
    let mut cursor = 0usize;
    candidates.retain(|candidate| {
        let keep = keep_flags[cursor];
        if !keep {
            report.omitted.push(OmittedConstitutionLayer {
                path: candidate.layer.source_path.clone(),
                reason: ConstitutionOmissionReason::DuplicateContent,
            });
        }
        cursor += 1;
        keep
    });

    // 2) 低优先级超 `max_source_bytes` 的层整层省略；最高优先级层保留首部待预算裁决。
    let top_source_index = candidates
        .iter()
        .filter(|candidate| candidate.kept)
        .map(|candidate| candidate.source_index)
        .max()
        .unwrap_or(usize::MAX);
    let oversized_victims: Vec<PathBuf> = candidates
        .iter()
        .filter(|candidate| candidate.oversized && candidate.source_index != top_source_index)
        .map(|candidate| candidate.layer.source_path.clone())
        .collect();
    for candidate in candidates.iter_mut() {
        if candidate.oversized && candidate.source_index != top_source_index {
            candidate.kept = false;
        }
    }
    report.omitted.extend(oversized_victims.into_iter().map(|path| {
        OmittedConstitutionLayer {
            path,
            reason: ConstitutionOmissionReason::OversizedSource,
        }
    }));

    // 3) 从最低优先级保留层开始逐层省略，直到渲染后字节落入预算。
    let mut rendered = compose_constitution(&candidates);
    while rendered.len() > max_bytes {
        // 最低优先级 = 候选列表最靠前（source_index 最小）的保留层。
        let victim = candidates
            .iter()
            .position(|candidate| candidate.kept && candidate.source_index != top_source_index);
        match victim {
            Some(index) => {
                report.omitted.push(OmittedConstitutionLayer {
                    path: candidates[index].layer.source_path.clone(),
                    reason: ConstitutionOmissionReason::BudgetExceeded,
                });
                candidates[index].kept = false;
                rendered = compose_constitution(&candidates);
            }
            None => break,
        }
    }

    // 4) 最高优先级层永不整层丢弃：UTF-8 边界截断 + `truncated <path>` 标记。
    if rendered.len() > max_bytes {
        if let Some(top) = candidates.iter().rposition(|candidate| candidate.kept) {
            let original = candidates[top].layer.content.clone();
            candidates[top].layer.content.clear();
            let envelope_len = compose_constitution(&candidates).len();
            let marker = format!("truncated {}", candidates[top].layer.source_path.display());
            let available = max_bytes
                .saturating_sub(envelope_len)
                .saturating_sub(marker.len() + 1);
            let mut content =
                String::with_capacity(available.min(original.len()) + marker.len() + 1);
            content.push_str(truncate_at_char_boundary(&original, available));
            content.push('\n');
            content.push_str(&marker);
            report.truncated = Some(TruncatedConstitutionLayer {
                path: candidates[top].layer.source_path.clone(),
                original_bytes: original.len(),
                included_bytes: content.len() - marker.len() - 1,
            });
            candidates[top].layer.content = content;
            rendered = compose_constitution(&candidates);
        }
    }

    // 5) 极限预算（连标题/标记都放不下）：退化为不超过 max_bytes 的截断摘要。
    if rendered.len() > max_bytes {
        rendered = clamp_to_budget(&rendered, max_bytes);
        report.clamped = true;
    }
    (rendered, report)
}

/// 硬裁剪到 `max_bytes` 以内的截断摘要（UTF-8 安全，尾部以 `...` 标记）。
fn clamp_to_budget(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    if max_bytes <= 3 {
        return ".".repeat(max_bytes);
    }
    let mut clamped = truncate_at_char_boundary(text, max_bytes - 3).to_string();
    clamped.push_str("...");
    clamped
}

/// 技能描述的字符数上限（Agent Skills 规范；wave-58 契约 F2-2）。
const MAX_SKILL_DESCRIPTION_CHARS: usize = 1024;

/// SKILL.md frontmatter 的解析结果（wave-58 契约 F2-1）。
#[derive(Debug, Default)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
    user_invocable: Option<bool>,
    allowed_tools: Vec<String>,
    disable_model_invocation: Option<bool>,
    model_invocable: Option<bool>,
}

/// frontmatter 字段取值的两种形态（标量 / 块列表）。
#[derive(Debug)]
enum FrontmatterValue {
    Scalar(String),
    List(Vec<String>),
}

/// 块标量风格（wave-58 契约 F2-1）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlockScalarStyle {
    /// `|`：字面风格 —— 续行以换行连接。
    Literal,
    /// `>`：折叠风格 —— 续行以空格连接，空行 run 合并为单换行。
    Folded,
}

/// 块标量尾换行策略（YAML chomping）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlockScalarChomp {
    /// 默认（clip）：保留单个尾换行。
    Clip,
    /// `-`（strip）：去掉尾换行。
    Strip,
    /// `+`（keep）：保留尾空行。
    Keep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BlockScalarHeader {
    style: BlockScalarStyle,
    chomp: BlockScalarChomp,
}

/// 识别块标量头：`|` / `>` 之后允许一个 chomping 标记（`-` / `+`）、缩进指示符（`1`..=`9`）
/// 与尾随空白 / 注释；其他内容（如 `|foo`）不算块标量，按普通标量处理（fail-safe）。
fn parse_block_scalar_header(value: &str) -> Option<BlockScalarHeader> {
    let mut characters = value.chars();
    let style = match characters.next()? {
        '|' => BlockScalarStyle::Literal,
        '>' => BlockScalarStyle::Folded,
        _ => return None,
    };
    let mut chomp = BlockScalarChomp::Clip;
    let mut saw_chomp = false;
    for character in characters {
        match character {
            '-' if !saw_chomp => {
                chomp = BlockScalarChomp::Strip;
                saw_chomp = true;
            }
            '+' if !saw_chomp => {
                chomp = BlockScalarChomp::Keep;
                saw_chomp = true;
            }
            '1'..='9' => {}
            ' ' | '\t' => break,
            _ => return None,
        }
    }
    Some(BlockScalarHeader { style, chomp })
}

/// 去掉行首不超过 `indent` 个空白字符。
fn strip_indent(line: &str, indent: usize) -> &str {
    if indent == 0 {
        return line;
    }
    let mut removed = 0usize;
    let mut byte_index = 0usize;
    for (position, character) in line.char_indices() {
        if removed >= indent || (character != ' ' && character != '\t') {
            byte_index = position;
            break;
        }
        removed += 1;
        byte_index = position + character.len_utf8();
    }
    &line[byte_index..]
}

/// 折叠标量拼装（`>`）：续行以空格连接，空行 run 合并为单换行（wave-58 F2-1）。
fn fold_block_lines(lines: &[&str]) -> String {
    let mut folded = String::new();
    let mut pending_blank = 0usize;
    let mut started = false;
    for line in lines {
        if line.trim().is_empty() {
            pending_blank += 1;
            continue;
        }
        if started {
            if pending_blank == 0 {
                folded.push(' ');
            } else {
                for _ in 0..pending_blank {
                    folded.push('\n');
                }
            }
        }
        folded.push_str(line);
        started = true;
        pending_blank = 0;
    }
    folded
}

/// 读取块标量续行（wave-58 F2-1）：返回 (文本, 下一个待处理行号)。
///
/// 续行 = 空行或缩进更深（列 0 起算）的行；遇到列 0 起的非空行（下一个键 / 闭合 `---`）结束。
/// 公共缩进取非空续行的最小缩进并剥离；尾部空行计入 chomping。
fn read_block_scalar(lines: &[&str], start: usize, header: BlockScalarHeader) -> (String, usize) {
    let mut collected: Vec<&str> = Vec::new();
    let mut index = start;
    while index < lines.len() {
        let raw = lines[index];
        if raw.trim().is_empty() || raw.starts_with(' ') || raw.starts_with('\t') {
            collected.push(raw);
            index += 1;
            continue;
        }
        break;
    }

    let mut trailing_blank = 0usize;
    while let Some(last) = collected.last() {
        if last.trim().is_empty() {
            collected.pop();
            trailing_blank += 1;
        } else {
            break;
        }
    }

    let common_indent = collected
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start_matches(|c| c == ' ' || c == '\t').len())
        .min()
        .unwrap_or(0);
    let dedented: Vec<&str> = collected
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                ""
            } else {
                strip_indent(line, common_indent)
            }
        })
        .collect();

    let mut text = match header.style {
        BlockScalarStyle::Literal => dedented.join("\n"),
        BlockScalarStyle::Folded => fold_block_lines(&dedented),
    };
    if !text.is_empty() {
        match header.chomp {
            BlockScalarChomp::Strip => {}
            BlockScalarChomp::Clip => text.push('\n'),
            BlockScalarChomp::Keep => {
                text.push('\n');
                for _ in 0..trailing_blank {
                    text.push('\n');
                }
            }
        }
    }
    (text, index)
}

/// 读取块列表续行（`- item` 形态，wave-58 F2-1）：返回 (条目, 下一个待处理行号)。
fn read_block_list(lines: &[&str], start: usize) -> (Vec<String>, usize) {
    let mut items: Vec<String> = Vec::new();
    let mut index = start;
    while index < lines.len() {
        let line = lines[index].trim();
        if line.is_empty() {
            index += 1;
            continue;
        }
        if line == "-" || line.starts_with("- ") {
            let item = unquote_scalar(line.trim_start_matches('-').trim());
            if !item.is_empty() {
                items.push(item);
            }
            index += 1;
            continue;
        }
        break;
    }
    (items, index)
}

/// 去掉两端成对的引号（单 / 双），并处理最小转义（`\"` `\n` `\\`）。
fn unquote_scalar(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let (first, last) = (bytes[0], bytes[value.len() - 1]);
        if first == b'"' && last == b'"' {
            let inner = &value[1..value.len() - 1];
            return inner
                .replace("\\\"", "\"")
                .replace("\\n", "\n")
                .replace("\\\\", "\\");
        }
        if first == b'\'' && last == b'\'' {
            return value[1..value.len() - 1].replace("''", "'");
        }
    }
    value.to_string()
}

/// 解析 frontmatter 布尔值（对齐 dsh `frontmatterBoolean` 的宽松口径）。
///
/// 缺陷背景：`bool::from_str` 只认小写 `true`/`false`，因此 `disable-model-invocation: True`
/// 会被 `unwrap_or(false)` 静默吃掉 ⇒ 策略闸门 fail-open（大写首字母即可绕过）。
/// 口径：大小写不敏感，接受 `true/yes/on/1` 与 `false/no/off/0`；其余（含空值）回退 `default`。
fn parse_frontmatter_bool(raw: &str, default: bool) -> bool {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => true,
        "false" | "no" | "off" | "0" => false,
        _ => default,
    }
}

/// 把单个 frontmatter 字段写入解析结果。
fn apply_frontmatter_field(parsed: &mut SkillFrontmatter, key: &str, value: FrontmatterValue) {
    match (key, value) {
        ("name", FrontmatterValue::Scalar(scalar)) => {
            let scalar = scalar.trim();
            if !scalar.is_empty() {
                parsed.name = Some(scalar.to_string());
            }
        }
        ("description", FrontmatterValue::Scalar(scalar)) => {
            parsed.description = Some(scalar);
        }
        ("description", FrontmatterValue::List(items)) => {
            parsed.description = Some(items.join(", "));
        }
        ("user-invocable", FrontmatterValue::Scalar(scalar)) => {
            parsed.user_invocable = Some(parse_frontmatter_bool(&scalar, true));
        }
        ("disable-model-invocation", FrontmatterValue::Scalar(scalar)) => {
            parsed.disable_model_invocation = Some(parse_frontmatter_bool(&scalar, false));
        }
        ("allowed-tools", FrontmatterValue::List(items)) => {
            parsed.allowed_tools = items;
        }
        ("allowed-tools", FrontmatterValue::Scalar(scalar)) => {
            parsed.allowed_tools = scalar
                .split(',')
                .map(|item| unquote_scalar(item.trim()))
                .filter(|item| !item.is_empty())
                .collect();
        }
        ("model-invocable", FrontmatterValue::Scalar(scalar)) => {
            parsed.model_invocable = Some(parse_frontmatter_bool(&scalar, true));
        }
        _ => {}
    }
}

/// 解析 frontmatter 围栏内的字段（wave-58 F2-1）；无 `---` 围栏时返回 `None`。
fn parse_skill_frontmatter(content: &str) -> Option<SkillFrontmatter> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    let rest = &trimmed[3..];
    let lines: Vec<&str> = rest.lines().collect();
    let mut parsed = SkillFrontmatter::default();
    let mut index = 0usize;
    while index < lines.len() {
        let raw = lines[index];
        let line = raw.trim();
        let indented = raw.starts_with(' ') || raw.starts_with('\t');
        // 闭合围栏：列 0 起、内容恰为 `---`（容忍行尾空白 / CRLF）。
        if !indented && line == "---" {
            break;
        }
        if line.is_empty() || line.starts_with('#') {
            index += 1;
            continue;
        }
        // 缩进行属于上一字段的续行（块标量 / 块列表已消费），此处仅防御性跳过。
        if indented {
            index += 1;
            continue;
        }
        let Some(colon_index) = line.find(':') else {
            index += 1;
            continue;
        };
        let key = line[..colon_index].trim().to_string();
        let value = line[colon_index + 1..].trim().to_string();

        if let Some(header) = parse_block_scalar_header(&value) {
            let (block, next_index) = read_block_scalar(&lines, index + 1, header);
            apply_frontmatter_field(&mut parsed, &key, FrontmatterValue::Scalar(block));
            index = next_index;
            continue;
        }
        if value.is_empty() {
            let (items, next_index) = read_block_list(&lines, index + 1);
            if !items.is_empty() {
                apply_frontmatter_field(&mut parsed, &key, FrontmatterValue::List(items));
            }
            index = next_index.max(index + 1);
            continue;
        }

        let scalar = unquote_scalar(&value);
        if key == "allowed-tools" && scalar.starts_with('[') && scalar.ends_with(']') {
            let items = scalar[1..scalar.len() - 1]
                .split(',')
                .map(|item| unquote_scalar(item.trim()))
                .filter(|item| !item.is_empty())
                .collect();
            apply_frontmatter_field(&mut parsed, &key, FrontmatterValue::List(items));
        } else {
            apply_frontmatter_field(&mut parsed, &key, FrontmatterValue::Scalar(scalar));
        }
        index += 1;
    }
    Some(parsed)
}

/// 描述截断（wave-58 契约 F2-2）：超过 `max_chars` 字符时按字符（UTF-8 码点）边界截断。
fn truncate_description(description: &str, max_chars: usize) -> String {
    if description.chars().count() <= max_chars {
        return description.to_string();
    }
    description.chars().take(max_chars).collect()
}

/// 把可能含换行的文本压平成单行（wave-58 F1-2 / F2-1：技能清单必须「一个技能一行」）。
fn flatten_single_line(text: &str) -> String {
    let mut flattened = String::with_capacity(text.len());
    let mut last_was_space = false;
    for character in text.chars() {
        let normalized = if character == '\n' || character == '\r' || character == '\t' {
            ' '
        } else {
            character
        };
        if normalized == ' ' {
            if last_was_space {
                continue;
            }
            last_was_space = true;
        } else {
            last_was_space = false;
        }
        flattened.push(normalized);
    }
    flattened.trim().to_string()
}

/// 解析 SKILL.md 的标准 YAML Frontmatter（wave-58 F2-1 / F2-2）。
///
/// 支持的最小 YAML 子集（手写，不引入外部依赖）：
/// - 单行标量 `key: value`（值内可含 `:`；两端引号剥离）；
/// - **块标量** `|` `|-` `|+` / `>` `>-` `>+`：字面以换行连接、折叠以空格连接并合并空行，
///   自动剥离公共缩进，`description` 不再退化为字面量 `"|"` / `">"`；
/// - `allowed-tools` 三形态：块列表（`- item`）、行内数组（`[a, b]`）、逗号分隔；
/// - `description` 超过 [`MAX_SKILL_DESCRIPTION_CHARS`] 字符时按字符边界截断。
pub(crate) fn parse_skill_metadata(
    content: &str,
    fallback_name: &str,
) -> (String, String, bool, Vec<String>, bool, bool) {
    let Some(parsed) = parse_skill_frontmatter(content) else {
        return (
            fallback_name.to_string(),
            truncate_description(
                &extract_fallback_description(content),
                MAX_SKILL_DESCRIPTION_CHARS,
            ),
            true,
            Vec::new(),
            false,
            true,
        );
    };

    let name = parsed
        .name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| fallback_name.to_string());
    let mut description = parsed.description.unwrap_or_default();
    if description.trim().is_empty() {
        description = extract_fallback_description(content);
    }
    let disable_model_invocation = parsed.disable_model_invocation.unwrap_or(false);
    let model_invocable = if disable_model_invocation {
        false
    } else {
        parsed.model_invocable.unwrap_or(true)
    };
    let user_invocable = parsed.user_invocable.unwrap_or(true);
    (
        name,
        truncate_description(description.trim_end(), MAX_SKILL_DESCRIPTION_CHARS),
        user_invocable,
        parsed.allowed_tools,
        disable_model_invocation,
        model_invocable,
    )
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

/// 超限技能名的有界读取（wave-58 F2-5 / D3）：只读 frontmatter 前缀取 `name`。
///
/// 用于 `too_large` 的 omitted 记录 —— 该候选已通过信任判定（受信根内的普通文件），
/// 因此可以有界读取前缀；读不到真实 name 时由调用方回落目录名。
fn skill_name_from_frontmatter_prefix(skill_file: &Path) -> Option<String> {
    let (_fingerprint, content) =
        FileFingerprint::compute_bounded(skill_file, SKILL_FRONTMATTER_READ_LIMIT)?;
    let (name, _description, _user_invocable, _allowed_tools, _disable_model, _model_inv) =
        parse_skill_metadata(&content, "");
    if name.trim().is_empty() {
        None
    } else {
        Some(name)
    }
}

impl WorkspaceHotloader {
    /// 带预算的宪法解析（wave-42 / PA-150）。
    ///
    /// 契约：
    /// - 单层文件超过 `max_source_bytes` 时**有界读取**（不整份入内存）：最高优先级层
    ///   截断保留首部，低优先级层整层省略并计入 `omitted`；
    /// - 预算按**渲染后字节**计（含 `## ... (<path>)` 标题、层间空行、`omitted` 行），
    ///   两遍法先定保留集合再渲染，保证 `rendered.len() <= max_bytes` 严格成立；
    /// - **最高优先级层永不整层丢弃**（无论 Project 还是 Global）：放不下时按 UTF-8
    ///   边界截断正文并追加 `truncated <path>` 标记；预算极小时退化为不超过上限的摘要；
    /// - 低优先级层从低到高逐层省略，每个省略层在末尾按优先级高→低追加 `omitted <path>`；
    /// - 未被省略/截断的层沿用 `render_constitution` 的既有格式（默认预算下与
    ///   `resolve_constitution_with_globals` 字节完全一致）。
    pub fn resolve_constitution_with_budget<P: AsRef<Path>>(
        &self,
        workspace_root: P,
        global_homes: &[PathBuf],
        budget: InstructionBudget,
    ) -> Option<(String, bool)> {
        let sources = discover_constitution_sources(workspace_root.as_ref(), global_homes);
        if sources.is_empty() {
            return None;
        }

        let (snapshots, layers, hit) =
            self.load_constitution_layers(&sources, budget.max_source_bytes);
        if layers.is_empty() {
            // 全部来源读取失败：与既有行为一致，视为无可注入内容（不伪造 omitted）。
            return None;
        }

        let (rendered, report) =
            render_constitution_with_budget(&snapshots, layers, budget.max_bytes);
        // 结构化诊断：预算裁决不是静默行为，省略/截断/硬裁剪都要在可观测面留痕。
        if !report.is_empty() {
            tracing::warn!(
                target: "hotloader",
                omitted = report.omitted.len(),
                duplicated = report
                    .omitted
                    .iter()
                    .filter(|entry| entry.reason == ConstitutionOmissionReason::DuplicateContent)
                    .count(),
                oversized = report
                    .omitted
                    .iter()
                    .filter(|entry| entry.reason == ConstitutionOmissionReason::OversizedSource)
                    .count(),
                budget_exceeded = report
                    .omitted
                    .iter()
                    .filter(|entry| entry.reason == ConstitutionOmissionReason::BudgetExceeded)
                    .count(),
                truncated = report.truncated.is_some(),
                truncated_original_bytes = report.truncated.as_ref().map_or(0, |t| t.original_bytes),
                truncated_included_bytes = report.truncated.as_ref().map_or(0, |t| t.included_bytes),
                clamped = report.clamped,
                "[pony-agent][hotloader] 宪法预算裁决：省略/截断/硬裁剪已发生"
            );
        }
        Some((rendered, hit))
    }

    /// 带预算的 Skills 扫描（wave-42 / PA-150）：注入字节总量受 `max_bytes` 约束，
    /// 超限时从**最低优先级**技能开始丢弃（rank 大者先出局，同 rank 全局层先出局）。
    pub fn scan_skills_with_budget<P: AsRef<Path>>(
        &self,
        workspace_root: P,
        global_homes: &[PathBuf],
        budget: InstructionBudget,
    ) -> (Vec<HotloadedSkill>, bool) {
        let (skills, hit) = self.scan_skills_with_globals(workspace_root, global_homes);
        (apply_skill_injection_budget(skills, budget), hit)
    }
}

/// 单个技能在静态前缀里占用的注入字节数（`- `name` [scope]: description`）。
fn skill_injection_cost(skill: &HotloadedSkill) -> usize {
    skill.name.len() + skill.scope.as_str().len() + skill.description.len() + 8
}

/// 由 cwd 向上探测 `marker` 定位项目根，`bound` 为不可越过的上界。
///
/// - `marker` 可以是文件或目录（`.git` 的 worktree/submodule 形态即为文件）；
/// - 探测包含 `bound` 自身，但绝不越过它（bound 之上不再探测）；
/// - 未命中返回 `bound`。
pub fn find_project_root(cwd: &Path, marker: &str, bound: &Path) -> PathBuf {
    let mut current = cwd.to_path_buf();
    loop {
        if current.join(marker).exists() {
            return current;
        }
        if current == bound {
            return bound.to_path_buf();
        }
        // 只在 bound 之内（含 bound）继续上爬；越界即停在 bound。
        match current.parent() {
            Some(parent) if parent != current && parent.starts_with(bound) => {
                current = parent.to_path_buf();
            }
            _ => return bound.to_path_buf(),
        }
    }
}

/// 生成 root→cwd 的祖先目录链（含两端，浅→深）。
///
/// `root == cwd` 时返回单元素；若 `cwd` 不在 `root` 之下，仍保证链首为 `root`、链尾为 `cwd`。
pub fn ancestor_chain(root: &Path, cwd: &Path) -> Vec<PathBuf> {
    let mut chain: Vec<PathBuf> = vec![cwd.to_path_buf()];
    let mut current = cwd.to_path_buf();
    while current != root {
        match current.parent() {
            Some(parent) if parent != current && parent.starts_with(root) => {
                chain.push(parent.to_path_buf());
                current = parent.to_path_buf();
            }
            _ => break,
        }
    }
    chain.reverse();
    if chain.first().map(PathBuf::as_path) != Some(root) {
        chain.insert(0, root.to_path_buf());
    }
    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "{prefix}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_skill(root: &Path, folder: &str, name: &str, description: &str) {
        let dir = root.join("skills").join(folder);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n\n# Body\n"),
        )
        .unwrap();
    }

    #[test]
    fn test_hotloader_cache_hit_and_reload() {
        let temp_dir = unique_dir("pony_test");

        let agents_md = temp_dir.join("AGENTS.md");
        fs::write(&agents_md, "# Test Rules v1\nDo not break.").unwrap();

        let hotloader = WorkspaceHotloader::new();
        // 显式注入空全局层，避免宿主机用户级指令影响断言。
        let (content, hit) = hotloader
            .resolve_constitution_with_globals(&temp_dir, &[])
            .unwrap();
        assert_eq!(content, "# Test Rules v1\nDo not break.");
        assert!(!hit);

        let (content2, hit2) = hotloader
            .resolve_constitution_with_globals(&temp_dir, &[])
            .unwrap();
        assert_eq!(content2, content);
        assert!(hit2);

        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&agents_md, "# Test Rules v2\nUpdated.").unwrap();
        let (content3, hit3) = hotloader
            .resolve_constitution_with_globals(&temp_dir, &[])
            .unwrap();
        assert_eq!(content3, "# Test Rules v2\nUpdated.");
        assert!(!hit3);

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_global_homes_order_and_dedupe() {
        let home = PathBuf::from("/home/tester");
        let config = PathBuf::from("/home/tester/.config");
        let explicit = PathBuf::from("/home/tester/.config/pony-agent");

        let homes = resolve_global_agent_homes_from(Some(&explicit), Some(&config), Some(&home));
        assert_eq!(
            homes,
            vec![
                home.join(".agents"),
                home.join(".claude"),
                config.join("pony-agent"),
            ],
            "显式覆盖目录与产品根重合时只保留一次"
        );

        let from_home_only = resolve_global_agent_homes_from(None, None, Some(&home));
        assert_eq!(from_home_only.len(), 2);
        assert!(from_home_only[0].ends_with(".agents"));
        assert!(from_home_only[1].ends_with(".claude"));

        assert!(resolve_global_agent_homes_from(None, None, None).is_empty());
        assert!(
            resolve_global_agent_homes_from(Some(Path::new("")), None, None).is_empty(),
            "空环境变量不得产生兜底目录"
        );
    }

    #[test]
    fn test_dsh_global_root_rank_is_reachable() {
        // 声明表（GlobalRootIdentity 文档）写了 `~/.dsh/skills`=500；本用例锁死该分支可达，
        // 否则「声明与实现不一致」会再次出现（PA-173 / ISSUE-20261111-007）。
        let identity = GlobalRootIdentity {
            env_home: None,
            home_dir: Some(PathBuf::from("/home/tester")),
            known: Vec::new(),
        };
        assert_eq!(identity.rank(Path::new("/home/tester/.dsh"), 0), 500);
        assert_eq!(identity.rank(Path::new("/home/tester/.agents"), 0), 510);
        assert_eq!(identity.rank(Path::new("/home/tester/.claude"), 0), 520);
        // 自定义技能根（无身份）落 450+ 兜底段：优于 `~/.dsh`、劣于产品自有根。
        assert_eq!(identity.rank(Path::new("/opt/skills-a"), 0), 450);
        assert_eq!(identity.rank(Path::new("/opt/skills-b"), 1), 451);
    }

    #[test]
    fn test_custom_skill_dirs_parse_and_dedupe() {
        let raw = std::env::join_paths(["/a", "/a", "", "/b"]).unwrap();
        assert_eq!(
            resolve_custom_skill_dirs_from(Some(raw.as_os_str())),
            vec![PathBuf::from("/a"), PathBuf::from("/b")],
            "空项跳过、重复项按首次出现去重且保序"
        );
        assert!(resolve_custom_skill_dirs_from(None).is_empty());
    }

    #[test]
    fn test_global_instructions_precede_project_instructions() {
        let workspace = unique_dir("pony_global_ws");
        let global_home = unique_dir("pony_global_home");
        fs::write(global_home.join("AGENTS.md"), "# Global Rules\nBe terse.").unwrap();
        fs::write(workspace.join("AGENTS.md"), "# Project Rules\nUse Rust.").unwrap();

        let hotloader = WorkspaceHotloader::new();
        let (content, hit) = hotloader
            .resolve_constitution_with_globals(&workspace, &[global_home.clone()])
            .unwrap();

        assert!(!hit);
        assert!(
            content.contains("# Global Rules\nBe terse."),
            "全局层内容必须出现"
        );
        assert!(
            content.contains("# Project Rules\nUse Rust."),
            "项目层内容必须出现"
        );
        assert!(content.contains("Global user-level instructions"));
        assert!(content.contains("Project instructions"));
        assert!(
            content.find("Global user-level instructions").unwrap()
                < content.find("Project instructions").unwrap(),
            "全局层必须排在项目层之前（优先级更低）"
        );
        assert!(
            content.find("# Global Rules").unwrap() < content.find("# Project Rules").unwrap(),
            "项目层内容必须位于全局层内容之后"
        );

        let (cached, cached_hit) = hotloader
            .resolve_constitution_with_globals(&workspace, &[global_home.clone()])
            .unwrap();
        assert!(cached_hit);
        assert_eq!(cached, content);

        fs::remove_dir_all(&workspace).ok();
        fs::remove_dir_all(&global_home).ok();
    }

    #[test]
    fn test_global_only_instructions_and_global_cache_invalidation() {
        let workspace = unique_dir("pony_global_only_ws");
        let global_home = unique_dir("pony_global_only_home");
        let global_md = global_home.join("AGENTS.md");
        fs::write(&global_md, "# User Rules v1").unwrap();

        let hotloader = WorkspaceHotloader::new();
        let (content, hit) = hotloader
            .resolve_constitution_with_globals(&workspace, &[global_home.clone()])
            .unwrap();
        assert!(!hit);
        assert!(content.contains("# User Rules v1"));
        assert!(content.contains("Global user-level instructions"));

        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&global_md, "# User Rules v2").unwrap();
        let (content2, hit2) = hotloader
            .resolve_constitution_with_globals(&workspace, &[global_home.clone()])
            .unwrap();
        assert!(!hit2, "全局层文件变更必须使缓存失效");
        assert!(content2.contains("# User Rules v2"));
        assert!(!content2.contains("# User Rules v1"));

        fs::remove_dir_all(&workspace).ok();
        fs::remove_dir_all(&global_home).ok();
    }

    #[test]
    fn test_claude_compat_instruction_candidate_is_honored() {
        let workspace = unique_dir("pony_claude_ws");
        let global_home = unique_dir("pony_claude_home");
        fs::create_dir_all(global_home.join(".claude")).unwrap();
        fs::write(global_home.join(".claude/CLAUDE.md"), "# Claude Global").unwrap();

        let hotloader = WorkspaceHotloader::new();
        let (content, _) = hotloader
            .resolve_constitution_with_globals(&workspace, &[global_home.clone()])
            .unwrap();
        assert!(content.contains("# Claude Global"));
        assert!(content.contains(".claude/CLAUDE.md"));

        fs::remove_dir_all(&workspace).ok();
        fs::remove_dir_all(&global_home).ok();
    }

    #[test]
    fn test_scan_skills_merges_global_and_project_layers() {
        let workspace = unique_dir("pony_skill_ws");
        let global_home = unique_dir("pony_skill_home");
        write_skill(&global_home, "global-one", "global-one", "A global skill");
        let project_skill_dir = workspace.join(".agents").join("skills");
        fs::create_dir_all(project_skill_dir.join("project-one")).unwrap();
        fs::write(
            project_skill_dir.join("project-one").join("SKILL.md"),
            "---\nname: project-one\ndescription: A project skill\n---\n\n# Body\n",
        )
        .unwrap();

        let hotloader = WorkspaceHotloader::new();
        let (skills, hit) = hotloader.scan_skills_with_globals(&workspace, &[global_home.clone()]);

        assert!(!hit);
        assert_eq!(skills.len(), 2);
        let by_name = |target: &str| {
            skills
                .iter()
                .find(|skill| skill.name == target)
                .unwrap_or_else(|| panic!("missing skill {target}"))
        };
        assert_eq!(by_name("global-one").scope, AgentScope::Global);
        assert_eq!(by_name("project-one").scope, AgentScope::Project);

        let (again, hit_again) =
            hotloader.scan_skills_with_globals(&workspace, &[global_home.clone()]);
        assert!(hit_again, "未变更时第二次扫描必须命中缓存");
        assert_eq!(again, skills);

        fs::remove_dir_all(&workspace).ok();
        fs::remove_dir_all(&global_home).ok();
    }

    #[test]
    fn test_project_skill_overrides_global_skill_with_same_folder_name() {
        let workspace = unique_dir("pony_override_ws");
        let global_home = unique_dir("pony_override_home");
        write_skill(&global_home, "review", "review", "Global review skill");
        let project_skill_dir = workspace.join(".agents").join("skills").join("review");
        fs::create_dir_all(&project_skill_dir).unwrap();
        fs::write(
            project_skill_dir.join("SKILL.md"),
            "---\nname: review\ndescription: Project review skill\n---\n\n# Body\n",
        )
        .unwrap();

        let hotloader = WorkspaceHotloader::new();
        let (skills, _) = hotloader.scan_skills_with_globals(&workspace, &[global_home.clone()]);

        assert_eq!(skills.len(), 1, "同名技能只保留一个");
        assert_eq!(skills[0].description, "Project review skill");
        assert_eq!(skills[0].scope, AgentScope::Project);

        fs::remove_dir_all(&workspace).ok();
        fs::remove_dir_all(&global_home).ok();
    }

    #[test]
    fn test_prompt_prefix_labels_skill_scope() {
        let hotloader = WorkspaceHotloader::new();
        let prefix = hotloader.build_cached_prompt_prefix(
            Some("rules"),
            &[HotloadedSkill {
                name: "audit".to_string(),
                description: "audit things".to_string(),
                user_invocable: true,
                allowed_tools: Vec::new(),
                path: PathBuf::from("/tmp/skills/audit/SKILL.md"),
                fingerprint: FileFingerprint {
                    path: PathBuf::from("/tmp/skills/audit/SKILL.md"),
                    mtime: None,
                    file_size: 10,
                    content_hash: 7,
                },
                scope: AgentScope::Global,
                rank: 0,
                disable_model_invocation: false,
                model_invocable: true,
            }],
        );
        assert!(prefix.contains("- `audit` [global]: audit things"));
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
        let (name, desc, user_invocable, tools, disable_model_invocation, model_invocable) =
            parse_skill_metadata(sample, "fallback");
        assert_eq!(name, "awesome-skill");
        assert_eq!(desc, "Comprehensive auditing tool for code.");
        assert!(!user_invocable);
        assert_eq!(tools, vec!["Bash", "Read"]);
        assert!(!disable_model_invocation);
        assert!(model_invocable);
    }

    #[test]
    fn test_session_kv_cache_preservation_on_mid_turn_edit() {
        let temp_dir = unique_dir("pony_session");

        let agents_md = temp_dir.join("AGENTS.md");
        fs::write(&agents_md, "# Baseline Rules").unwrap();

        let hotloader = WorkspaceHotloader::new();
        let session_id = "session_123";

        // 第 1 轮交互：创建基线
        let (prefix_turn_1, notice_turn_1) =
            hotloader.resolve_session_prompt_context_with_globals(session_id, 1, &temp_dir, &[]);
        assert!(prefix_turn_1.contains("# Baseline Rules"));
        assert!(notice_turn_1.is_none());

        // 用户在第 2 轮中途修改了 AGENTS.md 宪法文件
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&agents_md, "# Modified Mid-Session Rules").unwrap();

        // 第 2 轮交互：静态前缀必须保持不变（KV-Cache 命中保护！），变动以增量 notice 输出
        let (prefix_turn_2, notice_turn_2) =
            hotloader.resolve_session_prompt_context_with_globals(session_id, 2, &temp_dir, &[]);
        assert_eq!(
            prefix_turn_1, prefix_turn_2,
            "头部静态前缀必须严格相等，确保 KV Cache 不被冲刷！"
        );
        assert!(notice_turn_2.is_some());
        assert!(notice_turn_2
            .unwrap()
            .contains("constitution/rules updated"));

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    /// C6（L3-B #11）：基线未命中通知**只进结构化日志**，不得注入模型可见请求体。
    ///
    /// 反例（修正前）：进程重启后 `turn_count > 1` 会把
    /// `Notice: Session baseline was not retained …` 作为 `delta_notice` 返回，由
    /// `context.rs` 追加进 volatile 段 ⇒ 同一份落盘会话两次恢复的请求体不逐字相同。
    #[test]
    fn test_session_baseline_miss_never_injects_prompt_notice() {
        let temp_dir = unique_dir("pony_session_baseline_notice");
        fs::write(temp_dir.join("AGENTS.md"), "# Baseline Rules").unwrap();

        // 第 1 轮：固化基线，无通知。
        let hotloader = WorkspaceHotloader::new();
        let (_, notice_turn_1) = hotloader.resolve_session_prompt_context_with_globals(
            "session_baseline_notice",
            1,
            &temp_dir,
            &[],
        );
        assert!(notice_turn_1.is_none(), "首轮固化基线不产生通知");

        // "进程重启"：全新 hotloader（进程内基线缓存为空）+ turn_count > 1。
        let restarted = WorkspaceHotloader::new();
        let (prefix_turn_2, notice_turn_2) = restarted
            .resolve_session_prompt_context_with_globals(
                "session_baseline_notice",
                2,
                &temp_dir,
                &[],
            );
        assert!(
            notice_turn_2.is_none(),
            "基线未命中通知不得注入请求体（C6 幂等），实际 = {notice_turn_2:?}"
        );
        // 前缀仍按当前事实重新固化（通知移除不等于能力丢失）。
        assert!(prefix_turn_2.contains("# Baseline Rules"));

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    // ============================================================
    // B142 / item-5：宪法预算裁决的结构化报告（ConstitutionBudgetReport）
    // ============================================================

    /// 构造一层合成宪法（不碰文件系统；预算渲染只依赖内容与快照的 oversized 标记）。
    fn synthetic_layer(scope: AgentScope, name: &str, content: &str) -> ConstitutionLayer {
        ConstitutionLayer {
            scope,
            source_path: PathBuf::from(name),
            content: content.to_string(),
            fingerprint: FileFingerprint {
                path: PathBuf::from(name),
                mtime: None,
                file_size: content.len() as u64,
                content_hash: fnv1a_hash(content.as_bytes()),
            },
        }
    }

    /// 构造与层一一对应的来源快照（`oversized` 由调用方指定）。
    fn synthetic_snapshot(
        scope: AgentScope,
        name: &str,
        content: &str,
        oversized: bool,
    ) -> ConstitutionSourceSnapshot {
        ConstitutionSourceSnapshot {
            scope,
            path: PathBuf::from(name),
            mtime: None,
            file_size: content.len() as u64,
            read_ok: true,
            oversized,
            content: content.to_string(),
            content_hash: fnv1a_hash(content.as_bytes()),
        }
    }

    /// 默认预算、无重复：报告必须为空（不得凭空产生省略记录）。
    #[test]
    fn constitution_report_is_empty_when_nothing_is_decided() {
        let layers = vec![
            synthetic_layer(AgentScope::Global, "global.md", "# Global\nrule A"),
            synthetic_layer(AgentScope::Project, "project.md", "# Project\nrule B"),
        ];
        let snapshots = vec![
            synthetic_snapshot(AgentScope::Global, "global.md", "# Global\nrule A", false),
            synthetic_snapshot(AgentScope::Project, "project.md", "# Project\nrule B", false),
        ];

        let (text, report) =
            render_constitution_with_budget(&snapshots, layers, InstructionBudget::default().max_bytes);
        assert!(text.contains("rule A") && text.contains("rule B"));
        assert!(
            report.is_empty(),
            "默认预算下不应有任何省略/截断记录，实际：{report:?}"
        );
    }

    /// 跨层同文去重必须被结构化记账（历史行为不产生文本标记 → 观测面曾完全不可见）。
    #[test]
    fn constitution_report_records_duplicate_layers() {
        let same = "# Rules\nsame body";
        let layers = vec![
            synthetic_layer(AgentScope::Global, "global.md", same),
            synthetic_layer(AgentScope::Project, "project.md", same),
        ];
        let snapshots = vec![
            synthetic_snapshot(AgentScope::Global, "global.md", same, false),
            synthetic_snapshot(AgentScope::Project, "project.md", same, false),
        ];

        let (text, report) =
            render_constitution_with_budget(&snapshots, layers, InstructionBudget::default().max_bytes);
        assert_eq!(text.matches("same body").count(), 1, "同文只应保留一份");
        assert_eq!(
            report.omitted,
            vec![OmittedConstitutionLayer {
                path: PathBuf::from("global.md"),
                reason: ConstitutionOmissionReason::DuplicateContent,
            }],
            "被去重丢弃的层必须记账（且只记被丢弃的那一层）"
        );
        assert!(report.truncated.is_none() && !report.clamped);
    }

    /// 预算不足时逐层省略必须记账为 `BudgetExceeded`，且文本严格不超预算。
    #[test]
    fn constitution_report_records_budget_omissions() {
        let long_a = format!("# A\n{}", "a".repeat(200));
        let long_b = format!("# B\n{}", "b".repeat(200));
        let layers = vec![
            synthetic_layer(AgentScope::Global, "global.md", &long_a),
            synthetic_layer(AgentScope::Project, "project.md", &long_b),
        ];
        let snapshots = vec![
            synthetic_snapshot(AgentScope::Global, "global.md", &long_a, false),
            synthetic_snapshot(AgentScope::Project, "project.md", &long_b, false),
        ];

        let budget_bytes = 120usize;
        let (text, report) = render_constitution_with_budget(&snapshots, layers, budget_bytes);
        assert!(
            text.len() <= budget_bytes,
            "渲染文本必须严格落在预算内：{} > {budget_bytes}",
            text.len()
        );
        assert!(
            report
                .omitted
                .iter()
                .any(|entry| entry.reason == ConstitutionOmissionReason::BudgetExceeded),
            "超预算被省略的层必须记账为 budget_exceeded，实际：{report:?}"
        );
        assert!(
            text.contains("omitted "),
            "模型可见文本仍须保留既有的 omitted 标记（报告不改行为）"
        );
    }

    /// 最高优先级层超限时记录截断账：原始字节、保留字节与路径可核对。
    #[test]
    fn constitution_report_records_top_layer_truncation() {
        let long = format!("# Project\n{}", "x".repeat(600));
        let layers = vec![synthetic_layer(AgentScope::Project, "project.md", &long)];
        let snapshots = vec![synthetic_snapshot(
            AgentScope::Project,
            "project.md",
            &long,
            true,
        )];

        let budget_bytes = 200usize;
        let (text, report) = render_constitution_with_budget(&snapshots, layers, budget_bytes);
        assert!(text.len() <= budget_bytes, "渲染文本必须落在预算内");
        let truncated = report
            .truncated
            .expect("最高优先级层放不下时必须产生截断记录");
        assert_eq!(truncated.path, PathBuf::from("project.md"));
        assert_eq!(truncated.original_bytes, long.len());
        assert!(
            truncated.included_bytes < truncated.original_bytes,
            "保留字节必须小于原始字节（{} vs {}）",
            truncated.included_bytes,
            truncated.original_bytes
        );
        assert!(text.contains("truncated project.md"), "模型可见文本仍须保留截断标记");
    }
}
