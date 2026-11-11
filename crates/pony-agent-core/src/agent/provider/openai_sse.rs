// openai_sse: OpenAI SSE 流式响应累积器（OpenAiSseAccumulator）与相关类型的独立模块。
// 由 provider/mod.rs 拆分而来，行为与结构保持一致。
use super::*;

#[derive(Debug, Clone)]
pub(crate) struct OpenAiStreamMessage {
    pub(crate) output_text: String,
    /// D3b（P1-1 / B018）：本次流式响应携带的**整批**调用（按 SSE `index` 保序）。
    /// 单值字段 `tool_call` 仍是首元素的兼容视图（D1a 不变量），批执行读本字段。
    pub(crate) tool_calls: Vec<ToolCall>,
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
    /// ADR 0055 P0-2：首个 SSE data 事件是否已发出 `StreamStart` 时点信号。
    saw_first_chunk: bool,
    /// B060：缺 name 诊断降噪器（每个流式响应一个实例，随累积器生命周期）。
    missing_name_diagnostics: MissingNameDiagnostics,
}
#[derive(Debug, Default)]
struct PartialOpenAiToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

/// B060：`sse-tool-call-missing-name` 诊断降噪器（每个流式响应一个实例）。
/// 语义见 `.dev-team/contract-matrix-b060.md`：同一 index 每流至多输出 1 条诊断，
/// 重复次数只累计不打印，流结束时至多输出 1 条汇总。
#[derive(Debug, Default)]
pub(crate) struct MissingNameDiagnostics {
    seen_indexes: std::collections::BTreeSet<usize>,
    /// 至少被抑制过一次的 index 集合（汇总行口径：受抑制的不同 index 数）。
    suppressed_indexes: std::collections::BTreeSet<usize>,
    suppressed_total: u32,
    summary_taken: bool,
}

impl MissingNameDiagnostics {
    /// 判定本次观测是否必须输出诊断行：首次为 `true`，重复为 `false` 并累计抑制计数。
    pub(crate) fn observe(&mut self, index: usize) -> bool {
        if self.seen_indexes.insert(index) {
            return true;
        }
        self.suppressed_total = self.suppressed_total.saturating_add(1);
        self.suppressed_indexes.insert(index);
        false
    }

    /// 流结束时的汇总判定：存在被抑制的重复时返回 `(抑制总数, 受抑制 index 数)`，
    /// 且只返回一次（二次调用返回 `None`）。
    pub(crate) fn take_summary(&mut self) -> Option<(u32, usize)> {
        if self.summary_taken || self.suppressed_total() == 0 {
            return None;
        }
        self.summary_taken = true;
        Some((self.suppressed_total(), self.suppressed_indexes.len()))
    }

    /// B060 R2：构造本次观测的诊断行文本——首见返回 `Some(line)`（前缀与字段逐字保持），
    /// 重复返回 `None` 并累计抑制计数。
    pub(crate) fn diagnostic_line(&mut self, index: usize, item_preview: &str) -> Option<String> {
        unimplemented!("B060 R2: MissingNameDiagnostics::diagnostic_line pending implementation")
    }

    /// B060 R2：构造流结束汇总行文本——存在被抑制的重复时返回 `Some(line)` 且只返回一次；
    /// 文本前缀必须**不含** `sse-tool-call-missing-name` 子串（巡检针口径隔离，契约 C7）。
    pub(crate) fn summary_line(&mut self) -> Option<String> {
        unimplemented!("B060 R2: MissingNameDiagnostics::summary_line pending implementation")
    }

    /// 已抑制的累计条数（只读，供测试与汇总使用）。
    pub(crate) fn suppressed_total(&self) -> u32 {
        self.suppressed_total
    }
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

        // wave-tui-session-resilience C1：DSML 协议原文在解析层即被折叠，
        // 流式增量与最终累计文本都不再携带原始工具调用协议字面量。
        let delta_text = super::sanitize_dsml_protocol_text(&delta_text);
        let delta_reasoning_value = delta.and_then(extract_openai_delta_reasoning_value);
        let delta_reasoning = delta
            .and_then(extract_openai_delta_reasoning_content)
            .unwrap_or_default();
        if let Some(delta) = delta {
            merge_openai_stream_tool_calls(
                &mut self.tool_calls,
                &mut self.missing_name_diagnostics,
                delta,
            );
        }

        let mut emitted_content = false;
        if !delta_text.is_empty() {
            self.output_text.push_str(&delta_text);
            on_delta(ProviderStreamChunk::Text(delta_text));
            emitted_content = true;
        }

        if !delta_reasoning.is_empty() {
            self.reasoning_content.push_str(&delta_reasoning);
            on_delta(ProviderStreamChunk::Reasoning(delta_reasoning));
            emitted_content = true;
        }
        // ADR 0055 P0-2：首包时点信号。**仅当该首包不含 text/reasoning 增量时**才需要它
        // （纯工具调用轮次的后续 chunk 不会有内容增量，否则 ttft 永久缺失、速度分母退化成整段耗时）。
        // 首包本身带内容时既有 Text/Reasoning 增量已完成时点采集，不再额外发信号 ⇒
        // 既有「内容增量顺序」契约零变更。
        if !self.saw_first_chunk {
            self.saw_first_chunk = true;
            if !emitted_content {
                on_delta(ProviderStreamChunk::StreamStart);
            }
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
            saw_first_chunk: _,
            mut missing_name_diagnostics,
        } = self;

        // B060：流结束至多输出 1 条降噪汇总行（存在被抑制的重复诊断时才输出）。
        if let Some((suppressed_total, suppressed_indexes)) =
            missing_name_diagnostics.take_summary()
        {
            provider_log(format!(
                "provider:sse-tool-call-missing-name-suppressed suppressed_total={} suppressed_indexes={}",
                suppressed_total, suppressed_indexes
            ));
        }

        if !saw_data {
            return Err(format!(
                "provider 流式返回中未找到 SSE data 事件；响应预览: {}",
                response_preview
            ));
        }

        // D3b：不再丢弃多余调用——按 `index` 保序收集整批（空洞调用逐项跳过）。
        let tool_calls: Vec<ToolCall> = tool_calls
            .into_iter()
            .filter_map(|(_, partial)| partial_openai_tool_call_to_tool_call(partial))
            .collect();
        // 单值字段 = 首元素（D1a 兼容视图），既有单调用语义逐字不变。
        let tool_call = tool_calls.first().cloned();

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
            output_text: super::sanitize_dsml_protocol_text(&output_text),
            tool_calls,
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
    guard: &ProviderStreamGuard<'_>,
) -> Result<OpenAiStreamMessage, String>
where
    F: FnMut(ProviderStreamChunk),
{
    let response_preview = endpoint;
    // 帧循环复用公共 sse_data_line_reader（架构裁决 R5：拒绝第二份帧骨架）。
    // finish() 按 self 消费，这里用 Option 承载累积器。
    let mut accumulator = Some(OpenAiSseAccumulator::default());
    let mut finished: Option<Result<OpenAiStreamMessage, String>> = None;

    let read_result =
        super::responses_api::sse_data_line_reader_with_guard(
            response,
            started_at,
            endpoint,
            |data| {
            let acc = accumulator
                .as_mut()
                .expect("accumulator present until [DONE] finish");
            if acc.push_payload(data, on_delta)? {
                let acc = accumulator.take().expect("accumulator taken once");
                finished = Some(acc.finish(response_preview).map_err(|error| {
                    format!(
                        "解析 provider SSE 流失败: {}; elapsed={}ms; endpoint={}",
                        error,
                        started_at.elapsed().as_millis(),
                        endpoint,
                    )
                }));
                return Ok(true);
            }
            Ok(false)
            },
            guard,
        );

    match finished {
        Some(result) => result,
        None => {
            read_result?;
            let acc = accumulator.expect("accumulator still present at end of stream");
            acc.finish(response_preview).map_err(|error| {
                format!(
                    "解析 provider SSE 流失败: {}; elapsed={}ms; endpoint={}",
                    error,
                    started_at.elapsed().as_millis(),
                    endpoint,
                )
            })
        }
    }
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

        // wave-tui-session-resilience C1：DSML 协议原文在解析层即被折叠，
        // 流式增量与最终累计文本都不再携带原始工具调用协议字面量。
        let delta_text = super::sanitize_dsml_protocol_text(&delta_text);
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
        output_text: super::sanitize_dsml_protocol_text(&combined),
        // 纯文本收集器（仅测试用）：不解析 tool_calls，故整批恒为空。
        tool_calls: Vec::new(),
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
    missing_name_diagnostics: &mut MissingNameDiagnostics,
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
            // B060 降噪：同一 index 每流只输出 1 条诊断行，重复观测仅累计计数
            // （汇总由 `finish()` 至多输出 1 条）。前缀/字段/preview 上限保持不变。
            if missing_name_diagnostics.observe(index) {
                provider_log(format!(
                    "provider:sse-tool-call-missing-name index={} item_preview={}",
                    index,
                    preview_text(&item.to_string(), 400)
                ));
            }
        }
    }
}

fn partial_openai_tool_call_to_tool_call(partial: PartialOpenAiToolCall) -> Option<ToolCall> {
    // 空洞防护（46040ae 意图收窄，与 provider/mod.rs extract_openai_tool_call 同构）：
    // 仅"空名 + 空参数"的上游空洞 tool call（如 PonyLlm 某些网关只发了空 arguments）
    // 按无工具调用处理，交给 normalize_tool_directive 兜底丢弃，避免构造 name=""
    // 的 ToolCall 向下游传播；空名但参数非空时保留 name="" 的 ToolCall，交给
    // normalize_tool_directive 的修复路径（infer_tool_name_from_arguments）收编，
    // 不得在此丢弃（否则 SSE 流里"仅 reasoning + 空名非空参数 tool call、无 content"
    // 会被 finish() 误判为"未提取到文本内容"而整体失败）。
    let name = openai_original_tool_name(partial.name.as_deref().unwrap_or("").trim());
    let arguments = if partial.arguments.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str::<Value>(&partial.arguments)
            .map(|v| if v.is_null() { json!({}) } else { v })
            .unwrap_or_else(|_| json!({}))
    };
    if name.trim().is_empty() && is_hollow_tool_arguments(&arguments) {
        return None;
    }

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

#[cfg(test)]
mod adr0055_tests {
    use super::*;

    /// ADR 0055 P0-2：纯工具调用响应（只有 tool_calls 增量、无 text/reasoning）也必须发出
    /// 一次 `StreamStart`，否则 ttft 永久缺失、速度分母退化成整段耗时。
    #[test]
    fn stream_start_emitted_for_tool_call_only_chunk() {
        let mut acc = OpenAiSseAccumulator::default();
        let payload = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"Read","arguments":"{\"path\":"}}]}}]}"#;
        let mut seen: Vec<ProviderStreamChunk> = Vec::new();
        let mut on_delta = |chunk: ProviderStreamChunk| seen.push(chunk);
        acc.push_payload(payload, &mut on_delta).expect("chunk 必须可解析");
        assert_eq!(
            seen,
            vec![ProviderStreamChunk::StreamStart],
            "首个 SSE 事件必须恰好发出一次 StreamStart，且不得产生任何内容增量"
        );
    }

    /// 首包自带内容时**不**发 `StreamStart`：既有「内容增量顺序」契约零变更
    /// （时点已由首个 Text 增量采集，无需额外信号）。
    #[test]
    fn no_stream_start_when_first_chunk_carries_content() {
        let mut acc = OpenAiSseAccumulator::default();
        let first = r#"{"choices":[{"delta":{"content":"你好"}}]}"#;
        let second = r#"{"choices":[{"delta":{"content":"世界"}}]}"#;
        let mut seen: Vec<ProviderStreamChunk> = Vec::new();
        let mut on_delta = |chunk: ProviderStreamChunk| seen.push(chunk);
        acc.push_payload(first, &mut on_delta).unwrap();
        acc.push_payload(second, &mut on_delta).unwrap();
        assert_eq!(
            seen,
            vec![
                ProviderStreamChunk::Text("你好".to_string()),
                ProviderStreamChunk::Text("世界".to_string()),
            ]
        );
    }

    /// 首个内容增量之后再来的工具调用块不会补发 `StreamStart`（幂等：全流至多一次且只在开头）。
    #[test]
    fn stream_start_is_emitted_at_most_once() {
        let mut acc = OpenAiSseAccumulator::default();
        let tool_only = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"Run","arguments":"{}"}}]}}]}"#;
        let mut seen: Vec<ProviderStreamChunk> = Vec::new();
        let mut on_delta = |chunk: ProviderStreamChunk| seen.push(chunk);
        acc.push_payload(tool_only, &mut on_delta).unwrap();
        acc.push_payload(tool_only, &mut on_delta).unwrap();
        assert_eq!(seen, vec![ProviderStreamChunk::StreamStart]);
    }
}

#[cfg(test)]
#[path = "openai_sse_diag_tests.rs"]
mod openai_sse_diag_tests;
