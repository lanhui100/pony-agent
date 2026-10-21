//! 工具触达即载子目录指令（PA-152）：只读发现，按目录浅→深排序。

use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use super::workspace_hotloader::ancestor_chain;

/// 一条待注入的子目录指令。
pub struct TouchedInstruction {
    pub display_path: String,
    pub content: String,
}

/// 单步触达注入的字节上限。
pub const MAX_TOUCH_STEP_BYTES: usize = 16_384;

/// 每级目录的指令文件名，按优先级从高到低取首个命中。
const TOUCHED_INSTRUCTION_CANDIDATES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

/// 单条指令超限时追加的截断标记（内容尾部，保证 `content.len() <= MAX_TOUCH_STEP_BYTES`）。
const TOUCH_TRUNCATION_MARKER: &str = "\n[truncated]";

/// 发现 workspace_root→touched_path 父目录链上各级的指令文件（首命中），跳过已注入路径。
///
/// 语义（契约 C4）：
/// - touched 必须落在 workspace_root 内（词法 `..` 逃逸与绝对越界一律返回空）；
/// - 祖先链取 root→touched 父目录（含两端），浅→深排序；
/// - 每级按 `AGENTS.md` → `CLAUDE.md` 首命中；
/// - `already_injected` 保存**绝对路径**，命中的层直接跳过（会话级去重）；
/// - 单条超过 [`MAX_TOUCH_STEP_BYTES`] 时按 UTF-8 边界截断并追加 `[truncated]` 标记。
///
/// L3-B F1（安全）：每个候选指令文件都必须通过「canonical 后仍在 canonical(workspace_root) 内
/// + 元数据类型为普通文件」校验，否则跳过该级。这样 `sub/AGENTS.md -> /etc/passwd` 之类的
/// 符号链接逃逸、FIFO/设备文件都不会被读入注入结果；不满足只是跳过，不报错、不阻断工具。
pub fn instructions_for_touched_path(
    workspace_root: &Path,
    touched_path: &Path,
    already_injected: &BTreeSet<String>,
) -> Vec<TouchedInstruction> {
    // 词法归一（折叠 `.` / `..`，卷根之上钳制）后再做组件级边界判定：
    // 这样 `root/../outside/x` 这类词法逃逸与绝对越界都被拒绝，不会上溯到 workspace 外。
    let lexical_root = lexical_normalize(workspace_root);
    let lexical_touched = lexical_normalize(touched_path);
    if lexical_touched == lexical_root || !lexical_touched.starts_with(&lexical_root) {
        return Vec::new();
    }

    let touched_parent = lexical_touched
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| lexical_root.clone());

    // 安全基准：canonical(workspace_root)。取不到（不存在/不可解析）→ 宁可一级都不注入。
    let Ok(canonical_root) = workspace_root.canonicalize() else {
        return Vec::new();
    };
    let canonical_parent = match touched_parent.canonicalize() {
        Ok(parent) if parent == canonical_root || parent.starts_with(&canonical_root) => parent,
        // touched 父目录在 canonical 化后逃出 workspace（符号链接挂载点等）→ 不注入。
        _ => return Vec::new(),
    };

    let mut entries: Vec<TouchedInstruction> = Vec::new();
    for directory in ancestor_chain(&canonical_root, &canonical_parent) {
        let Some(instruction_file) = first_trusted_instruction(&directory, &canonical_root) else {
            continue;
        };
        let display_path = instruction_file.display().to_string();
        if already_injected.contains(&display_path) {
            continue;
        }
        // 读取失败（权限/编码/读取瞬间被替换）按"该级无指令"跳过，不打断工具调用主流程。
        // L3 r2 A3：先 metadata 预检 + 有界读取（take(MAX+1)），绝不整文件入内存。
        let Some(content) = read_instruction_bounded(&instruction_file) else {
            continue;
        };
        entries.push(TouchedInstruction {
            display_path,
            content,
        });
    }
    entries
}

/// 每级首个**安全**的指令文件（`AGENTS.md` → `CLAUDE.md`）。
///
/// 安全 = canonical 目标仍在 canonical root 内 + 目标是普通文件（跟随符号链接后判定）。
fn first_trusted_instruction(directory: &Path, canonical_root: &Path) -> Option<PathBuf> {
    for candidate in TOUCHED_INSTRUCTION_CANDIDATES {
        let path = directory.join(candidate);
        let Ok(canonical) = path.canonicalize() else {
            continue;
        };
        if !canonical.starts_with(canonical_root) {
            // 符号链接逃逸 / 挂载到 workspace 外：跳过该候选，绝不读取。
            continue;
        }
        match fs::metadata(&canonical) {
            Ok(metadata) if metadata.file_type().is_file() => return Some(canonical),
            // FIFO / 设备 / 目录：跳过。
            _ => continue,
        }
    }
    None
}

/// L3 r2 A3：有界读取单条指令文件。
///
/// 先 `metadata` 预检（普通文件 + 大小），再 `File::open` + `Read::take(MAX+1)` 最多读
/// `MAX_TOUCH_STEP_BYTES + 1` 字节；UTF-8 非法尾部回退到最近合法边界，最后按单步预算截断。
/// 512MiB 的 `AGENTS.md` 不会产生 ~1GiB 峰值。
fn read_instruction_bounded(path: &Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.file_type().is_file() {
        return None;
    }
    let file = fs::File::open(path).ok()?;
    let mut buffer: Vec<u8> = Vec::with_capacity(MAX_TOUCH_STEP_BYTES + 1);
    file.take(MAX_TOUCH_STEP_BYTES as u64 + 1)
        .read_to_end(&mut buffer)
        .ok()?;
    let text = match std::str::from_utf8(&buffer) {
        Ok(text) => text,
        Err(error) => std::str::from_utf8(&buffer[..error.valid_up_to()]).ok()?,
    };
    Some(truncate_to_step_budget(text))
}

/// 单步预算内的截断：超限时按 UTF-8 边界切到 `MAX - marker`，并在尾部追加截断标记。
fn truncate_to_step_budget(content: &str) -> String {
    if content.len() <= MAX_TOUCH_STEP_BYTES {
        return content.to_string();
    }
    let mut cut = MAX_TOUCH_STEP_BYTES.saturating_sub(TOUCH_TRUNCATION_MARKER.len());
    while cut > 0 && !content.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut truncated = String::with_capacity(cut + TOUCH_TRUNCATION_MARKER.len());
    truncated.push_str(&content[..cut]);
    truncated.push_str(TOUCH_TRUNCATION_MARKER);
    truncated
}

/// 词法归一：折叠 `.` / `..`，`..` 不越过卷根（绝对路径）或起点之前的相对前缀。
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match normalized.components().next_back() {
                // 卷根/前缀之上的 `..` 钳制在根上。
                Some(Component::RootDir) | Some(Component::Prefix(_)) => {}
                Some(Component::Normal(_)) => {
                    normalized.pop();
                }
                _ => normalized.push(".."),
            },
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}
