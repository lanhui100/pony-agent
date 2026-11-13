//! OpenAI 兼容线协议的轻量归一：SSE delta 文本提取与工具名合法化。
//!
//! 自 `provider/mod.rs` 拆分（file-size 收口，行为逐字等价）；
//! 父模块以 `pub(crate) use` 保持既有调用点不变。

use super::*;

#[allow(dead_code)]
pub(crate) fn extract_chat_delta_text(payload: &Value) -> Option<String> {
    let delta = payload
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("delta"))?;
    let content = delta.get("content")?;

    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let parts = items
                .iter()
                .filter_map(|item| {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        return Some(text.to_string());
                    }
                    item.get("text")
                        .and_then(|value| value.get("value"))
                        .and_then(Value::as_str)
                        .map(|text| text.to_string())
                })
                .collect::<Vec<_>>();

            if parts.is_empty() {
                None
            } else {
                Some(parts.join(""))
            }
        }
        _ => None,
    }
}

pub(crate) fn openai_safe_tool_name(name: &str) -> String {
    name.replace('.', "__dot__")
}
