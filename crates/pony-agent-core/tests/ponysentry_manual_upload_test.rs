//! 手动上传 Trace → OTel/Langfuse 重放迁移契约（Wave43 D5/D6）。
//!
//! 冻结契约（P1-3 契约更新：返回**结构化回执**，不再是裸 turn 数）：
//! 1. `HostControlPlane::upload_session_trace` 入口签名：
//!    `Result<crate::agent::otel::UploadTraceReceipt, String>`
//!    （`turns` / `trace_ids` / `failures`；前端提示与失败明细依赖该结构）；
//! 2. 实现必须转调 `otel::force_export_session_trace`（旁路全局开关 = 用户点击即逐次授权）；
//! 3. 空会话不发起任何导出，直接 `Ok(UploadTraceReceipt::default())`（零 egress）；
//! 4. 控制面实现不得再引用 PonySentry trace 模块。
//!
//! 行为级证明在 `otel_trace_migration_test.rs` 的 OT-6（本地 OTLP sink + wire 级断言）；
//! 「退役符号全仓 0 命中」由 OT-7 机械守卫覆盖——本文件**不再重复书写退役符号字面量**，
//! 否则守卫会把自己的负向断言误判为残留（自指污染）。

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read_repo_source(relative_to_repo_root: &str) -> String {
    let path = repo_root().join(relative_to_repo_root);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("契约证据源必须可读（{}）：{err}", path.display()))
}

/// 取 `needle` 之后最多 `len` 字节的片段（用于函数签名的形状断言）。
fn slice_after<'a>(source: &'a str, needle: &str, len: usize) -> &'a str {
    let index = source
        .find(needle)
        .unwrap_or_else(|| panic!("源码中必须存在 `{needle}`"));
    &source[index..(index + len).min(source.len())]
}

const QUERY_COMMANDS: &str = "crates/pony-agent-core/src/agent/control_plane/query_commands.rs";

#[test]
fn upload_session_trace_delegates_to_otel_replay() {
    let source = read_repo_source(QUERY_COMMANDS);
    // 窗口随签名长度调整（P1-3 起返回类型变长：`Result<UploadTraceReceipt, String>`）。
    let body = slice_after(&source, "pub async fn upload_session_trace(", 2_000);

    assert!(
        body.contains("crate::agent::otel::force_export_session_trace("),
        "D5：upload_session_trace 必须转调 OTel 重放入口（手动上传目标改为 Langfuse），实际片段：{body}"
    );
    assert!(
        !body.contains("ponysentry::"),
        "D6：upload_session_trace 不得再引用 PonySentry 模块（trace 能力已下线），实际片段：{body}"
    );
}

#[test]
fn upload_session_trace_keeps_structured_receipt_contract() {
    let source = read_repo_source(QUERY_COMMANDS);
    // 签名冻结（P1-3 契约更新）：前端回执文案依赖 `turns/traceIds/failures` 结构化语义，
    // 不再使用裸 turn 数。
    assert!(
        source.contains("-> Result<crate::agent::otel::UploadTraceReceipt, String>"),
        "P1-3：upload_session_trace 必须返回结构化回执 `-> Result<UploadTraceReceipt, String>`"
    );
}

#[test]
fn empty_session_short_circuits_with_zero_egress() {
    let source = read_repo_source(QUERY_COMMANDS);
    // 窗口随签名长度调整（P1-3 起返回类型变长：`Result<UploadTraceReceipt, String>`）。
    let body = slice_after(&source, "pub async fn upload_session_trace(", 2_000);

    assert!(
        body.contains("traces.is_empty()") && body.contains("UploadTraceReceipt::default()"),
        "P1-3：空会话必须直接返回 `Ok(UploadTraceReceipt::default())`（零 egress，语义同旧 Ok(0)），实际片段：{body}"
    );
}
