//! 接线测试：助手正文里的 GFM 管道表格必须走 `tui::table` 块级渲染。
//!
//! `table::table_tests` 只覆盖 `table.rs` 自身的形态判定与排版；一旦有人在
//! `message_lines_with_ctx` 里把表格分支摘掉、或把切片起点算错（`&src[idx..]`
//! 吞掉表头、`&src[idx - 2..]` 把上一段正文当块首），`table.rs` 依旧全绿，
//! 用户看到的却又是满屏竖线。故这里用公开入口 `message_lines_for` 做端到端断言。

use super::*;

fn message(role: MessageRole, content: &str) -> ChatMessage {
    ChatMessage {
        role,
        content: content.to_string(),
        timestamp_ms: 0,
    }
}

fn plain(lines: &[Line<'static>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn assistant_table_renders_as_block_without_pipes() {
    let content = "对照如下：\n\n| 提交 | 内容 |\n| --- | --- |\n| `8e575f32` | CLI 帮助 |\n| `289c444a` | 看板符号 |\n\n以上。";
    let lines = message_lines_for(&message(MessageRole::Assistant, content), 80);
    let texts = plain(&lines);
    assert!(
        !texts.iter().any(|text| text.contains('|')),
        "表格竖线不得残留: {texts:#?}"
    );
    assert!(
        texts.iter().any(|text| text.contains('─')),
        "表头下方应有发丝线: {texts:#?}"
    );
    // 块级排版：同一行里同时看得见两列文本。
    assert!(
        texts
            .iter()
            .any(|text| text.contains("8e575f32") && text.contains("CLI 帮助")),
        "同一行应同时含两列: {texts:#?}"
    );
    // 表格前的段落与表格后的尾巴都必须保留：切片起点不得把表头前一行当块首。
    assert!(
        texts.iter().any(|text| text.contains("对照如下")),
        "表格前缀段落丢失: {texts:#?}"
    );
    assert!(
        texts.iter().any(|text| text.contains("以上")),
        "表格后缀段落丢失: {texts:#?}"
    );
}

#[test]
fn fenced_table_is_left_verbatim() {
    let content = "```markdown\n| a | b |\n| --- | --- |\n| 1 | 2 |\n```";
    let lines = message_lines_for(&message(MessageRole::Assistant, content), 80);
    let texts = plain(&lines);
    assert!(
        texts.iter().any(|text| text.contains("| a | b |")),
        "围栏内的表格必须原样保留: {texts:#?}"
    );
    assert!(
        !texts.iter().any(|text| text.contains('─')),
        "围栏内不得生成表头发丝线: {texts:#?}"
    );
}
