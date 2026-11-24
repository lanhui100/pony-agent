//! wave-tui-session-resilience 补丁红相：C1 分片闭合缺口（持久化/回灌面）。
//!
//! 物理反例来源：Lead 实跑 `/tmp/l3a-repro/repro` exit 0 —— 完整 DSML 块切成
//! `["<｜", "｜DSML｜｜ calls>…"]` 两片逐片喂
//! `pony_agent_core::agent::provider::sanitize_dsml_protocol_text` 并累加，得
//! `"<｜［工具调用］"`：半截 `<｜` 残留进 `OpenAiSseAccumulator.output_text`
//! 并回灌会话。TUI 用户可见面 S1 已截掉，但 provider 清洗入口未闭合。
//!
//! 本文件由独立 Test Agent 追加，与已冻结的
//! `crates/pony-agent-core/tests/tui_session_resilience_acceptance.rs` 正交；
//! 不改生产代码、不改冻结验收文件、不 `#[ignore]`、不吞异常。
//! 红相预期：用例 1 以 Rust 原生 `assert!` 失败（`<｜` 残留）；用例 2 通过，
//! 防止 Executor 用「见 `<` 就删」的误伤式修法把普通正文改坏。

use pony_agent_core::agent::provider::sanitize_dsml_protocol_text;

const FULL_DSML: &str = "<｜｜DSML｜｜ calls>\n<｜｜DSML｜｜ invoke name=\"Run\">\n<｜｜DSML｜｜ parameter name=\"command\">cargo test</｜｜DSML｜｜ parameter>\n</｜｜DSML｜｜ invoke>\n</｜｜DSML｜｜ calls>";

/// C1 / CHUNK-A01：两片分片逐片调用清洗入口并累加，不得残留半截协议前缀。
#[test]
fn c1_chunk_split_dsml_prefix_is_not_persisted() {
    // Arrange：严格按 L3 物理反例切两片；用合法 UTF-8 边界 split_at（非裸 byte 切片）。
    let split_at = "<｜".len();
    assert!(
        FULL_DSML.is_char_boundary(split_at),
        "CHUNK-A01 harness：切点必须是合法 UTF-8 字符边界"
    );
    assert!(
        FULL_DSML.starts_with("<｜"),
        "CHUNK-A01 harness：完整 DSML 块必须以 '<｜' 开头"
    );
    let (first, second) = FULL_DSML.split_at(split_at);
    assert_eq!(first, "<｜", "CHUNK-A01 harness：第一片必须是半截 '<｜'");
    assert!(
        second.starts_with("｜DSML｜｜ calls>"),
        "CHUNK-A01 harness：第二片必须承接 DSML calls 开标签，实际 = {second:?}"
    );
    assert_eq!(
        format!("{first}{second}"),
        FULL_DSML,
        "CHUNK-A01 harness：两片拼回必须等于完整 DSML 块"
    );

    // Act：逐片调用同一个生产清洗入口并累加（模拟 SSE delta → output_text）。
    let mut acc = String::new();
    for chunk in [first, second] {
        acc.push_str(&sanitize_dsml_protocol_text(chunk));
    }

    // Assert：半截前缀 / DSML 标记 / 内部参数名一律不得进入累计文本；必须保留用户可读占位。
    assert!(
        !acc.contains("<｜"),
        "C1/CHUNK-A01：分片累加后不得残留半截 '<｜'，实际 = {acc:?}"
    );
    assert!(
        !acc.contains("｜DSML｜｜"),
        "C1/CHUNK-A01：分片累加后不得残留 DSML 标记，实际 = {acc:?}"
    );
    assert!(
        !acc.contains("invoke name=") && !acc.contains("parameter name="),
        "C1/CHUNK-A01：分片累加后不得反显内部协议字段，实际 = {acc:?}"
    );
    assert!(
        acc.contains("工具调用"),
        "C1/CHUNK-A01：清洗后必须保留面向用户的「工具调用」占位，实际 = {acc:?}"
    );
}

/// C1 / CHUNK-A02：普通正文以尖括号结尾、markdown 代码块逐字保真（防误伤）。
#[test]
fn c1_plain_angle_bracket_text_is_verbatim() {
    // Arrange
    let plain = "以尖括号结尾 <";
    let markdown = "```rust\nlet x = \"<a>\";\n// <b> 与 <tag> 是普通尖括号\n```\n正文 <tag attr=\"v\"> 保留";

    // Act + Assert：清洗入口对非 DSML 文本必须是恒等变换。
    assert_eq!(
        sanitize_dsml_protocol_text(plain),
        plain,
        "C1/CHUNK-A02：普通正文以 '<' 结尾必须逐字保真"
    );
    assert_eq!(
        sanitize_dsml_protocol_text(markdown),
        markdown,
        "C1/CHUNK-A02：markdown 代码块与含尖括号正文必须逐字保真"
    );
}
