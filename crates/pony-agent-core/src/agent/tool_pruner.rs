use serde_json::Value;
use std::borrow::Cow;

/// 默认修剪阈值（字符数）：超过此长度的工具结果将被裁剪。对齐 DSH 默认 8192。
pub const DEFAULT_PRUNE_THRESHOLD_CHARS: usize = 8192;
/// 默认头部保留字符数：对齐 DSH 默认 4096。
pub const DEFAULT_PRUNE_HEAD_CHARS: usize = 4096;
/// 默认尾部保留字符数：对齐 DSH 默认 1024。
pub const DEFAULT_PRUNE_TAIL_CHARS: usize = 1024;
/// 修剪连接标记（对齐 DSH 格式）
pub const PRUNE_MARKER: &str = "\n\n[... tool result middle pruned ...]\n\n";

/// 工具结果修剪预算（字符口径）的单一真源。
///
/// 跨轮 native transcript 回放与 provider 同轮 follow-up 是同一语义的两次消费，
/// 阈值与切片原语必须共用本结构，禁止在消费方各自硬编码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PruneBudget {
    pub threshold_chars: usize,
    pub head_chars: usize,
    pub tail_chars: usize,
}

impl PruneBudget {
    /// 跨轮 native transcript 回放（ADR 0123 / DSH 默认）。
    pub const TRANSCRIPT: Self = Self {
        threshold_chars: DEFAULT_PRUNE_THRESHOLD_CHARS,
        head_chars: DEFAULT_PRUNE_HEAD_CHARS,
        tail_chars: DEFAULT_PRUNE_TAIL_CHARS,
    };

    /// provider 同轮 follow-up 的内联工具结果上限（保持既有 12k / 4.5k / 2.5k 契约）。
    pub const PROVIDER_FOLLOWUP: Self = Self {
        threshold_chars: 12_000,
        head_chars: 4_500,
        tail_chars: 2_500,
    };
}

/// 字符安全的首尾切片原语。
///
/// 超预算时返回 `(head, tail, 原始字符数)`；未超预算返回 `None`（调用方零复制短路）。
/// 严格按字符（码点）边界切片，绝不截断多字节汉字与 emoji。
pub fn split_head_tail(content: &str, budget: PruneBudget) -> Option<(String, String, usize)> {
    // 字节快路径：UTF-8 每字符至少 1 字节，字节数不超阈值时字符数必不超。
    if content.len() <= budget.threshold_chars {
        return None;
    }

    let char_count = content.chars().count();
    if char_count <= budget.threshold_chars || budget.head_chars + budget.tail_chars >= char_count {
        return None;
    }

    let head_byte_end = content
        .char_indices()
        .nth(budget.head_chars)
        .map(|(idx, _)| idx)?;
    let tail_char_start = char_count - budget.tail_chars;
    let tail_byte_start = content
        .char_indices()
        .nth(tail_char_start)
        .map(|(idx, _)| idx)?;
    if head_byte_end >= tail_byte_start {
        return None;
    }

    Some((
        content[..head_byte_end].to_string(),
        content[tail_byte_start..].to_string(),
        char_count,
    ))
}

/// 纯函数：按给定预算对工具结果文本进行确定性首尾修剪。
///
/// 契约保证：
/// 1. 字符数 <= 阈值时，直接返回 `Cow::Borrowed`，零分配；
/// 2. 超阈值时安全按 UTF-8 字符切片，精确保留前 `head` 字符与后 `tail` 字符；
/// 3. 绝不会在多字节字符（中文、emoji 等）的内部切断造成 UTF-8 panic。
pub fn prune_tool_result_with_budget<'a>(content: &'a str, budget: PruneBudget) -> Cow<'a, str> {
    let Some((head_str, tail_str, _)) = split_head_tail(content, budget) else {
        return Cow::Borrowed(content);
    };

    let mut pruned = String::with_capacity(head_str.len() + PRUNE_MARKER.len() + tail_str.len());
    pruned.push_str(&head_str);
    pruned.push_str(PRUNE_MARKER);
    pruned.push_str(&tail_str);
    Cow::Owned(pruned)
}

/// 纯函数：对工具结果文本进行确定性首尾修剪（显式阈值入口，保留既有契约）。
pub fn prune_tool_result_content<'a>(
    content: &'a str,
    threshold: usize,
    head: usize,
    tail: usize,
) -> Cow<'a, str> {
    prune_tool_result_with_budget(
        content,
        PruneBudget {
            threshold_chars: threshold,
            head_chars: head,
            tail_chars: tail,
        },
    )
}

/// 使用默认参数修剪工具结果字符串
pub fn prune_tool_result_default<'a>(content: &'a str) -> Cow<'a, str> {
    prune_tool_result_with_budget(content, PruneBudget::TRANSCRIPT)
}

/// 就地修剪原生消息列表中的工具消息：仅超阈值消息发生替换，
/// 未超阈值消息零复制（避免每轮对全量 transcript 重复 clone）。
pub fn prune_native_messages_in_place(messages: &mut [Value]) {
    for message in messages {
        if message.get("role").and_then(Value::as_str) != Some("tool") {
            continue;
        }
        let Some(content_str) = message.get("content").and_then(Value::as_str) else {
            continue;
        };
        let Cow::Owned(pruned) = prune_tool_result_default(content_str) else {
            continue;
        };
        if let Some(map) = message.as_object_mut() {
            map.insert("content".to_string(), Value::String(pruned));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_prune_below_threshold_returns_borrowed() {
        let short_text = "short output from tool";
        let result = prune_tool_result_content(short_text, 100, 10, 10);
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, short_text);
    }

    #[test]
    fn test_prune_above_threshold_retains_head_and_tail() {
        let text = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ"; // 36 字符
                                                           // 阈值 20，前 5，后 5
        let result = prune_tool_result_content(text, 20, 5, 5);
        assert!(matches!(result, Cow::Owned(_)));
        let expected = format!("01234{PRUNE_MARKER}VWXYZ");
        assert_eq!(result.as_ref(), expected);
    }

    #[test]
    fn test_prune_utf8_multibyte_chinese_and_emoji_safe() {
        // 测试中文字符与 emoji 不会被截半字节
        let prefix = "这是一段中文测试开头，包含各种符号：🚀🔥💡。"; // 23 字符
        let middle = "中".repeat(100);
        let suffix = "这是中文结尾部分，包含尾部标记：🎉✨。"; // 19 字符
        let full = format!("{prefix}{middle}{suffix}");

        let result = prune_tool_result_content(&full, 50, 10, 10);
        assert!(matches!(result, Cow::Owned(_)));
        assert!(result.contains(PRUNE_MARKER));
        assert!(result.starts_with(&full.chars().take(10).collect::<String>()));
        assert!(result.ends_with(
            &full
                .chars()
                .skip(full.chars().count() - 10)
                .collect::<String>()
        ));
    }

    #[test]
    fn test_prune_native_messages_in_place_only_touches_oversized_tool_messages() {
        let large_output = "a".repeat(10000);
        let mut messages = vec![
            json!({
                "role": "tool",
                "tool_call_id": "call_123",
                "content": large_output
            }),
            json!({
                "role": "user",
                "content": large_output
            }),
        ];

        prune_native_messages_in_place(&mut messages);

        let content = messages[0].get("content").unwrap().as_str().unwrap();
        assert!(content.contains(PRUNE_MARKER));
        assert_eq!(messages[0].get("role").unwrap().as_str().unwrap(), "tool");
        assert_eq!(
            messages[0].get("tool_call_id").unwrap().as_str().unwrap(),
            "call_123"
        );

        // 非 tool 消息不受影响
        assert_eq!(
            messages[1].get("content").unwrap().as_str().unwrap().len(),
            10000
        );
    }

    #[test]
    fn test_repair_drops_orphan_tool_messages() {
        let transcript = vec![
            json!({ "role": "user", "content": "hello" }),
            json!({ "role": "tool", "tool_call_id": "ghost_call_001", "content": "orphan result" }),
            json!({ "role": "assistant", "content": "world" }),
        ];

        let (repaired, mutated) = repair_and_filter_tool_roundtrips(&transcript);
        assert!(mutated, "孤儿 tool 必须被识别并修改");
        assert_eq!(repaired.len(), 2);
        assert!(!repaired
            .iter()
            .any(|m| m.get("role").and_then(Value::as_str) == Some("tool")));
    }

    #[test]
    fn test_repair_enforces_strict_adjacency_for_consecutive_assistants() {
        let transcript = vec![
            json!({ "role": "user", "content": "start" }),
            json!({
                "role": "assistant",
                "content": "",
                "tool_calls": [{ "id": "call_A", "type": "function", "function": { "name": "f", "arguments": "{}" } }]
            }),
            json!({ "role": "assistant", "content": "second assistant directly without tool" }),
        ];

        let (repaired, mutated) = repair_and_filter_tool_roundtrips(&transcript);
        assert!(mutated);
        assert_eq!(repaired.len(), 4);
        assert_eq!(
            repaired[1].get("role").and_then(Value::as_str),
            Some("assistant")
        );
        assert_eq!(
            repaired[2].get("role").and_then(Value::as_str),
            Some("tool")
        );
        assert_eq!(
            repaired[2].get("tool_call_id").and_then(Value::as_str),
            Some("call_A")
        );
        assert_eq!(
            repaired[3].get("role").and_then(Value::as_str),
            Some("assistant")
        );
    }

    #[test]
    fn test_legal_transcript_takes_zero_repair_fast_path() {
        let transcript = vec![
            json!({ "role": "user", "content": "start" }),
            json!({
                "role": "assistant",
                "content": "",
                "tool_calls": [{ "id": "call_A", "type": "function", "function": { "name": "f", "arguments": "{}" } }]
            }),
            json!({ "role": "tool", "tool_call_id": "call_A", "content": "ok" }),
            json!({ "role": "assistant", "content": "done" }),
        ];

        assert!(!needs_tool_roundtrip_repair(&transcript));
        let (repaired, mutated) = repair_and_filter_tool_roundtrips(&transcript);
        assert!(!mutated);
        assert_eq!(repaired, transcript);
    }

    #[test]
    fn test_repair_closes_pending_roundtrip_before_system_boundary() {
        let transcript = vec![
            json!({ "role": "user", "content": "start" }),
            json!({
                "role": "assistant",
                "content": "",
                "tool_calls": [{ "id": "call_S", "type": "function", "function": { "name": "f", "arguments": "{}" } }]
            }),
            json!({ "role": "system", "content": "mid-conversation system note" }),
        ];

        assert!(needs_tool_roundtrip_repair(&transcript));
        let (repaired, mutated) = repair_and_filter_tool_roundtrips(&transcript);
        assert!(mutated);
        let roles: Vec<&str> = repaired
            .iter()
            .filter_map(|msg| msg.get("role").and_then(Value::as_str))
            .collect();
        assert_eq!(
            roles,
            vec!["user", "assistant", "tool", "assistant", "system"]
        );
        assert_eq!(
            repaired[2].get("tool_call_id").and_then(Value::as_str),
            Some("call_S")
        );
        assert!(repaired[3]
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("[中断轮次检查点"));
    }

    #[test]
    fn test_split_head_tail_shared_budget_keeps_exact_chars() {
        assert!(split_head_tail("short", PruneBudget::TRANSCRIPT).is_none());

        let content = "中".repeat(12_001);
        let (head, tail, char_count) =
            split_head_tail(&content, PruneBudget::PROVIDER_FOLLOWUP).expect("必须超预算");
        assert_eq!(char_count, 12_001);
        assert_eq!(head.chars().count(), 4_500);
        assert_eq!(tail.chars().count(), 2_500);
        assert_eq!(head.chars().next(), Some('中'));
        assert_eq!(tail.chars().next(), Some('中'));
    }
}

/// 单遍严格双向配对扫描：只判定是否需要修复，不构造新列表（零 clone 快路径）。
#[derive(Default)]
struct RoundtripScan {
    pending_tool_call_ids: Vec<String>,
    needs_repair: bool,
}

impl RoundtripScan {
    fn observe(&mut self, message: &Value) {
        match message.get("role").and_then(Value::as_str) {
            Some("assistant") => {
                // 出现新 assistant 时仍有挂起调用 → 前一轮未闭合。
                if !self.pending_tool_call_ids.is_empty() {
                    self.needs_repair = true;
                    self.pending_tool_call_ids.clear();
                }
                if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
                    self.pending_tool_call_ids
                        .extend(calls.iter().filter_map(|call| {
                            call.get("id").and_then(Value::as_str).map(str::to_string)
                        }));
                }
            }
            Some("tool") => {
                let Some(tool_call_id) = message.get("tool_call_id").and_then(Value::as_str) else {
                    self.needs_repair = true;
                    return;
                };
                if self
                    .pending_tool_call_ids
                    .iter()
                    .any(|pending| pending == tool_call_id)
                {
                    self.pending_tool_call_ids
                        .retain(|existing| existing != tool_call_id);
                } else {
                    self.needs_repair = true;
                }
            }
            // user / system / developer 都是对话边界：跨边界不允许悬挂工具调用。
            Some("user") | Some("system") | Some("developer") => {
                if !self.pending_tool_call_ids.is_empty() {
                    self.needs_repair = true;
                    self.pending_tool_call_ids.clear();
                }
            }
            _ => {}
        }
    }

    fn finish(mut self) -> bool {
        if !self.pending_tool_call_ids.is_empty() {
            self.needs_repair = true;
        }
        self.needs_repair
    }
}

/// 判定 transcript 是否需要工具往返修复（与构造器共用同一配对语义）。
pub fn needs_tool_roundtrip_repair(transcript: &[Value]) -> bool {
    let mut scan = RoundtripScan::default();
    for message in transcript {
        scan.observe(message);
    }
    scan.finish()
}

fn interrupted_tool_result(tool_call_id: &str) -> Value {
    serde_json::json!({
        "role": "tool",
        "tool_call_id": tool_call_id,
        "content": "{\"status\":\"interrupted\",\"message\":\"Tool execution was interrupted before returning a result.\"}"
    })
}

fn interrupted_turn_checkpoint() -> Value {
    serde_json::json!({
        "role": "assistant",
        "content": "[中断轮次检查点: 前序工具调用在返回前被中断]",
        "reasoning_content": "（本轮在工具调用完成前被中断，无有效推理链）"
    })
}

fn flush_pending_tool_calls(
    result: &mut Vec<Value>,
    pending: &mut Vec<String>,
    with_checkpoint: bool,
) {
    if pending.is_empty() {
        return;
    }
    for id in pending.drain(..) {
        result.push(interrupted_tool_result(&id));
    }
    if with_checkpoint {
        result.push(interrupted_turn_checkpoint());
    }
}

/// 单遍严格双向配对状态机（借鉴 DSH ToolCallRecovery 与 OpenAI Chat 协议契约）：
/// 1. 过滤并丢弃孤儿 tool 消息（无对应的先行 assistant tool_calls）；
/// 2. 连续 assistant 时，若前一个 assistant 仍有未闭合的 tool_calls，立即在其之前就地补齐 synthetic tool 占位，恢复严格紧邻性；
/// 3. user / system / developer 边界或序列末尾时，补齐未闭合的 synthetic tool 结果与 [中断轮次检查点] assistant 消息；
/// 4. 返回修补后的消息列表以及是否发生过修改的布尔标志。
pub fn repair_and_filter_tool_roundtrips(transcript: &[Value]) -> (Vec<Value>, bool) {
    if !needs_tool_roundtrip_repair(transcript) {
        return (transcript.to_vec(), false);
    }

    let mut result = Vec::with_capacity(transcript.len() + 4);
    let mut pending_tool_call_ids: Vec<String> = Vec::new();

    for msg in transcript {
        match msg.get("role").and_then(Value::as_str) {
            Some("assistant") => {
                // 新 assistant 之前必须闭合前一轮工具往返（只补 tool 结果，assistant 紧随其后）。
                flush_pending_tool_calls(&mut result, &mut pending_tool_call_ids, false);
                if let Some(calls) = msg.get("tool_calls").and_then(Value::as_array) {
                    for call in calls {
                        if let Some(id) = call.get("id").and_then(Value::as_str) {
                            pending_tool_call_ids.push(id.to_string());
                        }
                    }
                }
                result.push(msg.clone());
            }
            Some("tool") => {
                let tool_call_id = msg.get("tool_call_id").and_then(Value::as_str);
                match tool_call_id {
                    Some(id) if pending_tool_call_ids.iter().any(|pending| pending == id) => {
                        pending_tool_call_ids.retain(|existing| existing != id);
                        result.push(msg.clone());
                    }
                    // 孤儿 tool 消息：丢弃，绝不透传给模型以防 400。
                    _ => {}
                }
            }
            Some("user") | Some("system") | Some("developer") => {
                flush_pending_tool_calls(&mut result, &mut pending_tool_call_ids, true);
                result.push(msg.clone());
            }
            _ => {
                // 未知角色不构成对话边界，也不得跨越悬挂调用插入（保持原序）。
                result.push(msg.clone());
            }
        }
    }

    flush_pending_tool_calls(&mut result, &mut pending_tool_call_ids, true);
    (result, true)
}

/// 自动补齐未闭合工具往返（对外统一包装）
pub fn recover_incomplete_tool_roundtrips(transcript: &[Value]) -> Vec<Value> {
    repair_and_filter_tool_roundtrips(transcript).0
}
