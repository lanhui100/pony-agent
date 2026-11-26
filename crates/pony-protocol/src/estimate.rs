use crate::session_types::TurnHistoryMessage;

pub const PRUNE_MARKER: &str = "\n\n[... tool result middle pruned ...]\n\n";

/// 估算文本的 token 数（粗略估算：每 4 字符 ≈ 1 token）。
pub fn estimate_text_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// 压缩边界推导：`history/squash.base_seq` = 最后一个被压缩掉的折叠消息的 seq。
pub fn derive_squash_base_seq(
    folded: &[(u64, TurnHistoryMessage)],
    retained: &[TurnHistoryMessage],
) -> u64 {
    if folded.is_empty() {
        return 0;
    }
    let mut matched = 0usize;
    while matched < retained.len() && matched < folded.len() {
        let kept = &retained[retained.len() - 1 - matched];
        let (_, candidate) = &folded[folded.len() - 1 - matched];
        if kept.role == candidate.role && kept.content == candidate.content {
            matched += 1;
        } else {
            break;
        }
    }
    if matched >= folded.len() {
        return folded
            .iter()
            .map(|(seq, _)| *seq)
            .min()
            .unwrap_or(0)
            .saturating_sub(1);
    }
    folded[folded.len() - 1 - matched].0
}
