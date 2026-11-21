//! B264/A4：`/resume` 行列宽守卫（`resume_row_line` 的 pad 算术，纯渲染级、无 IO）。
//!
//! 为什么必须机械锁：`resume_row_line` 的整行宽度必须**精确等于面板内宽** `body.width`，
//! 由「选择条 1 + 呼吸位 1 + 左列 + 间隔 + 右列 + pad」拼成。pad 一旦算错，宽字符行会越界，
//! 或者右列标记（`· 当前` / `· <短id>`）被挤出面板。
//!
//! 取值口径：`body.width = 面板宽 - 2`（左右边框），面板宽 = `min(96, 终端宽 - 4)`，故
//! - 40 列终端 ⇒ 面板 36 ⇒ 内宽 34（最窄可用面板，取 38 的是「面板 40」同级边界用例）
//! - 80 列终端 ⇒ 面板 76 ⇒ 内宽 74
//! - 96 列终端 ⇒ 面板 92 ⇒ 内宽 90
//! - 120 列终端 ⇒ 面板 96（上限）⇒ 内宽 94
//!
//! 这里直接覆盖 38/78/94/118 四档内宽（含 80 列与 120 列两档），左列用超长 CJK 标题 +
//! 跨区前缀把预算压到边界，确保走的是「截断 + 省略号」路径而不是宽松路径。

use super::*;

/// 造一行「最坏情况」的列文本：超长 CJK 标题 + 跨区前缀 + active&selected 双标记。
fn worst_case_parts(inner_width: usize) -> resume_list::ResumeRowParts {
    let title = "重构会话列表与归档入口".repeat(20);
    let input = resume_list::ResumeRowInput {
        session_id: "pa_20261121-deadbeefcafe",
        title: &title,
        workspace_id: Some("ws-alpha"),
        workspace_label: Some("阿尔法工作区"),
        updated_at: "11-21 14:05",
        archived: false,
        is_active: true,
        is_selected: true,
        current_workspace_id: Some("ws-beta"),
    };
    // 渲染层给纯函数的预算固定为 `body.width - 2`（选择条 + 呼吸位）。
    resume_list::resume_row_parts(&input, inner_width.saturating_sub(2))
}

/// 把一行 span 拼成纯文本（宽度断言之外的「内容仍可见」断言用）。
fn line_text(line: &Line<'static>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

#[test]
fn resume_row_line_fills_exactly_the_panel_inner_width() {
    for width in [38usize, 78, 94, 118] {
        for flags in [
            ResumeRowFlags {
                selected: false,
                archived: false,
            },
            ResumeRowFlags {
                selected: true,
                archived: true,
            },
        ] {
            let parts = worst_case_parts(width);

            let line = resume_row_line(flags, &parts, width);

            assert_eq!(
                line_display_width(&line),
                width,
                "行宽必须精确等于面板内宽（width={width}, flags={flags:?}）\
                 —— pad 算术错一列就会越界或吃掉右列标记"
            );
        }
    }
}

#[test]
fn resume_row_line_keeps_right_column_markers_inside_the_panel() {
    for width in [38usize, 78, 94, 118] {
        let parts = worst_case_parts(width);

        let line = resume_row_line(
            ResumeRowFlags {
                selected: true,
                archived: false,
            },
            &parts,
            width,
        );

        let text = line_text(&line);
        // C1：选中行的右列必须带短 id；active 行必须带「当前」（38 列窄面板下也不能被截没）。
        assert!(
            text.contains("当前") && text.contains("deadbeef"),
            "width={width}: 右列标记必须完整落在面板内，实际行文本 {text:?}"
        );
        assert!(
            line_display_width(&line) <= width,
            "width={width}: 行宽不得越界（实际 {}）",
            line_display_width(&line)
        );
    }
}
