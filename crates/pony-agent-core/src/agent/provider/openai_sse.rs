// openai_sse: OpenAI SSE 流式响应累积器（OpenAiSseAccumulator）与相关类型的独立模块。
// 由 provider/mod.rs 拆分而来，行为与结构保持一致。
use super::*;

#[derive(Debug, Clone)]
pub(crate) struct OpenAiStreamMessage {
    pub(crate) output_text: String,
    pub(crate) tool_call: Option<ToolCall>,
    pub(crate) reasoning_content: Option<String>,
    pub(crate) reasoning_content_value: Option<Value>,
    pub(crate) token_usage: Option<TokenUsage>,
}

#[derive(Debug, Default)]
struct OpenAiSseAccumulator {
    output_text: String,
    reasoning_content: String,
    reasoning_content_value: Option<Value>,
    tool_calls: BTreeMap<usize, PartialOpenAiToolCall>,
    token_usage: Option<TokenUsage>,
    saw_data: bool,
}

#[derive(Debug, Default)]
struct PartialOpenAiToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl OpenAiSseAccumulator {
    fn push_payload<F>(&mut self, payload: &str, on_delta: &mut F) -> Result<bool, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        if payload == "[DONE]" {
            return Ok(true);
        }

        self.saw_data = true;
        let value = serde_json::from_str::<Value>(payload).map_err(|error| {
            format!(
                "解析 provider SSE chunk 失败: {}; 原始 chunk: {}",
                error, payload
            )
        })?;

        if let Some(token_usage) = extract_openai_usage(&value) {
            self.token_usage = Some(token_usage);
        }

        let delta = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("delta"));

        let delta_text = delta
            .and_then(extract_openai_delta_content_text)
            .unwrap_or_default();
        let delta_reasoning_value = delta.and_then(extract_openai_delta_reasoning_value);
        let delta_reasoning = delta
            .and_then(extract_openai_delta_reasoning_content)
            .unwrap_or_default();
        if let Some(delta) = delta {
            merge_openai_stream_tool_calls(&mut self.tool_calls, delta);
        }

        if !delta_text.is_empty() {
            self.output_text.push_str(&delta_text);
            on_delta(ProviderStreamChunk::Text(delta_text));
        }

        if !delta_reasoning.is_empty() {
            self.reasoning_content.push_str(&delta_reasoning);
            on_delta(ProviderStreamChunk::Reasoning(delta_reasoning));
        }
        // 字符串分片（如 DeepSeek 思考模式）逐片到达，value 只需在 finish() 中
        // 用完整累计文本回填；结构化值（如数组块）保留首块语义。
        if let Some(value) = delta_reasoning_value {
            if !matches!(value, Value::String(_)) && self.reasoning_content_value.is_none() {
                self.reasoning_content_value = Some(value);
            }
        }

        Ok(false)
    }

    fn finish(self, response_preview: &str) -> Result<OpenAiStreamMessage, String> {
        let OpenAiSseAccumulator {
            output_text,
            reasoning_content,
            reasoning_content_value,
            tool_calls,
            token_usage,
            saw_data,
        } = self;
        if !saw_data {
            return Err(format!(
                "provider 流式返回中未找到 SSE data 事件；响应预览: {}",
                response_preview
            ));
        }

        let tool_call = tool_calls
            .into_iter()
            .next()
            .and_then(|(_, partial)| partial_openai_tool_call_to_tool_call(partial));

        if output_text.is_empty() && tool_call.is_none() {
            return Err(format!(
                "provider 流式返回中未提取到文本内容；响应预览: {}",
                response_preview
            ));
        }

        // DeepSeek 等 thinking 模式要求后续请求回传完整的 reasoning_content，
        // 因此字符串类型的 value 必须用累计的完整思考文本回填，而不是首个分片。
        let final_reasoning_value = if reasoning_content_value
            .as_ref()
            .is_none_or(|value| matches!(value, Value::String(_)))
            && !reasoning_content.trim().is_empty()
        {
            Some(Value::String(reasoning_content.clone()))
        } else {
            reasoning_content_value.or_else(|| {
                (!reasoning_content.trim().is_empty())
                    .then(|| Value::String(reasoning_content.clone()))
            })
        };

        Ok(OpenAiStreamMessage {
            output_text,
            tool_call,
            reasoning_content: if reasoning_content.trim().is_empty() {
                None
            } else {
                Some(reasoning_content.clone())
            },
            reasoning_content_value: final_reasoning_value,
            token_usage,
        })
    }
}

#[cfg(test)]
pub(crate) fn collect_openai_sse_message_from_reader<R, F>(
    reader: R,
    response_preview: &str,
    on_delta: &mut F,
) -> Result<OpenAiStreamMessage, String>
where
    R: BufRead,
    F: FnMut(ProviderStreamChunk),
{
    let mut accumulator = OpenAiSseAccumulator::default();

    for line in reader.lines() {
        let line = line.map_err(|error| format!("读取 provider SSE 数据失败: {}", error))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Some(data) = trimmed.strip_prefix("data:") else {
            continue;
        };

        if accumulator.push_payload(data.trim(), on_delta)? {
            break;
        }
    }

    accumulator.finish(response_preview)
}

pub(crate) fn collect_openai_sse_message_from_response<F>(
    response: reqwest::Response,
    started_at: Instant,
    endpoint: &str,
    on_delta: &mut F,
) -> Result<OpenAiStreamMessage, String>
where
    F: FnMut(ProviderStreamChunk),
{
    let response_preview = endpoint;
    let mut accumulator = OpenAiSseAccumulator::default();
    let mut line_buf: Vec<u8> = Vec::new();
    let mut total_bytes: usize = 0;

    let mut stream = response.bytes_stream();

    loop {
        match block_on(stream.next()) {
            Some(Ok(bytes)) => {
                let chunk_len = bytes.len();
                for &byte in bytes.iter() {
                    if byte == b'\n' {
                        if !line_buf.is_empty() {
                            let line_str = String::from_utf8_lossy(&line_buf);
                            let trimmed = line_str.trim();
                            if let Some(data) = trimmed.strip_prefix("data:") {
                                if accumulator.push_payload(data.trim(), on_delta)? {
                                    return accumulator.finish(response_preview).map_err(|error| {
                                        format!(
                                            "解析 provider SSE 流失败: {}; elapsed={}ms; endpoint={}",
                                            error,
                                            started_at.elapsed().as_millis(),
                                            endpoint,
                                        )
                                    });
                                }
                            }
                        }
                        line_buf.clear();
                    } else {
                        if line_buf.len() >= 1 << 20 {
                            return Err(format!(
                                "provider SSE 行缓冲超过 1MB 上限; elapsed={}ms; endpoint={}",
                                started_at.elapsed().as_millis(),
                                endpoint,
                            ));
                        }
                        line_buf.push(byte);
                    }
                }
                total_bytes += chunk_len;
            }
            Some(Err(error)) => {
                return Err(format!(
                    "读取 provider SSE 流失败: {}; elapsed={}ms; parsed_bytes={}; endpoint={}",
                    error,
                    started_at.elapsed().as_millis(),
                    total_bytes,
                    endpoint,
                ));
            }
            None => break,
        };
    }

    if !line_buf.is_empty() {
        let line_str = String::from_utf8_lossy(&line_buf);
        let trimmed = line_str.trim();
        if let Some(data) = trimmed.strip_prefix("data:") {
            accumulator.push_payload(data.trim(), on_delta)?;
        }
    }

    accumulator.finish(response_preview).map_err(|error| {
        format!(
            "解析 provider SSE 流失败: {}; elapsed={}ms; endpoint={}",
            error,
            started_at.elapsed().as_millis(),
            endpoint,
        )
    })
}

#[cfg(test)]
pub(crate) fn collect_openai_sse_message<F>(
    raw_text: &str,
    on_delta: &mut F,
) -> Result<OpenAiStreamMessage, String>
where
    F: FnMut(ProviderStreamChunk),
{
    let mut combined = String::new();
    let mut reasoning = String::new();
    let mut reasoning_value = None;
    let mut saw_data = false;

    for line in raw_text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Some(data) = trimmed.strip_prefix("data:") else {
            continue;
        };

        let payload = data.trim();
        if payload == "[DONE]" {
            break;
        }

        saw_data = true;
        let value = serde_json::from_str::<Value>(payload).map_err(|error| {
            format!(
                "解析 provider SSE chunk 失败：{}；原始 chunk：{}",
                error, payload
            )
        })?;

        let delta = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("delta"));

        let delta_text = delta
            .and_then(extract_openai_delta_content_text)
            .unwrap_or_default();
        let delta_reasoning_value = delta.and_then(extract_openai_delta_reasoning_value);
        let delta_reasoning = delta
            .and_then(extract_openai_delta_reasoning_content)
            .unwrap_or_default();

        if !delta_text.is_empty() {
            combined.push_str(&delta_text);
            on_delta(ProviderStreamChunk::Text(delta_text));
        }

        if !delta_reasoning.is_empty() {
            reasoning.push_str(&delta_reasoning);
            on_delta(ProviderStreamChunk::Reasoning(delta_reasoning));
        }
        // 与 OpenAiSseAccumulator 保持一致：字符串分片只累计文本，
        // value 在下方用完整文本回填；结构化值保留首块。
        if let Some(value) = delta_reasoning_value {
            if !matches!(value, Value::String(_)) && reasoning_value.is_none() {
                reasoning_value = Some(value);
            }
        }
    }

    if !saw_data {
        return Err(format!(
            "provider 流式返回中未找到 SSE data 事件；原始响应：{}",
            raw_text
        ));
    }

    if combined.is_empty() {
        return Err(format!(
            "provider 流式返回中未提取到文本内容；原始响应：{}",
            raw_text
        ));
    }

    let final_reasoning_value = if reasoning_value
        .as_ref()
        .is_none_or(|value| matches!(value, Value::String(_)))
        && !reasoning.trim().is_empty()
    {
        Some(Value::String(reasoning.clone()))
    } else {
        reasoning_value
            .or_else(|| (!reasoning.trim().is_empty()).then(|| Value::String(reasoning.clone())))
    };

    Ok(OpenAiStreamMessage {
        output_text: combined,
        tool_call: None,
        reasoning_content: if reasoning.trim().is_empty() {
            None
        } else {
            Some(reasoning.clone())
        },
        reasoning_content_value: final_reasoning_value,
        token_usage: None,
    })
}

fn merge_openai_stream_tool_calls(
    tool_calls: &mut BTreeMap<usize, PartialOpenAiToolCall>,
    delta: &Value,
) {
    let Some(items) = delta.get("tool_calls").and_then(Value::as_array) else {
        return;
    };

    for item in items {
        let index = item
            .get("index")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(0);
        let partial = tool_calls.entry(index).or_default();

        if let Some(id) = item.get("id").and_then(Value::as_str) {
            if !id.is_empty() {
                partial.id = Some(id.to_string());
            }
        }

        if let Some(function) = item.get("function") {
            // 只在 name 非空时覆盖。ppx 等 provider 可能在后续 chunk 中发送空 name，
            // 导致之前已经正确设置的 name 被覆盖。
            if let Some(name) = function.get("name").and_then(Value::as_str) {
                let trimmed = name.trim();
                if partial.name.is_none() || !trimmed.is_empty() {
                    partial.name = Some(trimmed.to_string());
                }
            }

            if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                partial.arguments.push_str(arguments);
            }
        }

        // 诊断：当 tool call item 包含 arguments 但缺少 function.name 时，
        // 说明 provider 返回了非标准格式，打印原始 delta 用于排查。
        let has_arguments = item.get("arguments").is_some()
            || item
                .get("function")
                .and_then(|f| f.get("arguments"))
                .is_some();
        let has_name_via_function = item
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(Value::as_str)
            .map_or(false, |n| !n.is_empty());
        let has_name_direct = item
            .get("name")
            .and_then(Value::as_str)
            .map_or(false, |n| !n.is_empty());
        if has_arguments && !has_name_via_function && !has_name_direct {
            provider_log(format!(
                "provider:sse-tool-call-missing-name index={} item_preview={}",
                index,
                preview_text(&item.to_string(), 400)
            ));
        }
    }
}

fn partial_openai_tool_call_to_tool_call(partial: PartialOpenAiToolCall) -> Option<ToolCall> {
    let name = openai_original_tool_name(partial.name.as_deref()?);
    let arguments = if partial.arguments.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str::<Value>(&partial.arguments)
            .map(|v| if v.is_null() { json!({}) } else { v })
            .unwrap_or_else(|_| json!({}))
    };

    Some(ToolCall {
        call_id: partial.id,
        name,
        arguments,
        plan: None,
    })
}

fn extract_openai_delta_content_text(delta: &Value) -> Option<String> {
    match delta.get("content")? {
        Value::String(text) => Some(text.to_string()),
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
                        .map(str::to_string)
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

fn extract_openai_delta_reasoning_content(delta: &Value) -> Option<String> {
    extract_openai_delta_reasoning_value(delta)
        .as_ref()
        .and_then(extract_reasoning_text_from_value)
}

fn extract_openai_delta_reasoning_value(delta: &Value) -> Option<Value> {
    delta.get("reasoning_content").cloned()
}
