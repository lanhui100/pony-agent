//! W26「大输出落盘溢出（spill）」红相验收测试。
//!
//! 背景：`workspace_read_file` 对 >`MAX_FULL_READ_BYTES`（120_000 bytes）的文件直接返回
//! `file_too_large` 错误（`agent/tools.rs::read_file`），模型拿不到任何内容，只能猜测改用
//! `workspace_read_file_segment`。W26 契约要求改为**落盘溢出**：返回 `status:"ok"` +
//! `{"spilled":true,"content_ref":"<64hex>","bytes":N,"preview":"..."}`，并新增
//! `spill_read` 回读工具按 ref + offset/limit 分片读回原文。
//!
//! 本文件冻结 W26 契约（与 `docs/audit/agent-core-improvement-plan.md` §W26 对齐）：
//! - T1：超限读取（>120000 且 ≤16MiB）改为 spill，output 为合法 JSON，含 spilled /
//!   content_ref(64 位小写十六进制) / bytes(== 文件字节数) / preview(非空且 <4000)，
//!   且不得再出现旧错误码 `file_too_large`；
//! - T2：≤120000 bytes 的小文件行为不变（status ok、旧式文本输出、无 spilled 标记）；
//! - T3：`spill_read` 默认分片（offset=0, limit=8000）契约：status ok + `result` 对象
//!   含 ref_id/offset/limit/total_bytes/content/truncated，content ≤ 8000 bytes；
//! - T4：`spill_read` 全量回读（offset=0, limit=total_bytes ≤ 200000）content 与原文相等；
//! - T5：`spill_read` 非零 offset 切片必须严格对应原文字节区间；
//! - T6：非法 ref（含 `..`、含 `/`、非 64 位十六进制）→ `error.code=="invalid_ref"`；
//! - T7：格式合法但不存在（64 个 `0`）→ `error.code=="ref_not_found"`；
//! - T8：端到端（写 ~200KB 临时文件 → workspace_read_file 取 content_ref → spill_read
//!   回读全文 → 与源文件逐字节相等）。
//!
//! 红相说明：当前实现不存在 spill 与 `spill_read`（未注册的工具名走
//! `unsupported_tool`），因此本文件在实现落地前必然失败。测试只使用 crate 公开 API，
//! `spill_read` 通过 `ToolRouter::execute` 按**工具名**调用，不依赖任何新 API 符号，
//! 以保证本文件在红相阶段即可编译（失败发生在断言层，而不是编译层）。
//!
//! 契约层级说明（已在回执中提请 Lead 裁决）：W26 只规定「output 是合法 JSON 且含
//! spilled/content_ref/bytes/preview」，未规定这些字段挂在顶层还是 `result`/`spill` 容器内。
//! 因此 T1 的定位逻辑按 top-level → `result` → `spill` 顺序查找并打印命中位置，
//! 其余（字段名、类型、取值语义）严格断言。

use std::fs;
use std::path::PathBuf;
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pony_agent_core::agent::tools::{ToolCall, ToolRouter};
use serde_json::{json, Value};

/// 大文件夹具：2000 行 × 100 bytes = 恰好 200_000 bytes（>120000 且 ≤16MiB）。
const BIG_LINE_BYTES: usize = 100;
const BIG_LINE_COUNT: usize = 2_000;
/// 期望的大文件字节数（夹具自校验，防止 UTF-8 多字节字符污染尺寸断言）。
const BIG_FILE_BYTES: usize = BIG_LINE_BYTES * BIG_LINE_COUNT; // 200_000
/// W26 冻结阈值：整文件读取上限。
const FULL_READ_LIMIT_BYTES: usize = 120_000;
/// spill_read 默认 limit。
const DEFAULT_SPILL_LIMIT: u64 = 8_000;

// ─────────────────────────────────────────────────────────────────────────────
// 夹具与工具函数
// ─────────────────────────────────────────────────────────────────────────────

/// 唯一临时工作区（`std::env::temp_dir()` 下唯一子目录，drop 时清理）。
struct TempWorkspace {
    root: PathBuf,
}

impl TempWorkspace {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let seq = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "pony-agent-w26-{tag}-{}-{nanos}-{seq}",
            process::id()
        ));
        fs::create_dir_all(&dir).expect("创建 W26 临时工作区失败");
        // canonicalize：macOS 上 /var → /private/var，保证 router 的 root 判定与写盘路径一致。
        let root = dir.canonicalize().unwrap_or(dir);
        Self { root }
    }

    fn root(&self) -> &PathBuf {
        &self.root
    }

    /// 在工作区内写文件（content 原样写入，不做换行转换）。
    fn write(&self, relative: &str, content: &str) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("创建 W26 临时工作区子目录失败");
        }
        fs::write(&path, content).expect("写入 W26 临时测试文件失败");
        path
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn make_call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        call_id: None,
        name: name.to_string(),
        arguments,
        plan: None,
    }
}

fn read_file_call(relative_path: &str) -> ToolCall {
    make_call("workspace_read_file", json!({ "path": relative_path }))
}

fn spill_read_call(arguments: Value) -> ToolCall {
    make_call("spill_read", arguments)
}

/// 夹具自校验：确定性生成恰好 `BIG_FILE_BYTES` bytes 的 ASCII 文本。
fn big_file_content() -> String {
    let mut content = String::with_capacity(BIG_FILE_BYTES + BIG_LINE_BYTES);
    for index in 0..BIG_LINE_COUNT {
        let mut line = String::with_capacity(BIG_LINE_BYTES);
        line.push_str(&format!("line-{index:04}-"));
        while line.len() + 1 < BIG_LINE_BYTES {
            line.push('a');
        }
        line.push('\n');
        assert_eq!(line.len(), BIG_LINE_BYTES, "W26 夹具行宽必须恰为 100 bytes");
        content.push_str(&line);
    }
    assert_eq!(
        content.len(),
        BIG_FILE_BYTES,
        "W26 夹具总长必须恰为 {BIG_FILE_BYTES} bytes（ASCII-only，字节数 == 字符数）"
    );
    content
}

/// JSON 字符串定位（字段缺失时给出空串，便于聚合违规信息）。
fn json_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn json_u64(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn json_bool(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

/// 64 位小写十六进制（W26 content_ref 形态）。
fn is_lowercase_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// 在 output JSON 中定位 spill 字段所在对象（层级未在契约中冻结，允许顶层/result/spill）。
fn locate_spill_object(parsed: &Value) -> Option<(&Value, &'static str)> {
    let candidates: [(&'static str, Option<&Value>); 3] = [
        ("(top-level)", Some(parsed)),
        ("/result", parsed.get("result")),
        ("/spill", parsed.get("spill")),
    ];
    for (location, candidate) in candidates {
        let Some(candidate) = candidate else {
            continue;
        };
        if candidate.get("spilled").is_some() || candidate.get("content_ref").is_some() {
            return Some((candidate, location));
        }
    }
    None
}

fn error_code(parsed: &Value) -> Option<&str> {
    parsed
        .pointer("/error/code")
        .and_then(Value::as_str)
        .or_else(|| parsed.get("error").and_then(|error| error.get("kind")).and_then(Value::as_str))
}

fn preview(output: &str, limit: usize) -> String {
    if output.chars().count() <= limit {
        output.to_string()
    } else {
        let head: String = output.chars().take(limit).collect();
        format!(
            "{head}…(truncated, total {} chars)",
            output.chars().count()
        )
    }
}

/// 走 T1 契约拿到 spill 的 content_ref：先把超限文件读成 spill，再取出 ref。
/// 每个用例独立失败并打印实际 output，便于实施者定位。
fn obtain_content_ref(router: &ToolRouter, relative_path: &str, expected_bytes: usize) -> String {
    let result = router.execute(&read_file_call(relative_path));
    assert_eq!(
        result.status, "ok",
        "W26 前置失败：`workspace_read_file` 读取 {} bytes 的超限文件应返回 status=\"ok\"（落盘溢出），\
         实际 status=\"{}\"，output={}",
        expected_bytes,
        result.status,
        preview(&result.output, 600)
    );
    assert!(
        !result.output.contains("file_too_large"),
        "W26 前置失败：超限读取仍返回旧错误码 file_too_large。output={}",
        preview(&result.output, 600)
    );
    let parsed: Value = serde_json::from_str(&result.output).unwrap_or_else(|error| {
        panic!(
            "W26 前置失败：超限读取的 output 不是合法 JSON（解析错误：{}）。output={}",
            error,
            preview(&result.output, 600)
        )
    });
    let (spill, location) = locate_spill_object(&parsed).unwrap_or_else(|| {
        panic!(
            "W26 前置失败：超限读取的 output JSON 中找不到 spilled/content_ref 字段。\
             实际 JSON={}",
            preview(&parsed.to_string(), 600)
        )
    });
    println!("[W26] spill 字段定位 = {location}");
    let content_ref = json_str(spill, "content_ref").unwrap_or_else(|| {
        panic!(
            "W26 前置失败：spill 对象缺少字符串字段 `content_ref`。实际={}",
            preview(&spill.to_string(), 600)
        )
    });
    content_ref.to_string()
}

// ─────────────────────────────────────────────────────────────────────────────
// T1：超限读取改为落盘溢出，不再返回 file_too_large
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t1_large_file_read_spills_instead_of_file_too_large() {
    // Arrange
    let workspace = TempWorkspace::new("t1-spill");
    let source = big_file_content();
    assert!(
        source.len() > FULL_READ_LIMIT_BYTES && source.len() <= 16 * 1024 * 1024,
        "W26 夹具必须落在 (120000, 16MiB] 区间，实际 {} bytes",
        source.len()
    );
    workspace.write("big.txt", &source);
    let router = ToolRouter::with_workspace_root(workspace.root().clone());

    // Act
    let result = router.execute(&read_file_call("big.txt"));
    println!(
        "[W26][T1] status={} output={}",
        result.status,
        preview(&result.output, 400)
    );

    // Assert：聚合全部契约违规，一次性暴露给实施者
    let mut violations: Vec<String> = Vec::new();

    if result.status != "ok" {
        violations.push(format!(
            "status=\"{}\"（期望 \"ok\"：超限读取必须落盘溢出而不是报错）",
            result.status
        ));
    }
    if result.output.contains("file_too_large") {
        violations.push(
            "output 中仍出现旧错误码 `file_too_large`（期望完全消失，改为 spilled 回执）"
                .to_string(),
        );
    }

    let parsed = match serde_json::from_str::<Value>(&result.output) {
        Ok(value) => Some(value),
        Err(error) => {
            violations.push(format!(
                "output 不是合法 JSON（解析错误：{error}）—— 期望含 spilled/content_ref/bytes/preview 的 JSON 回执"
            ));
            None
        }
    };

    if let Some(parsed) = parsed.as_ref() {
        match locate_spill_object(parsed) {
            None => violations.push(format!(
                "找不到 spill 字段（期望含 \"spilled\":true 与 \"content_ref\"）。实际 JSON={}",
                preview(&parsed.to_string(), 500)
            )),
            Some((spill, location)) => {
                println!("[W26][T1] spill 字段定位 = {location}");

                match spill.get("spilled") {
                    Some(Value::Bool(true)) => {}
                    other => violations.push(format!(
                        "`spilled`={other:?}（期望布尔 true）"
                    )),
                }

                match json_str(spill, "content_ref") {
                    Some(reference) if is_lowercase_hex_64(reference) => {}
                    Some(reference) => violations.push(format!(
                        "`content_ref`=\"{}\"（长度 {}）不是 64 位小写十六进制字符串",
                        preview(reference, 80),
                        reference.len()
                    )),
                    None => violations.push(
                        "`content_ref` 缺失或不是字符串（期望 64 位小写十六进制）".to_string(),
                    ),
                }

                match json_u64(spill, "bytes") {
                    Some(bytes) if bytes as usize == source.len() => {}
                    Some(bytes) => violations.push(format!(
                        "`bytes`={bytes}（期望等于文件字节数 {}）",
                        source.len()
                    )),
                    None => violations.push(format!(
                        "`bytes` 缺失或不是无符号整数（期望 {}）",
                        source.len()
                    )),
                }

                match json_str(spill, "preview") {
                    Some(text) if text.is_empty() => {
                        violations.push("`preview` 为空字符串（期望非空的原文预览）".to_string())
                    }
                    Some(text) if text.len() >= 4_000 => violations.push(format!(
                        "`preview` 长度 {} bytes（期望 < 4000）",
                        text.len()
                    )),
                    Some(_) => {}
                    None => {
                        violations.push("`preview` 缺失或不是字符串（期望非空预览）".to_string())
                    }
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "T1 失败：{} 项 W26 spill 契约违规。\n\
         期望：status=\"ok\" + 合法 JSON 含 spilled=true / content_ref(64 小写 hex) / bytes={} / preview(非空,<4000)，\
         且不得出现 file_too_large。\n\
         实际 output={}\n\
         违规明细：\n- {}",
        violations.len(),
        source.len(),
        preview(&result.output, 800),
        violations.join("\n- ")
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// T2：小文件（≤120000 bytes）读取行为不变
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t2_small_file_read_behaviour_is_unchanged() {
    // Arrange：小文件，带唯一哨兵内容便于断言「全文可见」
    let workspace = TempWorkspace::new("t2-small");
    let sentinel = format!("W26_SMALL_SENTINEL_{}_END", process::id());
    let mut source = String::new();
    for index in 0..40 {
        source.push_str(&format!("small-line-{index:02}\n"));
    }
    source.push_str(&sentinel);
    source.push('\n');
    assert!(
        source.len() <= FULL_READ_LIMIT_BYTES,
        "W26 夹具必须 ≤120000 bytes，实际 {} bytes",
        source.len()
    );
    workspace.write("small.txt", &source);
    let router = ToolRouter::with_workspace_root(workspace.root().clone());

    // Act
    let result = router.execute(&read_file_call("small.txt"));

    // Assert 1：行为不变（ok）
    assert_eq!(
        result.status, "ok",
        "T2 失败：小文件读取应保持 status=\"ok\"。实际 status=\"{}\"，output={}",
        result.status,
        preview(&result.output, 600)
    );

    // Assert 2：不得出现 spill 标记（小文件不应落盘溢出）
    assert!(
        !result.output.contains("spilled"),
        "T2 失败：小文件 output 出现了 spill 标记（期望行为不变、不落盘）。output={}",
        preview(&result.output, 600)
    );
    assert!(
        !result.output.contains("file_too_large"),
        "T2 失败：小文件 output 出现了旧错误码 file_too_large。output={}",
        preview(&result.output, 600)
    );

    // Assert 3：仍为旧式文本回执（含 content_hash 标记 + 正文哨兵），不要求 JSON
    assert!(
        result.output.contains("content_hash"),
        "T2 失败：小文件回执缺少旧式 `content_hash` 标记（期望沿用 text 回执）。output={}",
        preview(&result.output, 600)
    );
    assert!(
        result.output.contains(&sentinel),
        "T2 失败：小文件回执不含正文哨兵 `{sentinel}`（期望全文可见）。output={}",
        preview(&result.output, 600)
    );
    if let Ok(parsed) = serde_json::from_str::<Value>(&result.output) {
        assert!(
            parsed.get("content_ref").is_none(),
            "T2 失败：小文件 JSON 回执出现了 content_ref（期望小文件不落盘溢出）。实际={}",
            preview(&parsed.to_string(), 600)
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// T3：spill_read 默认分片契约（offset=0, limit=8000）
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t3_spill_read_default_slice_contract() {
    // Arrange
    let workspace = TempWorkspace::new("t3-default");
    let source = big_file_content();
    workspace.write("big.txt", &source);
    let router = ToolRouter::with_workspace_root(workspace.root().clone());
    let content_ref = obtain_content_ref(&router, "big.txt", source.len());

    // Act：仅传 ref_id，走默认 offset/limit
    let result = router.execute(&spill_read_call(json!({ "ref_id": content_ref })));
    println!(
        "[W26][T3] status={} output={}",
        result.status,
        preview(&result.output, 400)
    );

    // Assert 1：成功
    assert_eq!(
        result.status, "ok",
        "T3 失败：`spill_read` 默认分片应返回 status=\"ok\"，实际 status=\"{}\"，output={}",
        result.status,
        preview(&result.output, 800)
    );
    let parsed: Value = serde_json::from_str(&result.output).unwrap_or_else(|error| {
        panic!(
            "T3 失败：`spill_read` output 不是合法 JSON（解析错误：{}）。output={}",
            error,
            preview(&result.output, 800)
        )
    });

    // Assert 2：`result` 对象及其 6 个冻结字段
    let payload = parsed.get("result").unwrap_or_else(|| {
        panic!(
            "T3 失败：`spill_read` output 缺少 `result` 对象。实际 JSON={}",
            preview(&parsed.to_string(), 800)
        )
    });
    assert!(
        payload.is_object(),
        "T3 失败：`result` 必须是对象，实际={}",
        preview(&payload.to_string(), 400)
    );

    let mut violations: Vec<String> = Vec::new();

    match json_str(payload, "ref_id") {
        Some(actual) if actual == content_ref => {}
        other => violations.push(format!(
            "`result.ref_id`={other:?}（期望与入参一致：{content_ref}）"
        )),
    }
    match json_u64(payload, "offset") {
        Some(0) => {}
        other => violations.push(format!("`result.offset`={other:?}（期望默认 0）")),
    }
    match json_u64(payload, "limit") {
        Some(limit) if limit == DEFAULT_SPILL_LIMIT => {}
        other => violations.push(format!(
            "`result.limit`={other:?}（期望默认 {DEFAULT_SPILL_LIMIT}）"
        )),
    }
    match json_u64(payload, "total_bytes") {
        Some(total) if total as usize == source.len() => {}
        other => violations.push(format!(
            "`result.total_bytes`={other:?}（期望文件总字节 {}）",
            source.len()
        )),
    }
    match json_str(payload, "content") {
        Some(content) if content.is_empty() => {
            violations.push("`result.content` 为空（期望默认分片的正文内容）".to_string())
        }
        Some(content) if content.len() as u64 > DEFAULT_SPILL_LIMIT => violations.push(format!(
            "`result.content` 长度 {} bytes 超过默认 limit {DEFAULT_SPILL_LIMIT}",
            content.len()
        )),
        Some(content) if !source.starts_with(content) => violations.push(format!(
            "`result.content` 不是原文前缀（offset=0 时必须从文件头部开始）。content前80={:?}",
            preview(content, 80)
        )),
        Some(_) => {}
        None => violations.push("`result.content` 缺失或不是字符串".to_string()),
    }
    match json_bool(payload, "truncated") {
        // 200_000 bytes 的文件按默认 8000 切片必然被截断（两种常见语义下都为 true）
        Some(true) => {}
        other => violations.push(format!(
            "`result.truncated`={other:?}（期望 true：源文件 {} bytes 远大于默认 limit）",
            source.len()
        )),
    }

    assert!(
        violations.is_empty(),
        "T3 失败：{} 项 spill_read 默认分片契约违规。\n实际 output={}\n违规明细：\n- {}",
        violations.len(),
        preview(&result.output, 800),
        violations.join("\n- ")
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// T4：spill_read 全量回读必须与原文完全相等
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t4_spill_read_full_content_matches_source() {
    // Arrange
    let workspace = TempWorkspace::new("t4-full");
    let source = big_file_content();
    workspace.write("big.txt", &source);
    let router = ToolRouter::with_workspace_root(workspace.root().clone());
    let content_ref = obtain_content_ref(&router, "big.txt", source.len());

    // Act：offset=0, limit=total_bytes（200000，契约上限内）
    let limit = source.len();
    assert!(limit <= 200_000, "W26 全量回读夹具不得超过 limit 上限 200000");
    let result = router.execute(&spill_read_call(json!({
        "ref_id": content_ref,
        "offset": 0,
        "limit": limit
    })));

    // Assert
    assert_eq!(
        result.status, "ok",
        "T4 失败：`spill_read` 全量回读应返回 status=\"ok\"，实际 status=\"{}\"，output={}",
        result.status,
        preview(&result.output, 800)
    );
    let parsed: Value = serde_json::from_str(&result.output).unwrap_or_else(|error| {
        panic!(
            "T4 失败：`spill_read` output 不是合法 JSON（解析错误：{}）",
            error
        )
    });
    let payload = parsed.get("result").unwrap_or_else(|| {
        panic!(
            "T4 失败：`spill_read` output 缺少 `result` 对象。实际 JSON={}",
            preview(&parsed.to_string(), 800)
        )
    });
    let content = json_str(payload, "content").unwrap_or_else(|| {
        panic!(
            "T4 失败：`result.content` 缺失或不是字符串。实际 result={}",
            preview(&payload.to_string(), 800)
        )
    });

    assert_eq!(
        json_u64(payload, "offset"),
        Some(0),
        "T4 失败：`result.offset` 应回显入参 0"
    );
    assert_eq!(
        json_u64(payload, "limit"),
        Some(limit as u64),
        "T4 失败：`result.limit` 应回显入参 {limit}"
    );
    assert_eq!(
        json_bool(payload, "truncated"),
        Some(false),
        "T4 失败：全量回读 `truncated` 应为 false（已读到文件尾）。实际 result={}",
        preview(&payload.to_string(), 400)
    );
    assert_eq!(
        content.len(),
        source.len(),
        "T4 失败：全量回读 content 长度 {} 与源文件 {} 不一致（content前120={:?}）",
        content.len(),
        source.len(),
        preview(content, 120)
    );
    assert!(
        content == source,
        "T4 失败：全量回读 content 与源文件内容不相等。\
         content前120={:?}，源文件前120={:?}",
        preview(content, 120),
        preview(&source, 120)
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// T5：非零 offset 切片必须严格对应原文区间
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t5_spill_read_nonzero_offset_slices_exact_range() {
    // Arrange
    let workspace = TempWorkspace::new("t5-offset");
    let source = big_file_content();
    workspace.write("big.txt", &source);
    let router = ToolRouter::with_workspace_root(workspace.root().clone());
    let content_ref = obtain_content_ref(&router, "big.txt", source.len());

    let offset: usize = 8_000;
    let limit: usize = 8_000;

    // Act
    let result = router.execute(&spill_read_call(json!({
        "ref_id": content_ref,
        "offset": offset,
        "limit": limit
    })));

    // Assert
    assert_eq!(
        result.status, "ok",
        "T5 失败：带非零 offset 的 `spill_read` 应返回 status=\"ok\"，实际 status=\"{}\"，output={}",
        result.status,
        preview(&result.output, 800)
    );
    let parsed: Value = serde_json::from_str(&result.output).unwrap_or_else(|error| {
        panic!(
            "T5 失败：`spill_read` output 不是合法 JSON（解析错误：{}）",
            error
        )
    });
    let payload = parsed.get("result").unwrap_or_else(|| {
        panic!(
            "T5 失败：`spill_read` output 缺少 `result` 对象。实际 JSON={}",
            preview(&parsed.to_string(), 800)
        )
    });
    let content = json_str(payload, "content").unwrap_or_else(|| {
        panic!(
            "T5 失败：`result.content` 缺失或不是字符串。实际 result={}",
            preview(&payload.to_string(), 800)
        )
    });

    // 夹具为纯 ASCII，按字节切片即按字符切片，语义确定。
    let expected = &source[offset..offset + limit];
    assert_eq!(
        json_u64(payload, "offset"),
        Some(offset as u64),
        "T5 失败：`result.offset` 应回显入参 {offset}"
    );
    assert_eq!(
        json_u64(payload, "limit"),
        Some(limit as u64),
        "T5 失败：`result.limit` 应回显入参 {limit}"
    );
    assert_eq!(
        json_bool(payload, "truncated"),
        Some(true),
        "T5 失败：切片未到文件尾时 `truncated` 应为 true（文件共 {} bytes）",
        source.len()
    );
    assert!(
        content == expected,
        "T5 失败：offset={offset} limit={limit} 的切片与原文区间不一致。\
         期望前120={:?}；实际前120={:?}",
        preview(expected, 120),
        preview(content, 120)
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// T6：非法 ref → invalid_ref
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t6_spill_read_invalid_refs_return_invalid_ref_error() {
    // Arrange
    let workspace = TempWorkspace::new("t6-invalid-ref");
    let router = ToolRouter::with_workspace_root(workspace.root().clone());

    let invalid_refs: Vec<(&str, String)> = vec![
        ("路径穿越 ..", "../../etc/passwd".to_string()),
        ("含 / 分隔符", format!("{}/{}", "a".repeat(32), "b".repeat(32))),
        ("长度不足 64", "a".repeat(63)),
        ("长度超过 64", "a".repeat(65)),
        ("非法十六进制字符", "g".repeat(64)),
        ("含 .. 后缀", format!("{}/../{}", "c".repeat(32), "d".repeat(32))),
    ];

    // Act + Assert：聚合违规，便于一次看到全部非法输入的真实行为
    let mut violations: Vec<String> = Vec::new();

    for (label, ref_id) in invalid_refs {
        let result = router.execute(&spill_read_call(json!({ "ref_id": ref_id })));
        let parsed = serde_json::from_str::<Value>(&result.output).ok();
        let code = parsed.as_ref().and_then(error_code);

        if result.status != "error" {
            violations.push(format!(
                "[{label}] ref_id={:?}: status=\"{}\"（期望 \"error\"），output={}",
                preview(&ref_id, 80),
                result.status,
                preview(&result.output, 400)
            ));
            continue;
        }
        if code != Some("invalid_ref") {
            violations.push(format!(
                "[{label}] ref_id={:?}: error.code={code:?}（期望 \"invalid_ref\"），output={}",
                preview(&ref_id, 80),
                preview(&result.output, 400)
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "T6 失败：{} 个非法 ref 未按 W26 契约返回 `invalid_ref` 错误。\n\
         期望：status=\"error\" + error.code=\"invalid_ref\"。\n\
         实际违规：\n- {}",
        violations.len(),
        violations.join("\n- ")
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// T7：格式合法但不存在 → ref_not_found
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t7_spill_read_unknown_but_well_formed_ref_returns_ref_not_found() {
    // Arrange：64 个 0 是合法 64 位小写十六进制，但几乎不可能命中真实 spill
    let workspace = TempWorkspace::new("t7-not-found");
    let router = ToolRouter::with_workspace_root(workspace.root().clone());
    let missing_ref = "0".repeat(64);

    // Act
    let result = router.execute(&spill_read_call(json!({ "ref_id": missing_ref })));

    // Assert
    assert_eq!(
        result.status, "error",
        "T7 失败：不存在的合法 ref 应返回 status=\"error\"，实际 status=\"{}\"，output={}",
        result.status,
        preview(&result.output, 600)
    );
    let parsed: Value = serde_json::from_str(&result.output).unwrap_or_else(|error| {
        panic!(
            "T7 失败：`spill_read` 错误回执不是合法 JSON（解析错误：{}）。output={}",
            error,
            preview(&result.output, 600)
        )
    });
    assert_eq!(
        error_code(&parsed),
        Some("ref_not_found"),
        "T7 失败：不存在 ref（64 个 0）的 error.code 应为 \"ref_not_found\"，实际={:?}，output={}",
        error_code(&parsed),
        preview(&result.output, 600)
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// T8：端到端 —— 200KB 临时文件 → content_ref → spill_read 回读与源文件逐字节相等
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t8_end_to_end_spill_roundtrip_matches_source_file() {
    // Arrange：~200KB 文件写入测试用工作区
    let workspace = TempWorkspace::new("t8-e2e");
    let source = big_file_content();
    let source_path = workspace.write("big.txt", &source);
    // 物理事实先决：磁盘上的字节序列就是后续比对的基准
    let source_bytes = fs::read(&source_path).expect("回读 W26 源文件失败");
    assert_eq!(
        source_bytes.len(),
        BIG_FILE_BYTES,
        "W26 E2E 夹具落盘字节数异常"
    );
    assert!(
        source_bytes.len() > FULL_READ_LIMIT_BYTES,
        "W26 E2E 夹具必须超过整文件读取上限"
    );

    let router = ToolRouter::with_workspace_root(workspace.root().clone());

    // Act 1：workspace_read_file 拿到 content_ref
    let content_ref = obtain_content_ref(&router, "big.txt", source_bytes.len());

    // Act 2：spill_read 全量回读
    let result = router.execute(&spill_read_call(json!({
        "ref_id": content_ref,
        "offset": 0,
        "limit": source_bytes.len()
    })));

    // Assert
    assert_eq!(
        result.status, "ok",
        "T8 失败：端到端 spill_read 回读应返回 status=\"ok\"，实际 status=\"{}\"，output={}",
        result.status,
        preview(&result.output, 800)
    );
    let parsed: Value = serde_json::from_str(&result.output).unwrap_or_else(|error| {
        panic!(
            "T8 失败：`spill_read` output 不是合法 JSON（解析错误：{}）",
            error
        )
    });
    let payload = parsed.get("result").unwrap_or_else(|| {
        panic!(
            "T8 失败：`spill_read` output 缺少 `result` 对象。实际 JSON={}",
            preview(&parsed.to_string(), 800)
        )
    });
    let content = json_str(payload, "content").unwrap_or_else(|| {
        panic!(
            "T8 失败：`result.content` 缺失或不是字符串。实际 result={}",
            preview(&payload.to_string(), 800)
        )
    });
    assert_eq!(
        json_u64(payload, "total_bytes"),
        Some(source_bytes.len() as u64),
        "T8 失败：`result.total_bytes` 应等于源文件字节数 {}",
        source_bytes.len()
    );

    // 逐字节相等（不只是字符串相等：显式按字节比对）
    let spilled_bytes = content.as_bytes();
    assert_eq!(
        spilled_bytes.len(),
        source_bytes.len(),
        "T8 失败：回读字节数 {} 与源文件 {} 不相等",
        spilled_bytes.len(),
        source_bytes.len()
    );
    let mismatch = spilled_bytes
        .iter()
        .zip(source_bytes.iter())
        .position(|(left, right)| left != right);
    assert!(
        mismatch.is_none(),
        "T8 失败：端到端回读与源文件字节序列不一致（首个差异位于 offset {:?}）。\
         源文件片段={:?}，回读片段={:?}",
        mismatch,
        mismatch.map(|index| String::from_utf8_lossy(
            &source_bytes[index.saturating_sub(40)..(index + 40).min(source_bytes.len())]
        ).to_string()),
        mismatch.map(|index| String::from_utf8_lossy(
            &spilled_bytes[index.saturating_sub(40)..(index + 40).min(spilled_bytes.len())]
        ).to_string())
    );
}
