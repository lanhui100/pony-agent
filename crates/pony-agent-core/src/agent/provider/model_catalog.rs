// model_catalog: 模型目录拉取（design D3 / spec provider-registry Requirement 3）。
//
// 仅出网读取面：GET {base_url}/models、15s 超时、error-for-status、
// 非 JSON 正文显式报错（杜绝"失败被呈现成空目录"）、key 不入任何日志。
use super::*;

/// 目录条目上限：去重排序后截断并记 provider_log（不新增前端类型）。
const MODEL_CATALOG_LIMIT: usize = 1000;

pub fn fetch_model_ids(
    protocol: &ProviderProtocol,
    base_url: &str,
    auth_hint: &ProviderAuthType,
    api_key: &str,
) -> Result<Vec<String>, String> {
    let endpoint = format!("{}/models", base_url.trim_end_matches('/'));
    let client = build_http_client(Duration::from_secs(15))?;
    let started_at = Instant::now();

    let mut request = client.get(&endpoint);
    for (name, value) in catalog_auth_headers(protocol, auth_hint, api_key) {
        request = request.header(name, value.as_str());
    }

    let response = block_on(request.send())
        .map_err(|error| format_request_error("获取模型列表失败", &error, started_at.elapsed()))?;

    let status = response.status();
    let body = block_on(response.text()).map_err(|error| {
        format_request_error("读取模型列表返回失败", &error, started_at.elapsed())
    })?;

    if !status.is_success() {
        return Err(format!(
            "获取模型列表失败：状态码 {}；耗时={}ms；响应正文：{}",
            status,
            started_at.elapsed().as_millis(),
            preview_text(&body, 400)
        ));
    }

    parse_model_catalog_body(&body)
}

/// 鉴权头按家族推导，显式 auth hint 覆盖 Auto：
/// openai 族 → Authorization: Bearer；anthropic → x-api-key + anthropic-version。
fn catalog_auth_headers(
    protocol: &ProviderProtocol,
    auth_hint: &ProviderAuthType,
    api_key: &str,
) -> Vec<(&'static str, String)> {
    let effective = match auth_hint {
        ProviderAuthType::Auto => {
            if protocol.is_anthropic() {
                ProviderAuthType::XApiKey
            } else {
                ProviderAuthType::Bearer
            }
        }
        explicit => explicit.clone(),
    };

    match effective {
        ProviderAuthType::Bearer => vec![("Authorization", format!("Bearer {api_key}"))],
        ProviderAuthType::XApiKey | ProviderAuthType::Auto => vec![
            ("x-api-key", api_key.to_string()),
            ("anthropic-version", "2023-06-01".to_string()),
        ],
    }
}

/// 容错解析：`data[].id`（OpenAI/Anthropic/OpenRouter 通例）、根数组 `[{"id"}]`、
/// `models[].id`；去重排序，上限 1000 条截断并记日志。
fn parse_model_catalog_body(body: &str) -> Result<Vec<String>, String> {
    let value: Value = serde_json::from_str(body.trim()).map_err(|_| {
        // 内容嗅探：HTML 错误页 / 纯文本一律显式报错。
        format!(
            "模型目录返回不是有效 JSON；正文预览：{}",
            preview_text(body, 200)
        )
    })?;

    let mut ids = collect_model_ids(&value);
    if ids.is_empty() {
        return Err(format!(
            "模型目录中未找到可识别的模型 ID（支持 data[].id / models[].id / 根数组）；正文预览：{}",
            preview_text(body, 200)
        ));
    }

    ids.sort();
    ids.dedup();
    if ids.len() > MODEL_CATALOG_LIMIT {
        provider_log(format!(
            "model-catalog truncated: {} -> {} entries",
            ids.len(),
            MODEL_CATALOG_LIMIT
        ));
        ids.truncate(MODEL_CATALOG_LIMIT);
    }
    Ok(ids)
}

fn collect_model_ids(value: &Value) -> Vec<String> {
    if let Some(entries) = value.as_array() {
        return model_ids_from_entries(entries);
    }

    if let Some(entries) = value.get("data").and_then(Value::as_array) {
        return model_ids_from_entries(entries);
    }

    if let Some(entries) = value.get("models").and_then(Value::as_array) {
        return model_ids_from_entries(entries);
    }

    Vec::new()
}

fn model_ids_from_entries(entries: &[Value]) -> Vec<String> {
    entries
        .iter()
        .filter_map(|entry| entry.get("id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_data_array_ids() {
        let body = json!({
            "data": [
                { "id": "gpt-4.1-mini" },
                { "id": "gpt-5.4" },
                { "object": "model" }
            ]
        });
        let ids = parse_model_catalog_body(&body.to_string()).expect("parse data[]");
        assert_eq!(ids, vec!["gpt-4.1-mini".to_string(), "gpt-5.4".to_string()]);
    }

    #[test]
    fn parses_root_array_and_models_array_shapes() {
        let root_array = json!([{ "id": "claude-3-7" }, { "id": "claude-opus-4" }]);
        let ids = parse_model_catalog_body(&root_array.to_string()).expect("parse root array");
        assert_eq!(ids.len(), 2);

        let models_shape = json!({ "models": [{ "id": "deepseek-v4-pro" }] });
        let ids = parse_model_catalog_body(&models_shape.to_string()).expect("parse models[]");
        assert_eq!(ids, vec!["deepseek-v4-pro".to_string()]);
    }

    #[test]
    fn non_json_body_is_an_explicit_error_not_an_empty_list() {
        let html = "<html><body>502 Bad Gateway</body></html>";
        let error = parse_model_catalog_body(html).expect_err("non-JSON must fail");
        assert!(error.contains("不是有效 JSON"), "actual: {error}");

        let json_without_ids = json!({ "object": "list" });
        let error = parse_model_catalog_body(&json_without_ids.to_string())
            .expect_err("JSON without id entries must fail");
        assert!(error.contains("未找到可识别的模型 ID"));
    }

    #[test]
    fn dedupes_sorts_and_caps_entries_with_truncation() {
        let entries: Vec<Value> = ["b", "a", "a", "c"]
            .into_iter()
            .map(|id| json!({ "id": id }))
            .collect();
        let ids = parse_model_catalog_body(&json!({ "data": entries }).to_string())
            .expect("dedupe parse");
        assert_eq!(ids, vec!["a".to_string(), "b".to_string(), "c".to_string()]);

        let many: Vec<Value> = (0..1500)
            .map(|index| json!({ "id": format!("m-{index:05}") }))
            .collect();
        let ids =
            parse_model_catalog_body(&json!({ "data": many }).to_string()).expect("cap parse");
        assert_eq!(ids.len(), MODEL_CATALOG_LIMIT);
        assert_eq!(ids.last().map(String::as_str), Some("m-00999"));
    }

    #[test]
    fn auth_headers_follow_family_or_explicit_hint() {
        // openai 族 Auto → Bearer。
        let headers = catalog_auth_headers(
            &ProviderProtocol::OpenAiCompletions,
            &ProviderAuthType::Auto,
            "k",
        );
        assert_eq!(headers, vec![("Authorization", "Bearer k".to_string())]);
        // responses 同待遇。
        let headers = catalog_auth_headers(
            &ProviderProtocol::OpenAiResponses,
            &ProviderAuthType::Auto,
            "k",
        );
        assert_eq!(headers, vec![("Authorization", "Bearer k".to_string())]);
        // anthropic Auto → x-api-key + anthropic-version。
        let headers = catalog_auth_headers(
            &ProviderProtocol::AnthropicMessages,
            &ProviderAuthType::Auto,
            "k",
        );
        assert_eq!(
            headers,
            vec![
                ("x-api-key", "k".to_string()),
                ("anthropic-version", "2023-06-01".to_string()),
            ]
        );
        // anthropic 兼容端点要求 Bearer 的显式覆盖被尊重。
        let headers = catalog_auth_headers(
            &ProviderProtocol::AnthropicMessages,
            &ProviderAuthType::Bearer,
            "k",
        );
        assert_eq!(headers, vec![("Authorization", "Bearer k".to_string())]);
    }
}
