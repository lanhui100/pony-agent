use crate::session_types::TurnHistoryMessage;

pub const PRUNE_MARKER: &str = "\n\n[... tool result middle pruned ...]\n\n";

/// 估算文本的 token 数：ASCII 按 4 字符 ≈ 1 token，非 ASCII（如中文/全角标点）按 1.5 token/字校准，
/// 避免纯中文或密集符号载荷被低估 4~6 倍（仍独立于 128 MiB 请求体物理字节闸门）。
pub fn estimate_text_tokens(text: &str) -> usize {
    let mut ascii_chars = 0usize;
    let mut non_ascii_chars = 0usize;
    for ch in text.chars() {
        if ch.is_ascii() {
            ascii_chars += 1;
        } else {
            non_ascii_chars += 1;
        }
    }
    (ascii_chars + non_ascii_chars * 6).div_ceil(4)
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
