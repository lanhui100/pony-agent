//! 大输出落盘溢出（W26）。
//!
//! 背景：审计（`docs/audit/agent-core-gap-matrix.md` P0-4 / A09）指出本项目遇到大输出只有两种
//! 结局——按字节上限截断（内容永久丢失），或直接硬失败（`file_too_large`），模型只能反复
//! 重读/重跑命令。对比基线（claude-code `utils/toolResultStorage.ts`、DSH `spill-policy`）都是
//! 「落盘 + 返回引用 + 按需回读」。
//!
//! 本模块提供内容寻址的落盘溢出：
//! - [`spill_bytes`]：把超限内容写到落盘根目录下的 `<sha256>.spill`，返回 [`SpillRef`]；
//! - [`read_spill`]：按 `ref_id` + 偏移/长度回读，返回 [`SpillSlice`]（含真实起止偏移，供调用方
//!   精确判定是否读到文件尾）；
//! - 落盘根目录默认 `<temp>/pony-agent-spill`，可用环境变量 `PONY_AGENT_SPILL_DIR` 覆盖。
//!
//! 安全边界：`ref_id` 必须是 64 位小写十六进制（内容哈希），读写前强校验，杜绝 `..`/绝对路径
//! 穿越；落盘文件只允许位于根目录内。

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// 回读默认长度（字节上限）。
pub const DEFAULT_READ_LIMIT: usize = 8_000;
/// 回读允许的最大长度，防止模型一次性把巨型文件拉回上下文。
pub const MAX_READ_LIMIT: usize = 200_000;
/// 随结果返回的预览字节上限（按字节截断并回退到字符边界）。
pub const PREVIEW_BYTES: usize = 3_000;
/// 允许落盘的最大内容体积（超出则仍然硬失败，避免无界磁盘占用）。
pub const MAX_SPILL_BYTES: u64 = 64 * 1024 * 1024;

/// 落盘溢出相关错误（结构化，避免调用方靠中文子串匹配错误类型）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpillError {
    /// `ref_id` 非法（非 64 位小写十六进制 / 穿越尝试）。
    InvalidRef(String),
    /// 引用不存在（可能已被清理）。
    NotFound(String),
    /// 内容超过落盘上限。
    TooLarge(String),
    /// 落盘 IO 失败。
    Io(String),
}

impl SpillError {
    /// 稳定的机器可读错误码。
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRef(_) => "invalid_ref",
            Self::NotFound(_) => "ref_not_found",
            Self::TooLarge(_) => "file_too_large",
            Self::Io(_) => "spill_io_failed",
        }
    }

    /// 面向模型的中文说明。
    pub fn message(&self) -> &str {
        match self {
            Self::InvalidRef(msg) | Self::NotFound(msg) | Self::TooLarge(msg) | Self::Io(msg) => msg,
        }
    }
}

/// 一次落盘溢出的引用描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpillRef {
    /// 内容哈希（sha256 小写十六进制，64 字符），同时作为回读句柄。
    pub ref_id: String,
    /// 落盘文件绝对路径（仅用于排障与审计，模型侧优先使用 `ref_id`）。
    pub path: PathBuf,
    /// 内容字节数。
    pub bytes: u64,
    /// 头部预览（按字节上限截断，且不切断多字节字符）。
    pub preview: String,
}

/// 一次回读的切片结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpillSlice {
    /// 切片内容（UTF-8 安全）。
    pub content: String,
    /// 文件总字节数。
    pub total_bytes: u64,
    /// 实际起始字节偏移（已回退到字符边界，可能 < 请求的 offset）。
    pub start: u64,
    /// 实际结束字节偏移（不含）。
    pub end: u64,
    /// 实际生效的读取上限（已 clamp 到 [`MAX_READ_LIMIT`]）。
    pub limit: usize,
}

impl SpillSlice {
    /// 是否还有未读内容（真实判定，不依赖调用方重算偏移）。
    pub fn truncated(&self) -> bool {
        self.end < self.total_bytes
    }
}

/// 落盘根目录：`PONY_AGENT_SPILL_DIR` 优先，否则 `<temp>/pony-agent-spill`。
pub fn spill_root() -> PathBuf {
    match std::env::var("PONY_AGENT_SPILL_DIR") {
        Ok(value) if !value.trim().is_empty() => PathBuf::from(value),
        _ => std::env::temp_dir().join("pony-agent-spill"),
    }
}

/// `ref_id` 合法性：必须是小写十六进制的 sha256（64 字符）。
///
/// 这条校验同时是路径穿越的防线：任何 `/`、`\`、`.`、`..`、首尾空白、大写十六进制都会被拒绝。
pub fn is_valid_ref_id(candidate: &str) -> bool {
    candidate.len() == 64
        && candidate
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// 计算内容哈希。
pub fn content_ref_id(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    format!("{:x}", hasher.finalize())
}

/// 按字节上限做 UTF-8 安全截断（不切断多字节字符）。
pub fn preview_of(content: &str, max_bytes: usize) -> String {
    if content.len() <= max_bytes {
        return content.to_string();
    }
    let mut out = String::with_capacity(max_bytes.min(1_024));
    for ch in content.chars() {
        if out.len() + ch.len_utf8() > max_bytes {
            break;
        }
        out.push(ch);
    }
    out
}

/// 落盘 GC 的默认年龄上限（超过即回收）。
pub const GC_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
/// 落盘 GC 的默认总量上限。
pub const GC_MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
/// 机会式 GC 的最小间隔。
const GC_MIN_INTERVAL_MS: u64 = 5 * 60 * 1000;

static LAST_GC_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 收紧落盘目录/文件权限（Unix 下目录 0700、文件 0600；其他平台 no-op）。
fn tighten_permissions(path: &Path, is_dir: bool) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if is_dir { 0o700 } else { 0o600 };
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    {
        let _ = (path, is_dir);
    }
}

/// 机会式 GC（按 `GC_MIN_INTERVAL_MS` 限频），失败静默（不影响主流程）。
pub fn maybe_gc(root: &Path) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let last = LAST_GC_MS.load(std::sync::atomic::Ordering::Relaxed);
    if now_ms.saturating_sub(last) < GC_MIN_INTERVAL_MS {
        return;
    }
    if LAST_GC_MS
        .compare_exchange(
            last,
            now_ms,
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
        )
        .is_err()
    {
        return;
    }
    let _ = gc_spill_root(root, GC_MAX_AGE, GC_MAX_TOTAL_BYTES);
}

fn io_error(context: &str, err: std::io::Error) -> SpillError {
    SpillError::Io(format!("{context}：{err}。"))
}

/// 把内容落盘到指定根目录并返回引用（测试与显式根目录场景使用）。
pub fn spill_bytes_to(root: &Path, content: &[u8]) -> Result<SpillRef, SpillError> {
    if content.len() as u64 > MAX_SPILL_BYTES {
        return Err(SpillError::TooLarge(format!(
            "内容大小 {} bytes 超过落盘溢出上限 {} bytes。",
            content.len(),
            MAX_SPILL_BYTES
        )));
    }

    let root = root.to_path_buf();
    fs::create_dir_all(&root)
        .map_err(|err| io_error(&format!("创建落盘目录 {} 失败", root.display()), err))?;
    tighten_permissions(&root, true);

    let ref_id = content_ref_id(content);
    let path = root.join(format!("{ref_id}.spill"));

    if !path.exists() {
        // 先写唯一临时文件再 rename，避免读到半截内容；tmp 名带 pid + 随机后缀防并发冲突。
        let tmp = root.join(format!(
            "{ref_id}.{}.{}.tmp",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::write(&tmp, content)
            .map_err(|err| io_error(&format!("写入落盘文件 {} 失败", tmp.display()), err))?;
        tighten_permissions(&tmp, false);
        match fs::rename(&tmp, &path) {
            Ok(()) => {}
            // 并发下另一个写者可能已抢先 rename 成功：目标存在即视为成功。
            Err(_) if path.exists() => {
                let _ = fs::remove_file(&tmp);
            }
            Err(err) => {
                let _ = fs::remove_file(&tmp);
                return Err(io_error(
                    &format!("落盘文件改名 {} 失败", path.display()),
                    err,
                ));
            }
        }
    }

    // 机会式回收：每 5 分钟最多一次，按年龄/总量清理陈旧落盘（避免 temp 目录长期堆积）。
    maybe_gc(&root);

    let preview_source = String::from_utf8_lossy(content);
    Ok(SpillRef {
        ref_id,
        path,
        bytes: content.len() as u64,
        preview: preview_of(&preview_source, PREVIEW_BYTES),
    })
}

/// 解析 `ref_id` 在指定根目录下的落盘路径（强校验，拒绝任何非法/穿越输入）。
pub fn resolve_ref_path_in(root: &Path, ref_id: &str) -> Result<PathBuf, SpillError> {
    if !is_valid_ref_id(ref_id) {
        return Err(SpillError::InvalidRef(format!(
            "非法 content_ref：`{ref_id}`（要求 64 位小写十六进制内容哈希）。"
        )));
    }
    let root = root.to_path_buf();
    let path = root.join(format!("{ref_id}.spill"));
    if path.parent() != Some(root.as_path()) {
        return Err(SpillError::InvalidRef(
            "非法 content_ref：解析路径越出落盘根目录。".to_string(),
        ));
    }
    Ok(path)
}

/// 解析 `ref_id` 对应的落盘路径（使用默认落盘根目录）。
pub fn resolve_ref_path(ref_id: &str) -> Result<PathBuf, SpillError> {
    resolve_ref_path_in(&spill_root(), ref_id)
}

/// 把内容落盘并返回引用（默认落盘根目录）；内容寻址天然去重（同内容只写一次）。
pub fn spill_bytes(content: &[u8]) -> Result<SpillRef, SpillError> {
    spill_bytes_to(&spill_root(), content)
}

/// 按引用从指定根目录回读切片（测试/显式根目录使用）。
///
/// - `limit` 被 clamp 到 [`MAX_READ_LIMIT`]，`limit == 0` 返回空切片（绝不退化为整篇回读）；
/// - `offset` 落在多字节字符中间时回退到最近字符边界；
/// - 区间尾部同样回退到字符边界，且**不会**因回退而越过 `limit`。
pub fn read_spill_from(
    root: &Path,
    ref_id: &str,
    offset: usize,
    limit: usize,
) -> Result<SpillSlice, SpillError> {
    let path = resolve_ref_path_in(root, ref_id)?;
    if !path.is_file() {
        return Err(SpillError::NotFound(format!(
            "content_ref `{ref_id}` 不存在（可能已过期或被清理）。"
        )));
    }

    // 符号链接防护：解析真实路径后必须仍落在落盘根目录内、且文件名与 ref_id 一致，
    // 否则拒绝（`ref_id` 是内容哈希，指向别的文件即内容与句柄不符）。
    let canonical_root = fs::canonicalize(root).map_err(|_| {
        SpillError::NotFound(format!(
            "落盘根目录 {} 不可用（content_ref `{ref_id}` 无法解析）。",
            root.display()
        ))
    })?;
    let canonical_path = fs::canonicalize(&path).map_err(|_| {
        SpillError::NotFound(format!(
            "content_ref `{ref_id}` 不存在（可能已过期或被清理）。"
        ))
    })?;
    let expected_name = format!("{ref_id}.spill");
    if canonical_path.parent() != Some(canonical_root.as_path())
        || canonical_path.file_name().and_then(|name| name.to_str())
            != Some(expected_name.as_str())
    {
        return Err(SpillError::InvalidRef(format!(
            "content_ref `{ref_id}` 解析到的真实路径越出落盘根目录或与句柄不符（疑似符号链接替换）。"
        )));
    }

    let bytes = fs::read(&canonical_path)
        .map_err(|err| io_error(&format!("读取落盘文件 {} 失败", canonical_path.display()), err))?;
    let total = bytes.len();

    let limit = limit.min(MAX_READ_LIMIT);
    if limit == 0 {
        let at = offset.min(total);
        return Ok(SpillSlice {
            content: String::new(),
            total_bytes: total as u64,
            start: at as u64,
            end: at as u64,
            limit: 0,
        });
    }

    let mut start = offset.min(total);
    while start > 0 && start < total && (bytes[start] & 0b1100_0000) == 0b1000_0000 {
        start -= 1;
    }

    // 硬上限：start + limit，再回退到字符边界（只后退，绝不扩大）。
    let mut end = (start + limit).min(total);
    while end > start && end < total && (bytes[end] & 0b1100_0000) == 0b1000_0000 {
        end -= 1;
    }
    if end < start {
        end = start;
    }

    let content = String::from_utf8_lossy(&bytes[start..end]).to_string();
    Ok(SpillSlice {
        content,
        total_bytes: total as u64,
        start: start as u64,
        end: end as u64,
        limit,
    })
}

/// 按引用回读切片（默认落盘根目录）。
pub fn read_spill(ref_id: &str, offset: usize, limit: usize) -> Result<SpillSlice, SpillError> {
    read_spill_from(&spill_root(), ref_id, offset, limit)
}

/// 落盘目录清理入口（W26 参考基线 DSH spill：按年龄与总量回收）。
///
/// - 删除 `mtime` 早于 `max_age` 的 `.spill`/`.tmp` 文件；
/// - 若剩余总量超过 `max_total_bytes`，按 `mtime` 从旧到新继续删除直到达标。
///
/// 返回删除的文件数。调用方（会话收尾/启动自检）可按需调度；本波不自动触发，
/// 以避免在未定义生命周期策略前误删仍在被引用的内容。
pub fn gc_spill_root(
    root: &Path,
    max_age: std::time::Duration,
    max_total_bytes: u64,
) -> Result<u64, SpillError> {
    if !root.is_dir() {
        return Ok(0);
    }

    let now = std::time::SystemTime::now();
    let mut candidates: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
    let entries = fs::read_dir(root)
        .map_err(|err| io_error(&format!("列出落盘目录 {} 失败", root.display()), err))?;

    for entry in entries.flatten() {
        let path = entry.path();
        let is_spill = path
            .extension()
            .is_some_and(|ext| ext == "spill" || ext == "tmp");
        if !is_spill {
            continue;
        }
        let Ok(metadata) = entry.metadata() else { continue };
        let modified = metadata.modified().unwrap_or(now);
        candidates.push((modified, metadata.len(), path));
    }

    let mut removed = 0u64;
    let mut kept: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();

    for (modified, size, path) in candidates {
        let age = now.duration_since(modified).unwrap_or_default();
        if age >= max_age {
            if fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        } else {
            kept.push((modified, size, path));
        }
    }

    let mut total: u64 = kept.iter().map(|(_, size, _)| *size).sum();
    if total > max_total_bytes {
        kept.sort_by_key(|(modified, _, _)| *modified);
        for (_, size, path) in kept {
            if total <= max_total_bytes {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                removed += 1;
                total = total.saturating_sub(size);
            }
        }
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "pony-spill-unit-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn spill_is_content_addressed_and_readable() {
        let root = temp_root();
        let payload = "a".repeat(50_000);
        let stored = spill_bytes_to(&root, payload.as_bytes()).expect("spill should succeed");
        assert_eq!(stored.bytes, 50_000);
        assert!(is_valid_ref_id(&stored.ref_id));
        assert_eq!(stored.ref_id, content_ref_id(payload.as_bytes()));
        assert!(stored.path.starts_with(&root));
        assert!(!stored.preview.is_empty());
        assert!(stored.preview.len() <= PREVIEW_BYTES);

        let slice = read_spill_from(&root, &stored.ref_id, 0, 8_000).expect("read");
        assert_eq!(slice.total_bytes, 50_000);
        assert_eq!(slice.content.len(), 8_000);
        assert!(slice.truncated());

        let all = read_spill_from(&root, &stored.ref_id, 0, MAX_READ_LIMIT).expect("read all");
        assert_eq!(all.content.len(), 50_000);
        assert!(!all.truncated());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn same_content_dedupes_to_one_file() {
        let root = temp_root();
        let first = spill_bytes_to(&root, b"duplicate payload").expect("first spill");
        let second = spill_bytes_to(&root, b"duplicate payload").expect("second spill");
        assert_eq!(first.ref_id, second.ref_id);
        let count = fs::read_dir(&root)
            .expect("read dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "spill"))
            .count();
        assert_eq!(count, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_traversal_and_malformed_refs() {
        for bad in [
            "../etc/passwd",
            "/etc/passwd",
            "..",
            "",
            &"a".repeat(63),
            &"A".repeat(64),
        ] {
            assert!(!is_valid_ref_id(bad), "`{bad}` 应被判非法");
            assert_eq!(
                resolve_ref_path_in(Path::new("/tmp"), bad)
                    .expect_err("must reject")
                    .code(),
                "invalid_ref"
            );
        }
    }

    #[test]
    fn missing_but_wellformed_ref_is_not_found() {
        let root = temp_root();
        let err = read_spill_from(&root, &"0".repeat(64), 0, 10).expect_err("must fail");
        assert_eq!(err.code(), "ref_not_found");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preview_respects_byte_cap_without_splitting_chars() {
        let content = "中文内容".repeat(1_000);
        let preview = preview_of(&content, 9);
        assert_eq!(preview, "中文内");
        assert!(preview.len() <= 9, "预览必须按字节受控");
    }

    #[test]
    fn limit_is_hard_capped_and_zero_limit_returns_empty() {
        let root = temp_root();
        let payload = "a".repeat(50_000);
        let stored = spill_bytes_to(&root, payload.as_bytes()).expect("spill");

        let zero = read_spill_from(&root, &stored.ref_id, 0, 0).expect("limit=0 read");
        assert!(zero.content.is_empty(), "limit=0 必须返回空切片");

        let one = read_spill_from(&root, &stored.ref_id, 0, 1).expect("limit=1 read");
        assert_eq!(one.content.len(), 1, "limit=1 只允许 1 字节，禁止退化为整篇");

        let huge = read_spill_from(&root, &stored.ref_id, 0, MAX_READ_LIMIT * 10).expect("huge");
        assert_eq!(huge.content.len(), 50_000);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn truncated_reflects_real_end_after_char_boundary_retreat() {
        let root = temp_root();
        // 多字节内容：确保尾部回退发生时 truncated 仍判定为真。
        let payload = format!("X😀{}é", "a".repeat(200));
        let stored = spill_bytes_to(&root, payload.as_bytes()).expect("spill");
        let slice = read_spill_from(&root, &stored.ref_id, 4, 195).expect("read");
        assert!(
            slice.truncated(),
            "尾部未读完时必须判定 truncated=true（start={} end={} total={}）",
            slice.start,
            slice.end,
            slice.total_bytes
        );
        assert!(slice.end <= slice.start + 195);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn offset_beyond_end_returns_empty_without_panic() {
        let root = temp_root();
        let stored = spill_bytes_to(&root, b"short payload").expect("spill");
        let slice =
            read_spill_from(&root, &stored.ref_id, 10_000, 8_000).expect("offset beyond EOF");
        assert!(slice.content.is_empty(), "越界 offset 应返回空切片");
        assert_eq!(slice.total_bytes, "short payload".len() as u64);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn gc_removes_expired_and_over_cap_files() {
        let root = temp_root();
        let a = spill_bytes_to(&root, b"first content").expect("spill a");
        let _b = spill_bytes_to(&root, b"second content").expect("spill b");

        // max_age = 0：全部视为过期 → 全部删除。
        let removed = gc_spill_root(&root, std::time::Duration::from_secs(0), u64::MAX)
            .expect("gc by age");
        assert_eq!(removed, 2, "过期文件应被全部清理");
        assert!(!a.path.exists());
        let _ = fs::remove_dir_all(&root);

        // 总量上限：max_age 足够大（不按年龄删），cap=1 字节 → 至少删到不超过上限。
        let root2 = temp_root();
        let _c = spill_bytes_to(&root2, b"cccccccccc").expect("spill c");
        let _d = spill_bytes_to(&root2, b"dddddddddddd").expect("spill d");
        let removed2 = gc_spill_root(&root2, std::time::Duration::from_secs(3_600), 1)
            .expect("gc by cap");
        assert!(removed2 >= 1, "超总量上限时必须回收文件");
        let remaining: u64 = fs::read_dir(&root2)
            .expect("read dir")
            .filter_map(Result::ok)
            .filter_map(|entry| entry.metadata().ok())
            .map(|meta| meta.len())
            .sum();
        assert!(remaining <= 1, "回收后总量必须不超过上限：{remaining}");
        let _ = fs::remove_dir_all(root2);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_spill_file_is_rejected() {
        use std::os::unix::fs::symlink;

        let root = temp_root();
        let outside = temp_root(); // 根目录外的目标
        fs::create_dir_all(&root).expect("root");
        fs::create_dir_all(&outside).expect("outside");
        let victim = outside.join("victim.txt");
        fs::write(&victim, b"content that does not match the ref_id").expect("write victim");

        // 合法 ref_id（内容哈希）但用符号链接指向根外文件。
        let ref_id = content_ref_id(b"genuine content");
        symlink(&victim, root.join(format!("{ref_id}.spill"))).expect("symlink");

        let err = read_spill_from(&root, &ref_id, 0, 32).expect_err("symlink must be rejected");
        assert_eq!(err.code(), "invalid_ref", "符号链接替换必须拒绝：{err:?}");
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn offset_walks_back_to_char_boundary() {
        let root = temp_root();
        let payload = "中文".repeat(10); // 每个汉字 3 字节
        let stored = spill_bytes_to(&root, payload.as_bytes()).expect("spill");
        let slice = read_spill_from(&root, &stored.ref_id, 1, 6).expect("read");
        assert!(
            !slice.content.contains('\u{FFFD}'),
            "切片不得产生替换字符：{:?}",
            slice.content
        );
        let _ = fs::remove_dir_all(root);
    }
}
