// responses_api: OpenAI Responses API 适配层（protocol = "openai-responses"）。
//
// 架构裁决（design D2 / R1 / R5）：
// - 存储与传输中枢统一为 chat 形态：assistant_message 在 sync/stream 两路都产出
//   `{role:"assistant", content, reasoning_content?, tool_calls[]}`；转换只发生在
//   `send_responses_*` 的请求/响应边界。
// - tool_calls[].id 一律取 Responses wire 的 **call_id** 字段（禁用 output item id），
//   sync 与 stream 同一口径，否则工具跟进 `function_call_output.call_id` 必挂 400。
// - SSE 帧循环复用公共 `sse_data_line_reader`，拒绝第二份帧骨架。
use super::*;

/// 公共 SSE 帧阅读器：字节缓冲 / 1MB 行上限 / `data:` 前缀提取 / 尾行冲刷 /
/// 流错误与解析错误的 elapsed 包装。`on_data_line` 返回 `Ok(true)` 表示提前结束
/// （如 `[DONE]` 或终态事件），阅读器随即停止读取。
pub(crate) fn sse_data_line_reader<F>(
    response: reqwest::Response,
    started_at: Instant,
    endpoint: &str,
    on_data_line: F,
) -> Result<(), String>
where
    F: FnMut(&str) -> Result<bool, String>,
{
    read_sse_data_lines(
        BlockingChunks {
            stream: response.bytes_stream(),
            _marker: std::marker::PhantomData,
        },
        started_at,
        endpoint,
        on_data_line,
    )
}

/// futures Stream → Iterator 适配（逐 chunk block_on），使帧循环可对任意
/// chunk 序列（含测试用内存序列）复用；不点名具体 chunk 类型，避免引入
/// bytes 直接依赖。
struct BlockingChunks<S, B, E> {
    stream: S,
    _marker: std::marker::PhantomData<fn() -> (B, E)>,
}

impl<S, B, E> Iterator for BlockingChunks<S, B, E>
where
    S: futures_util::Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
    E: std::fmt::Display,
{
    type Item = Result<B, E>;

    fn next(&mut self) -> Option<Self::Item> {
        block_on(futures_util::StreamExt::next(&mut self.stream))
    }
}

/// 帧循环核心：对任意「chunk 字节序列 + 可 Display 错误」的迭代器工作，
/// 供生产（reqwest 流）与单测（内存分片，含 CRLF/跨包/无尾换行/超限）共用。
fn read_sse_data_lines<I, B, E, F>(
    mut chunks: I,
    started_at: Instant,
    endpoint: &str,
    mut on_data_line: F,
) -> Result<(), String>
where
    I: Iterator<Item = Result<B, E>>,
    B: AsRef<[u8]>,
    E: std::fmt::Display,
    F: FnMut(&str) -> Result<bool, String>,
{
    let mut line_buf: Vec<u8> = Vec::new();
    let mut total_bytes: usize = 0;

    loop {
        match chunks.next() {
            Some(Ok(bytes)) => {
                let chunk_len = bytes.as_ref().len();
                for &byte in bytes.as_ref().iter() {
                    if byte == b'\n' {
                        if !line_buf.is_empty() {
                            let line_str = String::from_utf8_lossy(&line_buf);
                            let trimmed = line_str.trim();
                            if let Some(data) = trimmed.strip_prefix("data:") {
                                if on_data_line(data.trim())? {
                                    return Ok(());
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

    // 尾行冲刷：流以无换行 data 行结束时也要处理。
    if !line_buf.is_empty() {
        let line_str = String::from_utf8_lossy(&line_buf);
        let trimmed = line_str.trim();
        if let Some(data) = trimmed.strip_prefix("data:") {
            on_data_line(data.trim())?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Responses SSE 累积器
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct ResponsesStreamMessage {
    pub(crate) output_text: String,
    pub(crate) tool_call: Option<ToolCall>,
    pub(crate) reasoning_content: Option<String>,
    pub(crate) reasoning_content_value: Option<Value>,
    pub(crate) token_usage: Option<TokenUsage>,
    /// 多 function_call 时被丢弃的数量（首个之外）。
    pub(crate) dropped_function_calls: usize,
}

#[derive(Debug, Default)]
pub(crate) struct ResponsesSseAccumulator {
    output_text: String,
    reasoning_content: String,
    function_calls: Vec<PartialResponsesToolCall>,
    token_usage: Option<TokenUsage>,
    saw_event: bool,
}

#[derive(Debug, Default, Clone)]
struct PartialResponsesToolCall {
    item_id: Option<String>,
    call_id: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl PartialResponsesToolCall {
    fn matches_item(&self, item_id: &str) -> bool {
        self.item_id.as_deref() == Some(item_id)
    }

    fn is_complete(&self) -> bool {
        self.name
            .as_deref()
            .map(str::trim)
            .is_some_and(|name| !name.is_empty())
    }
}

impl ResponsesSseAccumulator {
    /// 处理一条 `data:` 负载；返回 `Ok(true)` 表示流应结束。
    pub(crate) fn push_payload<F>(
        &mut self,
        payload: &str,
        on_delta: &mut F,
    ) -> Result<bool, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        if payload == "[DONE]" {
            // 容错：部分网关在 response.completed 后仍补发 [DONE]。
            return Ok(true);
        }

        let value = serde_json::from_str::<Value>(payload).map_err(|error| {
            format!(
                "解析 provider SSE chunk 失败: {}; 原始 chunk: {}",
                error, payload
            )
        })?;
        self.saw_event = true;

        let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
        match event_type {
            "response.output_text.delta" => {
                if let Some(text) = value.get("delta").and_then(Value::as_str) {
                    if !text.is_empty() {
                        self.output_text.push_str(text);
                        on_delta(ProviderStreamChunk::Text(text.to_string()));
                    }
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                if let Some(text) = value.get("delta").and_then(Value::as_str) {
                    if !text.is_empty() {
                        self.reasoning_content.push_str(text);
                        on_delta(ProviderStreamChunk::Reasoning(text.to_string()));
                    }
                }
            }
            "response.function_call_arguments.delta" => {
                let item_id = value
                    .get("item_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let arguments = value
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !arguments.is_empty() || !item_id.is_empty() {
                    // 空 item_id 的分片归并到首个 partial（网关剥离 item_id 时参数不碎裂）。
                    let target = self
                        .function_calls
                        .iter_mut()
                        .find(|call| item_id.is_empty() || call.matches_item(&item_id));
                    if let Some(partial) = target {
                        if partial.item_id.is_none() && !item_id.is_empty() {
                            partial.item_id = Some(item_id.clone());
                        }
                        partial.arguments.push_str(arguments);
                    } else {
                        self.function_calls.push(PartialResponsesToolCall {
                            item_id: (!item_id.is_empty()).then_some(item_id),
                            call_id: None,
                            name: None,
                            arguments: arguments.to_string(),
                        });
                    }
                }
            }
            "response.output_item.done" => {
                let item = value.get("item").cloned().unwrap_or(Value::Null);
                if item.get("type").and_then(Value::as_str) == Some("function_call") {
                    let item_id = item
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    // 兜底补全 name/call_id；call_id 一律取 wire call_id 字段。
                    let call_id = item
                        .get("call_id")
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string);
                    let name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .map(openai_original_tool_name);
                    let arguments = item
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();

                    let exact = self
                        .function_calls
                        .iter()
                        .position(|call| call.matches_item(&item_id));
                    let adopt_index = exact.or_else(|| {
                        // 网关剥离 item_id 的场景（code-review A/P3-2）：
                        // 唯一未完成的 partial 即该调用的宿主，收养补全而非另起新项。
                        if self.function_calls.len() == 1 && !self.function_calls[0].is_complete() {
                            Some(0)
                        } else {
                            None
                        }
                    });
                    match adopt_index.and_then(|index| self.function_calls.get_mut(index)) {
                        Some(partial) => {
                            if partial.item_id.is_none() && !item_id.is_empty() {
                                partial.item_id = Some(item_id.clone());
                            }
                            if let Some(call_id) = call_id {
                                partial.call_id = Some(call_id);
                            }
                            if let Some(name) = name {
                                partial.name = Some(name);
                            }
                            if partial.arguments.trim().is_empty() && !arguments.trim().is_empty() {
                                partial.arguments = arguments;
                            }
                        }
                        None => {
                            self.function_calls.push(PartialResponsesToolCall {
                                item_id: (!item_id.is_empty()).then_some(item_id),
                                call_id,
                                name,
                                arguments,
                            });
                        }
                    }
                }
            }
            "response.completed" | "response.response.completed" => {
                let response_body = value.get("response").unwrap_or(&value);
                if let Some(token_usage) = extract_responses_usage(response_body) {
                    self.token_usage = Some(token_usage);
                }
                return Ok(true);
            }
            // 失败终态（code-review A/P2-2）：显式报错而非把截断文本当成功，
            // 避免无文本时触发 stream→sync 对已失败请求二次扣费。
            "response.failed" | "response.incomplete" | "error" => {
                let detail = value
                    .pointer("/response/error/message")
                    .or_else(|| value.pointer("/error/message"))
                    .or_else(|| value.get("delta"))
                    .or_else(|| value.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown responses stream failure");
                return Err(format!(
                    "responses 流式返回失败终态（{event_type}）: {detail}"
                ));
            }
            _ => {}
        }

        Ok(false)
    }

    pub(crate) fn finish(self, response_preview: &str) -> Result<ResponsesStreamMessage, String> {
        if !self.saw_event {
            return Err(format!(
                "provider 流式返回中未找到 SSE data 事件；响应预览: {}",
                response_preview
            ));
        }

        let dropped_function_calls = self.function_calls.len().saturating_sub(1);
        if dropped_function_calls > 0 {
            provider_log(format!(
                "responses:stream multiple function calls; keeping first; dropped={}",
                dropped_function_calls
            ));
        }
        let tool_call = self
            .function_calls
            .into_iter()
            .find(PartialResponsesToolCall::is_complete)
            .map(partial_responses_tool_call_to_tool_call);

        if self.output_text.trim().is_empty() && tool_call.is_none() {
            return Err(format!(
                "provider 流式返回中未提取到文本内容；响应预览: {}",
                response_preview
            ));
        }

        let reasoning_content = if self.reasoning_content.trim().is_empty() {
            None
        } else {
            Some(self.reasoning_content.clone())
        };
        let reasoning_content_value = reasoning_content
            .as_ref()
            .map(|text| Value::String(text.clone()));

        Ok(ResponsesStreamMessage {
            output_text: self.output_text,
            tool_call,
            reasoning_content,
            reasoning_content_value,
            token_usage: self.token_usage,
            dropped_function_calls,
        })
    }
}

fn partial_responses_tool_call_to_tool_call(partial: PartialResponsesToolCall) -> ToolCall {
    let arguments = if partial.arguments.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str::<Value>(&partial.arguments)
            .map(|v| if v.is_null() { json!({}) } else { v })
            .unwrap_or_else(|_| json!({}))
    };

    ToolCall {
        // 统一口径：只认 wire call_id，缺省留空由上层回退本地占位。
        call_id: partial.call_id,
        name: partial.name.unwrap_or_default(),
        arguments,
        plan: None,
    }
}

#[cfg(test)]
pub(crate) fn collect_responses_sse_message<F>(
    raw_text: &str,
    on_delta: &mut F,
) -> Result<ResponsesStreamMessage, String>
where
    F: FnMut(ProviderStreamChunk),
{
    let mut accumulator = ResponsesSseAccumulator::default();
    for line in raw_text.lines() {
        let trimmed = line.trim();
        if let Some(data) = trimmed.strip_prefix("data:") {
            if accumulator.push_payload(data.trim(), on_delta)? {
                break;
            }
        }
    }
    accumulator.finish(raw_text)
}

/// 从 HTTP 响应流收集 Responses SSE 消息（帧循环复用 sse_data_line_reader）。
#[allow(dead_code)]
pub(crate) fn collect_responses_sse_message_from_response<F>(
    response: reqwest::Response,
    started_at: Instant,
    endpoint: &str,
    on_delta: &mut F,
) -> Result<ResponsesStreamMessage, String>
where
    F: FnMut(ProviderStreamChunk),
{
    let response_preview = endpoint;
    let mut accumulator = Some(ResponsesSseAccumulator::default());
    let mut finished: Option<Result<ResponsesStreamMessage, String>> = None;

    let read_result = sse_data_line_reader(response, started_at, endpoint, |data| {
        let acc = accumulator
            .as_mut()
            .expect("accumulator present until terminal finish");
        if acc.push_payload(data, on_delta)? {
            let acc = accumulator.take().expect("accumulator taken once");
            finished = Some(acc.finish(response_preview));
            return Ok(true);
        }
        Ok(false)
    });

    match finished {
        Some(result) => result,
        None => {
            read_result?;
            let acc = accumulator.expect("accumulator still present at end of stream");
            acc.finish(response_preview)
        }
    }
}

// ---------------------------------------------------------------------------
// chat 形态 → Responses input 转换器
// ---------------------------------------------------------------------------

/// 外来形态守卫：content 为数组且首块 type ∈ {text, tool_use, tool_result,
/// thinking} 视为 anthropic blocks 形态（design D2）。chat 形态的 assistant/tool/user
/// 消息 content 均为字符串；openai 图片块数组的宿主消息不会进入 native transcript
/// 的 assistant/tool 位点，且跨协议切换允许丢上下文（现状语义）。
/// thinking 需显式列入：anthropic reasoning 转录的 assistant 消息以 thinking 块
/// 先行（provider_native_assistant_tool_call_message_for_protocol 构造顺序），
/// 只查首块会绕过守卫导致该消息被静默丢弃（code-review A/P3-1）。
fn is_foreign_native_message(message: &Value) -> bool {
    message
        .get("content")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|block| block.get("type").and_then(Value::as_str))
        .is_some_and(|block_type| {
            matches!(block_type, "text" | "tool_use" | "tool_result" | "thinking")
        })
}

/// 把 chat 形态消息列表转换为 Responses input items。
/// 任一消息为外来 anthropic blocks 形态时返回 `None`（skip-and-rebuild 信号）。
fn chat_messages_to_responses_input(messages: &[Value]) -> Option<Vec<Value>> {
    if messages
        .iter()
        .any(|message| is_foreign_native_message(message))
    {
        return None;
    }

    let mut items: Vec<Value> = Vec::new();
    for message in messages {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");
        match role {
            "assistant" => {
                if let Some(text) = message.get("content").and_then(Value::as_str) {
                    if !text.trim().is_empty() {
                        items.push(json!({
                            "role": "assistant",
                            "content": [{ "type": "output_text", "text": text }]
                        }));
                    }
                }

                for call in message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .unwrap_or(&Vec::new())
                {
                    let function = call.get("function");
                    let name = function
                        .and_then(|value| value.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let arguments = function
                        .and_then(|value| value.get("arguments"))
                        .and_then(Value::as_str)
                        .unwrap_or("{}");
                    let call_id = call
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                        .unwrap_or("tool_call_local");
                    items.push(json!({
                        "type": "function_call",
                        "call_id": call_id,
                        "name": name,
                        "arguments": arguments
                    }));
                }
            }
            "tool" => {
                let call_id = message
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("tool_call_local");
                let output = message.get("content").and_then(Value::as_str).unwrap_or("");
                items.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": output
                }));
            }
            "user" | "system" | "developer" => {
                if let Some(text) = message.get("content").and_then(Value::as_str) {
                    items.push(json!({
                        "role": role,
                        "content": [{ "type": "input_text", "text": text }]
                    }));
                }
            }
            _ => {}
        }
    }

    Some(items)
}

/// 从 ProviderRequest 构造 Responses input：
/// native_messages 为 chat 形态时逐条转换；检测到外来形态则 skip-and-rebuild
/// （丢弃外来项，从 request.input 重建）；为空时直接从 request.input 构建。
pub(crate) fn responses_input_from_request(request: &ProviderRequest) -> Vec<Value> {
    if !request.native_messages.is_empty() {
        if let Some(items) = chat_messages_to_responses_input(&request.native_messages) {
            return items;
        }
        provider_log(
            "responses:input foreign transcript shape detected; skip-and-rebuild from request input"
                .to_string(),
        );
    }

    build_input_from_provider_messages(request)
}

fn build_input_from_provider_messages(request: &ProviderRequest) -> Vec<Value> {
    let user_indexes = request
        .input
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            if matches!(message.role, ProviderRole::User) {
                Some(index)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let last_user_index = user_indexes.last().copied();

    request
        .input
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let include_images = !request.images.is_empty() && Some(index) == last_user_index;
            let mut content = Vec::new();
            if !message.content.trim().is_empty() || !include_images {
                content.push(json!({
                    "type": "input_text",
                    "text": message.content
                }));
            }
            if include_images {
                content.extend(request.images.iter().map(|image| {
                    json!({
                        "type": "input_image",
                        "image_url": image.data_url
                    })
                }));
            }
            json!({
                "role": to_chat_role(&message.role),
                "content": content
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// sync 请求构建 / 解析
// ---------------------------------------------------------------------------

/// Responses 工具定义：function 形态、name 平铺在 tool 对象上。
fn responses_tools_payload(tools: &[ToolDefinition]) -> Vec<Value> {
    let contract_views = if is_builtin_tool_definition_set(tools) {
        builtin_provider_contract_views()
    } else {
        tools.iter().map(|tool| tool.contract_view()).collect()
    };

    contract_views
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": openai_safe_tool_name(&tool.name),
                "description": tool.description,
                "parameters": tool.input_schema.clone()
            })
        })
        .collect()
}

/// 构建 Responses 请求体（sync/stream 共用输入构造，仅 stream 标志不同）。
///
/// - `store:false`：不依赖服务端会话状态；
/// - reasoning 模型附加 `"reasoning":{"effort":...}` 且此时省略 temperature
///   （o 系/GPT-5 拒收 temperature）；
/// - 工具能力裁剪沿用 completions 的 `apply_openai_tool_capability`。
pub(crate) fn build_responses_request_body(
    request: &ProviderRequest,
    tools: &[ToolDefinition],
    config: &ResolvedProviderSelection,
    stream: bool,
) -> Value {
    let mut body = json!({
        "model": request.model,
        "input": responses_input_from_request(request),
        "stream": stream,
        "store": false,
        "max_output_tokens": request.max_output_tokens,
    });

    let reasoning_model = config.capabilities.supports_reasoning;
    if reasoning_model {
        if let Some(effort) = config.reasoning_effort.as_ref() {
            body["reasoning"] = json!({ "effort": reasoning_effort_label(effort) });
        }
    } else {
        body["temperature"] = json!(request.temperature);
    }

    body["tools"] = Value::Array(responses_tools_payload(tools));
    body["tool_choice"] = json!("auto");

    apply_openai_tool_capability(body, tools, config)
}

#[derive(Debug, Clone)]
pub(crate) struct ResponsesSyncOutput {
    pub(crate) output_text: String,
    pub(crate) tool_call: Option<ToolCall>,
    pub(crate) reasoning_content: Option<String>,
    pub(crate) reasoning_content_value: Option<Value>,
    pub(crate) token_usage: Option<TokenUsage>,
    pub(crate) dropped_function_calls: usize,
}

/// Responses usage 映射：`{input_tokens, output_tokens, total_tokens,
/// reasoning_tokens?}`；若映射 `input_tokens_details.cached_tokens` 进
/// cache_hit_input_tokens 必须同设 cache_hit_source="responses"
/// （build_provider_call_cache_record 的 panic 契约）。
pub(crate) fn extract_responses_usage(payload: &Value) -> Option<TokenUsage> {
    let usage = payload.get("usage")?;
    provider_log(format!("usage:responses raw={}", usage));

    let cached_tokens = usage
        .get("input_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .and_then(Value::as_u64);

    Some(normalize_token_usage(TokenUsage {
        input_tokens: usage.get("input_tokens").and_then(Value::as_u64),
        cache_hit_input_tokens: cached_tokens,
        cache_hit_source: cached_tokens.map(|_| "responses".to_string()),
        reasoning_tokens: usage
            .get("output_tokens_details")
            .and_then(|details| details.get("reasoning_tokens"))
            .and_then(Value::as_u64),
        output_tokens: usage.get("output_tokens").and_then(Value::as_u64),
        total_tokens: usage.get("total_tokens").and_then(Value::as_u64),
    }))
}

/// 解析 Responses 同步响应：优先顶层 `output_text`；否则遍历 `output[]`
/// （message.content[].output_text 拼接、function_call → ToolCall、reasoning 汇总）。
/// 多 function_call 取首个并记丢弃数；ToolCall.call_id 一律取 wire call_id 字段。
pub(crate) fn parse_responses_output(payload: &Value) -> Result<ResponsesSyncOutput, String> {
    let mut text_parts: Vec<String> = Vec::new();
    if let Some(top_level) = payload.get("output_text").and_then(Value::as_str) {
        if !top_level.is_empty() {
            text_parts.push(top_level.to_string());
        }
    }

    let mut reasoning_parts: Vec<String> = Vec::new();
    let mut function_calls: Vec<PartialResponsesToolCall> = Vec::new();

    for item in payload
        .get("output")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
    {
        match item.get("type").and_then(Value::as_str).unwrap_or("") {
            "message" => {
                for block in item
                    .get("content")
                    .and_then(Value::as_array)
                    .unwrap_or(&Vec::new())
                {
                    if block.get("type").and_then(Value::as_str) == Some("output_text") {
                        if let Some(text) = block.get("text").and_then(Value::as_str) {
                            text_parts.push(text.to_string());
                        }
                    }
                }
            }
            "function_call" => {
                function_calls.push(PartialResponsesToolCall {
                    item_id: item.get("id").and_then(Value::as_str).map(str::to_string),
                    call_id: item
                        .get("call_id")
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string),
                    name: item
                        .get("name")
                        .and_then(Value::as_str)
                        .map(openai_original_tool_name),
                    arguments: item
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                });
            }
            "reasoning" => {
                for block in item
                    .get("summary")
                    .and_then(Value::as_array)
                    .unwrap_or(&Vec::new())
                {
                    if block.get("type").and_then(Value::as_str) == Some("summary_text") {
                        if let Some(text) = block.get("text").and_then(Value::as_str) {
                            reasoning_parts.push(text.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let dropped_function_calls = function_calls.len().saturating_sub(1);
    if dropped_function_calls > 0 {
        provider_log(format!(
            "responses:sync multiple function calls; keeping first; dropped={}",
            dropped_function_calls
        ));
    }
    let tool_call = function_calls
        .into_iter()
        .find(PartialResponsesToolCall::is_complete)
        .map(partial_responses_tool_call_to_tool_call);

    let output_text = text_parts.join("");
    if output_text.trim().is_empty() && tool_call.is_none() {
        return Err("responses 返回中缺少文本或工具调用".to_string());
    }

    let reasoning_content = if reasoning_parts.is_empty() {
        None
    } else {
        Some(reasoning_parts.join(""))
    };
    let reasoning_content_value = reasoning_content
        .as_ref()
        .map(|text| Value::String(text.clone()));

    Ok(ResponsesSyncOutput {
        output_text,
        tool_call,
        reasoning_content,
        reasoning_content_value,
        token_usage: extract_responses_usage(payload),
        dropped_function_calls,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_request_body_uses_store_false_and_flat_tool_names() {
        let request = ProviderRequest {
            model: "gpt-5.4".to_string(),
            input: vec![
                ProviderMessage::system("系统指令"),
                ProviderMessage::user("用户问题"),
            ],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };
        let config = responses_test_config(true, Some(ProviderReasoningEffort::Medium));

        let body = build_responses_request_body(
            &request,
            &[ToolDefinition {
                name: "my_custom_tool",
                description: "read file",
                input_schema: json!({ "type": "object" }),
            }],
            &config,
            false,
        );

        assert_eq!(body["model"], "gpt-5.4");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], false);
        assert_eq!(body["max_output_tokens"], 1024);
        // reasoning 模型：附 effort、省略 temperature。
        assert_eq!(body["reasoning"]["effort"], "medium");
        assert!(body.get("temperature").is_none());
        // tools function 形态：name 平铺。
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "my_custom_tool");
        assert!(body["tools"][0].get("parameters").is_some());
        assert_eq!(body["tool_choice"], "auto");

        // 非 reasoning 模型：带 temperature、不带 reasoning。
        let chat_config = responses_test_config(false, None);
        let chat_body = build_responses_request_body(&request, &[], &chat_config, true);
        assert!((chat_body["temperature"].as_f64().unwrap() - 0.2).abs() < 1e-6);
        assert!(chat_body.get("reasoning").is_none());
        assert_eq!(chat_body["stream"], true);
    }

    #[test]
    fn chat_messages_convert_to_responses_input_items() {
        let messages = vec![
            json!({ "role": "user", "content": "当前文件夹下有哪些文件？" }),
            json!({
                "role": "assistant",
                "content": "我先列出目录。",
                "reasoning_content": null,
                "tool_calls": [{
                    "id": "call_list",
                    "type": "function",
                    "function": { "name": "List", "arguments": "{\"path\":\".\"}" }
                }]
            }),
            json!({
                "role": "tool",
                "tool_call_id": "call_list",
                "content": "Cargo.toml\nsrc"
            }),
        ];

        let items = chat_messages_to_responses_input(&messages).expect("chat form converts");

        assert_eq!(items[0]["role"], "user");
        assert_eq!(items[0]["content"][0]["type"], "input_text");
        assert_eq!(items[1]["role"], "assistant");
        assert_eq!(items[1]["content"][0]["type"], "output_text");
        assert_eq!(items[2]["type"], "function_call");
        assert_eq!(items[2]["call_id"], "call_list");
        assert_eq!(items[2]["name"], "List");
        assert_eq!(items[3]["type"], "function_call_output");
        assert_eq!(items[3]["call_id"], "call_list");
        assert_eq!(items[3]["output"], "Cargo.toml\nsrc");
    }

    #[test]
    fn foreign_anthropic_blocks_trigger_skip_and_rebuild_from_input() {
        let request = ProviderRequest {
            model: "gpt-5.4".to_string(),
            input: vec![ProviderMessage::user("重建后的用户消息")],
            images: Vec::new(),
            native_messages: vec![
                json!({
                    "role": "assistant",
                    "content": [
                        { "type": "thinking", "thinking": "先读取目录。" },
                        { "type": "tool_use", "id": "toolu_1", "name": "List", "input": {} }
                    ]
                }),
                json!({
                    "role": "user",
                    "content": [
                        { "type": "tool_result", "tool_use_id": "toolu_1", "content": "Cargo.toml" }
                    ]
                }),
            ],
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };

        // 外来形态 → 整体丢弃 native transcript，从 request.input 重建。
        assert!(chat_messages_to_responses_input(&request.native_messages).is_none());
        let items = responses_input_from_request(&request);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["role"], "user");
        assert_eq!(items[0]["content"][0]["type"], "input_text");
        assert_eq!(items[0]["content"][0]["text"], "重建后的用户消息");
    }

    #[test]
    fn last_user_with_images_appends_input_image_block() {
        let request = ProviderRequest {
            model: "gpt-4.1-mini".to_string(),
            input: vec![
                ProviderMessage::system("看图"),
                ProviderMessage::user("请描述这张图"),
            ],
            images: vec![crate::agent::input::TurnInputImage {
                data_url: "data:image/png;base64,Zm9v".to_string(),
                mime_type: "image/png".to_string(),
                name: Some("demo.png".to_string()),
            }],
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };

        let items = responses_input_from_request(&request);
        assert_eq!(items.len(), 2);
        assert_eq!(items[1]["content"][0]["type"], "input_text");
        assert_eq!(items[1]["content"][1]["type"], "input_image");
        assert_eq!(
            items[1]["content"][1]["image_url"],
            "data:image/png;base64,Zm9v"
        );
    }

    #[test]
    fn responses_sse_accumulates_text_reasoning_and_function_arguments() {
        let raw_text = concat!(
            "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"先看结构\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"第 1 段\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"第 2 段\"}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\"{\\\"path\\\"\"}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\":\\\"Cargo.toml\\\"}\"}\n\n",
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_wire_1\",\"name\":\"Read\",\"arguments\":\"{\\\"path\\\":\\\"Cargo.toml\\\"}\"}}\n\n",
            "data: [DONE]\n\n"
        );
        let mut deltas = Vec::new();

        let message = collect_responses_sse_message(raw_text, &mut |delta| deltas.push(delta))
            .expect("responses stream should parse");

        assert_eq!(message.output_text, "第 1 段第 2 段");
        assert_eq!(message.reasoning_content.as_deref(), Some("先看结构"));
        assert_eq!(
            deltas.first(),
            Some(&ProviderStreamChunk::Reasoning("先看结构".to_string()))
        );
        let tool_call = message.tool_call.expect("function call captured");
        // 统一口径：call_id 取 wire call_id 字段，而非 output item id（fc_1）。
        assert_eq!(tool_call.call_id.as_deref(), Some("call_wire_1"));
        assert_eq!(tool_call.name, "Read");
        assert_eq!(tool_call.arguments["path"], "Cargo.toml");
        assert_eq!(message.dropped_function_calls, 0);
    }

    #[test]
    fn responses_sse_completed_event_supplies_usage_and_ends_stream() {
        let raw_text = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"答案\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":120,\"input_tokens_details\":{\"cached_tokens\":48},\"output_tokens\":32,\"output_tokens_details\":{\"reasoning_tokens\":12},\"total_tokens\":152}}}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"不应再累计\"}\n\n"
        );

        let message =
            collect_responses_sse_message(raw_text, &mut |_delta| {}).expect("stream parses");

        assert_eq!(message.output_text, "答案");
        let usage = message.token_usage.expect("completed supplies usage");
        assert_eq!(usage.input_tokens, Some(120));
        // cache_hit_input_tokens 映射自 cached_tokens 时必须同设 cache_hit_source="responses"。
        assert_eq!(usage.cache_hit_input_tokens, Some(48));
        assert_eq!(usage.cache_hit_source.as_deref(), Some("responses"));
        assert_eq!(usage.reasoning_tokens, Some(12));
        assert_eq!(usage.output_tokens, Some(32));
        assert_eq!(usage.total_tokens, Some(152));
    }

    #[test]
    fn responses_sync_parse_prefers_call_id_field_and_first_function_call_only() {
        let payload = json!({
            "output": [
                { "type": "reasoning", "summary": [{ "type": "summary_text", "text": "推理摘要" }] },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "先读文件。" }]
                },
                {
                    "type": "function_call",
                    "id": "fc_item_a",
                    "call_id": "call_a",
                    "name": "workspace.read_file",
                    "arguments": "{\"path\":\"a.rs\"}"
                },
                {
                    "type": "function_call",
                    "id": "fc_item_b",
                    "call_id": "call_b",
                    "name": "Read",
                    "arguments": "{}"
                }
            ],
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5,
                "total_tokens": 15
            }
        });

        let parsed = parse_responses_output(&payload).expect("sync output parses");

        assert_eq!(parsed.output_text, "先读文件。");
        assert_eq!(parsed.reasoning_content.as_deref(), Some("推理摘要"));
        let tool_call = parsed.tool_call.expect("first function call kept");
        // 禁用 output item id：call_id 只认 wire 字段。
        assert_eq!(tool_call.call_id.as_deref(), Some("call_a"));
        assert_eq!(tool_call.name, "workspace.read_file");
        assert_eq!(parsed.dropped_function_calls, 1);
        let usage = parsed.token_usage.expect("usage mapped");
        assert_eq!(usage.cache_hit_input_tokens, None);
        assert_eq!(usage.cache_hit_source, None);
        assert_eq!(usage.total_tokens, Some(15));
    }

    #[test]
    fn responses_sync_top_level_output_text_wins_without_output_items() {
        let payload = json!({
            "output_text": "顶层文本",
            "output": []
        });

        let parsed = parse_responses_output(&payload).expect("parses");
        assert_eq!(parsed.output_text, "顶层文本");
        assert!(parsed.tool_call.is_none());
    }

    #[test]
    fn responses_sync_missing_text_and_tool_call_is_error() {
        let payload = json!({ "output": [] });
        assert!(parse_responses_output(&payload)
            .err()
            .unwrap_or_default()
            .contains("缺少文本或工具调用"));
    }

    fn responses_test_config(
        supports_reasoning: bool,
        effort: Option<ProviderReasoningEffort>,
    ) -> ResolvedProviderSelection {
        ResolvedProviderSelection {
            requested_name: "test-responses".to_string(),
            provider_name: "test-responses".to_string(),
            protocol: ProviderProtocol::OpenAiResponses,
            base_url: "https://api.openai.com/v1".to_string(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "TEST_API_KEY".to_string(),
            api_key: Some("test-key".to_string()),
            model: "gpt-5.4".to_string(),
            temperature: 0.2,
            max_output_tokens: 1024,
            reasoning_effort: effort,
            reasoning_budget_tokens: None,
            capabilities: crate::agent::config::ProviderModelCapabilities {
                context_window_tokens: Some(128_000),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: true,
                supports_reasoning,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::EffortStandard,
        }
    }

    // ---- 帧循环核心直测（code-review A/P2-4：CRLF/跨包分片/尾冲刷/1MB 上限）----

    fn collect_lines(chunks: Vec<Result<Vec<u8>, String>>) -> Result<Vec<String>, String> {
        let mut lines = Vec::new();
        read_sse_data_lines(
            chunks.into_iter(),
            Instant::now(),
            "https://unit.test/v1/responses",
            |line| {
                lines.push(line.to_string());
                Ok(false)
            },
        )
        .map(|_| lines)
    }

    #[test]
    fn sse_reader_handles_crlf_and_split_chunks_with_trailing_flush() {
        // CRLF 行尾 + data 行跨 chunk 分片 + 末行无换行（依赖尾行冲刷）
        let lines = collect_lines(vec![
            Ok(b"data: {\"a\":1}\r\n".to_vec()),
            Ok(b"data: {\"b".to_vec()),
            Ok(b":2}\"\n\n".to_vec()),
            Ok(b"data: [DONE]".to_vec()),
        ])
        .expect("reader should succeed");

        assert_eq!(lines, vec!["{\"a\":1}", "{\"b:2}\"", "[DONE]"]);
    }

    #[test]
    fn sse_reader_wraps_stream_errors_with_elapsed_and_bytes() {
        let error = collect_lines(vec![
            Ok(b"data: first\n".to_vec()),
            Err("socket reset".to_string()),
        ])
        .expect_err("stream error should surface");

        assert!(error.contains("读取 provider SSE 流失败"));
        assert!(error.contains("socket reset"));
        assert!(error.contains("parsed_bytes=12"));
    }

    #[test]
    fn sse_reader_stops_early_when_handler_signals_done() {
        let mut lines = Vec::new();
        let chunks: Vec<Result<Vec<u8>, String>> =
            vec![Ok(b"data: one\ndata: two\ndata: three\n".to_vec())];
        let result = read_sse_data_lines(
            chunks.into_iter(),
            Instant::now(),
            "https://unit.test/v1/responses",
            |line| {
                lines.push(line.to_string());
                Ok(line == "two")
            },
        );

        assert!(result.is_ok());
        assert_eq!(lines, vec!["one", "two"]);
    }

    #[test]
    fn sse_reader_enforces_1mb_line_cap() {
        let error = collect_lines(vec![Ok(vec![b'x'; (1 << 20) + 8])])
            .expect_err("oversized line should be rejected");
        assert!(error.contains("1MB"));
    }

    #[test]
    fn sse_accumulator_treats_failed_terminal_as_error_even_with_partial_text() {
        let raw = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"部分文本\"}\n\n",
            "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"message\":\"quota exceeded\"}}}\n\n"
        );
        let mut deltas = Vec::new();
        let error = collect_responses_sse_message(raw, &mut |chunk| match chunk {
            ProviderStreamChunk::Text(text) => deltas.push(text),
            _ => {}
        })
        .expect_err("failed terminal must surface as error");

        assert!(error.contains("response.failed"));
        assert!(error.contains("quota exceeded"));
        assert_eq!(deltas.len(), 1);
    }

    #[test]
    fn sse_accumulator_merges_argument_deltas_without_item_id_into_single_call() {
        // 用 json! 构造负载避免多层手工转义；delta 片段拼出 {"path":1}。
        let ev = |value: Value| format!("data: {value}\n\n");
        let raw = format!(
            "{}{}{}{}",
            ev(json!({"type": "response.function_call_arguments.delta", "delta": "{\"pa"})),
            ev(json!({"type": "response.function_call_arguments.delta", "delta": "th\":1}"})),
            ev(json!({"type": "response.output_item.done", "item": {
                "type": "function_call", "id": "fc_9", "call_id": "call_wire_x", "name": "Read"
            }})),
            ev(json!({"type": "response.completed", "response": {
                "usage": {"input_tokens": 3, "output_tokens": 4}
            }})),
        );
        let message =
            collect_responses_sse_message(&raw, &mut |_| {}).expect("stream should parse");

        let tool_call = message.tool_call.expect("merged call should complete");
        assert_eq!(tool_call.name, "Read");
        assert_eq!(tool_call.call_id.as_deref(), Some("call_wire_x"));
        assert_eq!(tool_call.arguments, serde_json::json!({"path": 1}));
    }
}
