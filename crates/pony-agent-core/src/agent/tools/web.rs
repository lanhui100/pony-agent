//! `web_fetch_url` / `web_search_query` 面：web 访问策略、pin 到已验证地址的 HTTP 连接、
//! 正文解码与超时重试。
use super::*;
use super::exec::error_result;
use super::exec::json_string;
use super::paths::preview_text;
use super::run::TOOL_TIMEOUT_RETRY_MAX_ATTEMPTS;

pub(super) const DEFAULT_WEB_TIMEOUT_MS: u64 = 15_000;

/// Hard cap on a fetched response body (design Decision 8 / phase-4..7 review P1-4): never read a
/// response unbounded; past this the body is truncated and surfaced honestly.
pub(super) const MAX_WEB_BODY_BYTES: usize = 2 * 1024 * 1024;

/// Fixed user agent for both the legacy web client and the pinned per-hop web client.
pub(super) const WEB_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// Aggregate wall-clock budget for one `web_fetch_url` call — every retry attempt, redirect hop
/// and per-hop request combined (design Decision 8 / tasks 6.2-6.3: a chain-level total deadline).
/// The effective budget is `max(timeoutMs, WEB_FETCH_TOTAL_DEADLINE_MS)` so a single slow hop
/// keeps its full requested `timeoutMs`, while the aggregate never multiplies across hops ×
/// retries × per-hop timeout.
pub(super) const WEB_FETCH_TOTAL_DEADLINE_MS: u64 = 30_000;

impl ToolRouter {
    pub(super) fn web_fetch(&self, call: &ToolCall) -> ToolResult {
        let Some(url) = call.arguments.get("url").and_then(Value::as_str) else {
            return error_result(
                TOOL_WEB_FETCH_URL,
                "missing_argument",
                "缺少必填参数 `url`。".to_string(),
                Some("参数示例：{\"url\":\"https://example.com\",\"timeoutMs\":15000}".to_string()),
            );
        };
        let url = url.trim();

        let timeout_ms = call
            .arguments
            .get("timeoutMs")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, 60_000))
            .unwrap_or(DEFAULT_WEB_TIMEOUT_MS);

        // Fail-closed web access policy + pinned connector (design Decision 8, PA-076 item 2):
        // every URL — the initial one and each of up to `max_redirects` redirect hops — is
        // validated for scheme, credentials, host, resolved addresses, port and redirect budget
        // BEFORE any connection, and every connection is pinned to the decision's validated
        // addresses (the per-hop client's DNS for the hostname is overridden to the validated
        // IPs, so the Host header and TLS SNI preserve the original authority while only the
        // validated peers can ever be reached). The default resolver cannot resolve any host, so
        // arbitrary hostname URLs still fail closed until a production resolver is injected (a
        // governed-path follow-up). On `Deny` the structured reason is returned as a fail-closed
        // error; there is no "validate then let the default client re-resolve" weak mode.
        let policy = WebAccessPolicy::default();
        let connector = PinnedConnector::new(&policy, self.web_resolver.as_ref());
        let sender = ReqwestPinnedSender;

        // Keep the legacy timeout retry (at most two attempts) but preserve the structured error
        // of the final attempt so fail-closed denials keep their machine-readable evidence. One
        // aggregate chain deadline bounds the whole call (retries + redirect hops + per-hop
        // requests, design Decision 8 / tasks 6.2-6.3): at least the per-hop `timeoutMs` so a
        // single slow hop keeps its full requested timeout, but never a multiple of hops ×
        // retries × per-hop timeout.
        let total_budget_ms = timeout_ms.max(WEB_FETCH_TOTAL_DEADLINE_MS);
        let mut structured_error: Option<PinnedFetchError> = None;
        let exchange = match retry_tool_timeout(TOOL_WEB_FETCH_URL, total_budget_ms, |deadline| {
            pinned_web_fetch(
                &connector,
                url,
                timeout_ms,
                total_budget_ms,
                deadline,
                &sender,
            )
            .map_err(|error| {
                let message = pinned_fetch_error_message(&error);
                structured_error = Some(error);
                message
            })
        }) {
            Ok(value) => value,
            Err(message) => {
                return match structured_error.take() {
                    Some(PinnedFetchError::Denied { url, decision }) => {
                        web_fetch_denied(&url, &decision)
                    }
                    Some(PinnedFetchError::Transport(transport)) => {
                        web_fetch_transport_error(&transport)
                    }
                    None => error_result(
                        TOOL_WEB_FETCH_URL,
                        "request_failed",
                        message,
                        Some("请确认目标地址可访问，或稍后重试。".to_string()),
                    ),
                };
            }
        };

        let status = exchange.status;
        let final_url = exchange.url;
        let headers = exchange.headers;
        let (bytes, body_truncated) = (exchange.body, exchange.body_truncated);

        let body = decode_content(&headers, &bytes);

        ToolResult {
            tool_name: TOOL_WEB_FETCH_URL.to_string(),
            status: if status.is_success() {
                "ok".to_string()
            } else {
                "error".to_string()
            },
            output: json_string(json!({
                "ok": status.is_success(),
                "url": final_url,
                "statusCode": status.as_u16(),
                "contentPreview": preview_text(&body, 2000),
                "contentLength": body.len(),
                "truncated": body_truncated,
                "truncationReason": body_truncated.then(|| {
                    format!("响应体超过 {MAX_WEB_BODY_BYTES} 字节上限，已截断。")
                }),
                "error": (!status.is_success()).then(|| json!({
                    "code": "http_error",
                    "kind": "http_error",
                    "message": format!("抓取 URL 返回非成功状态码 {}。", status.as_u16()),
                    "hint": "请确认目标地址可访问，或检查服务端响应状态。"
                })),
                "summary": {
                    "text": format!("已抓取 URL {}，状态码 {}。", url, status.as_u16())
                }
            })),
            duration_ms: 0,
        }
    }

    pub(super) fn web_search(&self, call: &ToolCall) -> ToolResult {
        let Some(query) = call.arguments.get("query").and_then(Value::as_str) else {
            return error_result(
                TOOL_WEB_SEARCH_QUERY,
                "missing_argument",
                "缺少必填参数 `query`。".to_string(),
                Some("参数示例：{\"query\":\"rust reqwest tutorial\",\"objective\":\"优先权威文档并抽取 API 用法\"}".to_string()),
            );
        };
        let query = query.trim();
        if query.is_empty() {
            return error_result(
                TOOL_WEB_SEARCH_QUERY,
                "empty_query",
                "参数 `query` 不能为空字符串。".to_string(),
                Some("请提供外部搜索关键词。".to_string()),
            );
        }

        let Some(objective) = call.arguments.get("objective").and_then(Value::as_str) else {
            return error_result(
                TOOL_WEB_SEARCH_QUERY,
                "missing_argument",
                "缺少必填参数 `objective`。".to_string(),
                Some("参数示例：{\"query\":\"rust reqwest tutorial\",\"objective\":\"优先权威文档，抽取 API 用法与版本事实\"}".to_string()),
            );
        };
        let objective = objective.trim();
        if objective.is_empty() {
            return error_result(
                TOOL_WEB_SEARCH_QUERY,
                "empty_objective",
                "参数 `objective` 不能为空字符串。".to_string(),
                Some("请说明本轮搜索目标：哪些文档应排在前、应排除什么、要抽取哪些事实或数字。".to_string()),
            );
        }
        let timeout_ms = call
            .arguments
            .get("timeoutMs")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, 60_000))
            .unwrap_or(DEFAULT_WEB_TIMEOUT_MS);

        let api_key = match crate::agent::config::ProviderRegistryStore::new().get_service_api_key("exa") {
            Some(key) => key,
            None => match std::env::var("EXA_API_KEY").ok() {
                Some(key) if !key.trim().is_empty() => key,
                _ => {
                    return error_result(
                        TOOL_WEB_SEARCH_QUERY,
                        "missing_api_key",
                        "未设置 EXA_API_KEY。请在设置页面输入 Exa API Key，或设置 EXA_API_KEY 环境变量。".to_string(),
                        Some("访问 https://dashboard.exa.ai 获取 API Key。".to_string()),
                    )
                }
            }
        };

        let client = match build_web_client(timeout_ms) {
            Ok(value) => value,
            Err(error) => {
                return error_result(TOOL_WEB_SEARCH_QUERY, "client_build_failed", error, None)
            }
        };

        // 推荐请求 = query + objective + contents.highlights；不含 type/numResults 等未被任务显式要求的字段。
        let body = exa_search_request_body(query, objective);

        let response = match retry_tool_timeout(
            TOOL_WEB_SEARCH_QUERY,
            timeout_ms.max(WEB_FETCH_TOTAL_DEADLINE_MS),
            |_deadline| {
                let request = client
                    .post("https://api.exa.ai/search")
                    .header("x-api-key", &api_key)
                    .header("Content-Type", "application/json");
                // Wave-4 C3：工具出网在活跃 turn 内级联 traceparent（无活跃 turn 时零改动）。
                let request = crate::agent::otel::with_current_traceparent(request);
                // `body` 由 `FnMut` 闭包按引用捕获，先克隆出 owned 字符串再 move 进 async 块。
                let body_text = body.to_string();
                block_on(async move { request.body(body_text).send().await })
                .map_err(|error| {
                    if is_reqwest_timeout_error(&error) {
                        format!("timeout: Exa 搜索请求超时：{}。", error)
                    } else {
                        format!("Exa 搜索请求失败：{}。", error)
                    }
                })
            },
        ) {
            Ok(value) => value,
            Err(error) => {
                let code = if is_tool_timeout_message(&error) {
                    "timeout"
                } else {
                    "request_failed"
                };
                return error_result(
                    TOOL_WEB_SEARCH_QUERY,
                    code,
                    error,
                    Some("请确认 API Key 有效且网络可访问 api.exa.ai。".to_string()),
                );
            }
        };

        let status = response.status();
        let response_body = match block_on(async move { response.text().await }) {
            Ok(value) => value,
            Err(error) => {
                return error_result(
                    TOOL_WEB_SEARCH_QUERY,
                    "read_body_failed",
                    format!("读取 Exa 响应失败：{}。", error),
                    None,
                )
            }
        };

        let parsed: Value = match serde_json::from_str(&response_body) {
            Ok(value) => value,
            Err(_) => {
                return error_result(
                    TOOL_WEB_SEARCH_QUERY,
                    "api_error",
                    format!(
                        "Exa API 返回异常（状态码 {}）：{}",
                        status.as_u16(),
                        preview_text(&response_body, 200)
                    ),
                    Some("请检查 API Key 和查询参数。".to_string()),
                )
            }
        };

        if !status.is_success() {
            let error_msg = parsed
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("未知错误");
            return error_result(
                TOOL_WEB_SEARCH_QUERY,
                "api_error",
                format!("Exa API 错误（状态码 {}）：{}", status.as_u16(), error_msg),
                Some("请检查 API Key 和查询参数。".to_string()),
            );
        }

        let results: Vec<Value> = parsed
            .get("results")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .map(|r| {
                        json!({
                            "title": r.get("title").and_then(Value::as_str).unwrap_or(""),
                            "url": r.get("url").and_then(Value::as_str).unwrap_or(""),
                            "snippet": r.get("highlights")
                                .and_then(Value::as_array)
                                .and_then(|h| h.first())
                                .and_then(Value::as_str)
                                .unwrap_or(""),
                            "score": r.get("score").and_then(Value::as_f64).unwrap_or(0.0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        ToolResult {
            tool_name: TOOL_WEB_SEARCH_QUERY.to_string(),
            status: "ok".to_string(),
            output: json_string(json!({
                "ok": true,
                "query": query,
                "objective": objective,
                "statusCode": status.as_u16(),
                "resultCount": results.len(),
                "results": results,
                "summary": {
                    "text": format!("已完成 Exa 搜索 `{}`，返回 {} 条结果。", query, results.len())
                }
            })),
            duration_ms: 0,
        }
    }
}


/// Exa `/search` 推荐请求体：只含 `query` + `objective` + `contents.highlights`。
/// 刻意省略 `type` / `numResults` / 域过滤等门控字段——除任务显式要求外不加装饰字段
/// （见 build-with-exa 技能：`objective` 是搜索被 LLM 当工具调用时的唯一默认新增字段）。
pub(super) fn exa_search_request_body(query: &str, objective: &str) -> Value {
    json!({
        "query": query,
        "objective": objective,
        "contents": {
            "highlights": true
        }
    })
}

pub(super) fn build_web_client(timeout_ms: u64) -> Result<Client, String> {
    // Design Decision 8: no ambient proxy and no automatic redirects. The web access policy
    // validates every URL (and would validate every redirect hop) before any connection; the
    // client never follows redirects on its own and never reads proxy environment variables.
    // Used by `web_search` (fixed, trusted Exa endpoint); `web_fetch` uses the pinned per-hop
    // client built by [`build_pinned_web_client`] instead.
    Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .user_agent(WEB_USER_AGENT)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("创建 HTTP 客户端失败：{}。", error))
}

/// One validated HTTP hop in the pinned fetch pipeline: the requested URL, the response status
/// and headers, the bounded body (empty for redirect hops, whose bodies are never read), whether
/// the body was truncated at the cap, and the actual peer address of the connection (verified to
/// be one of the decision's resolved addresses).
#[derive(Clone, Debug)]
pub(super) struct PinnedHttpExchange {
    pub(super) url: String,
    pub(super) status: reqwest::StatusCode,
    pub(super) headers: reqwest::header::HeaderMap,
    pub(super) body: Vec<u8>,
    pub(super) body_truncated: bool,
    /// The actual peer IP of the successful connection; validated against resolved_addrs.
    #[allow(dead_code)]
    pub(super) peer_ip: Option<IpAddr>,
}

impl PinnedHttpExchange {
    /// The `Location` header when present and non-empty after trimming.
    pub(super) fn location(&self) -> Option<String> {
        self.headers
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    /// A hop is a redirect only when it carries a 3xx status AND a usable `Location` header; a
    /// 3xx without `Location` is treated as a terminal response and is never auto-followed.
    fn is_redirect(&self) -> bool {
        self.status.is_redirection() && self.location().is_some()
    }
}

/// Transport/pinning failures from [`PinnedSender`]. Distinct from policy denials
/// ([`PinnedFetchError::Denied`]).
#[derive(Clone, Debug)]
pub(super) enum WebFetchTransportError {
    ClientBuild(String),
    Request(String),
    UnsupportedContentType {
        content_type: String,
    },
    ReadBody(String),
    PeerMismatch {
        expected: Vec<IpAddr>,
        actual: IpAddr,
    },
    /// The whole-fetch chain deadline was exhausted before the fetch completed (design Decision 8
    /// / tasks 6.2-6.3). The fetch is cut structurally rather than letting per-hop timeouts
    /// accumulate across redirect hops and retries.
    DeadlineExceeded {
        /// The chain budget (ms) assigned to the whole fetch.
        budget_ms: u64,
    },
}

/// Failure of the whole pinned web fetch pipeline: a policy denial (initial URL or a redirect
/// hop) or a transport/pinning failure.
#[derive(Clone, Debug)]
pub(super) enum PinnedFetchError {
    Denied {
        url: String,
        decision: WebAccessDecision,
    },
    Transport(WebFetchTransportError),
}

/// Build a per-hop reqwest client whose connections for `hostname` are pinned to the decision's
/// validated addresses (design Decision 8 / PA-076 item 2).
///
/// The requested URL keeps the original authority, so the `Host` header and the TLS SNI both
/// preserve the validated hostname; only the socket connect target is overridden via reqwest's
/// `resolve_to_addrs` (port `0` means "use the URL's port"). For literal-IP hosts there is no DNS
/// step at all, so the connection is pinned by construction. For hostname URLs the override is
/// mandatory: an empty address set refuses to build the client rather than ever falling back to
/// ambient DNS. This override makes it impossible for the client to connect anywhere except the
/// validated addresses; the per-response peer check in [`ReqwestPinnedSender`] is defense in
/// depth on top of it.
pub(super) fn build_pinned_web_client(
    decision: &WebAccessDecision,
    hostname: &str,
    timeout_ms: u64,
) -> Result<Client, String> {
    let WebAccessDecision::Allow { resolved_addrs, .. } = decision else {
        return Err("不能为已拒绝的 Web 访问决策构建连接。".to_string());
    };
    let mut builder = Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .user_agent(WEB_USER_AGENT)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none());
    if hostname.parse::<IpAddr>().is_err() {
        if resolved_addrs.is_empty() {
            return Err("pinned connector 收到空地址集，拒绝回退到环境 DNS。".to_string());
        }
        let pinned: Vec<SocketAddr> = resolved_addrs
            .iter()
            .map(|address| SocketAddr::new(*address, 0))
            .collect();
        builder = builder.resolve_to_addrs(hostname, &pinned);
    }
    builder
        .build()
        .map_err(|error| format!("创建 HTTP 客户端失败：{}。", error))
}

/// Verify that the actual peer of a completed connection belongs to the decision's validated
/// address set. The reqwest DNS override in [`build_pinned_web_client`] already makes it
/// impossible to connect outside the validated set; this check is defense in depth so that even a
/// future connector change that removes the override cannot silently accept a different peer.
/// When the peer is not reported (a transport that exposes no address), the override guarantee
/// still holds and no extra check is possible.
pub(super) fn verify_pinned_peer(
    peer: Option<IpAddr>,
    decision: &WebAccessDecision,
) -> Result<(), WebFetchTransportError> {
    if let Some(peer) = peer {
        if !decision.resolved_addrs().contains(&peer) {
            return Err(WebFetchTransportError::PeerMismatch {
                expected: decision.resolved_addrs().to_vec(),
                actual: peer,
            });
        }
    }
    Ok(())
}

/// Production [`PinnedSender`]: per-hop reqwest client pinned to the decision's validated
/// addresses, with a post-connection peer-IP check (`response.remote_addr()`).
pub(super) struct ReqwestPinnedSender;

/// Drive a pinned web fetch: validate the initial URL, perform each hop through the injected
/// sender (production: a per-hop client pinned to the decision's validated addresses), and
/// re-validate + re-pin every redirect hop against the same policy. At most
/// `policy.max_redirects` redirects may be followed; any hop that fails validation fails the
/// whole fetch closed. Never falls back to ambient DNS or to a re-resolving client.
///
/// An aggregate chain deadline bounds the whole fetch (design Decision 8 / tasks 6.2-6.3):
/// `deadline` is the single wall-clock instant the whole call — all hops and, via the caller's
/// retry loop, all retries — must finish by. Each hop is granted at most
/// `min(timeout_ms, remaining chain budget)`; once the budget is exhausted the fetch fails
/// structurally with [`WebFetchTransportError::DeadlineExceeded`] instead of letting per-hop
/// timeouts accumulate.
pub(super) fn pinned_web_fetch(
    connector: &PinnedConnector<'_>,
    url: &str,
    timeout_ms: u64,
    total_budget_ms: u64,
    deadline: Instant,
    sender: &dyn PinnedSender,
) -> Result<PinnedHttpExchange, PinnedFetchError> {
    let mut current_url = url.to_string();
    let mut decision = connector.validate_initial(&current_url);
    if !decision.is_allowed() {
        return Err(PinnedFetchError::Denied {
            url: current_url,
            decision,
        });
    }

    let mut hops_followed: usize = 0;
    loop {
        let remaining_ms = deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u64;
        if remaining_ms == 0 {
            return Err(PinnedFetchError::Transport(
                WebFetchTransportError::DeadlineExceeded {
                    budget_ms: total_budget_ms,
                },
            ));
        }
        // Clamp the per-hop request timeout to the remaining chain budget so a slow hop cannot
        // consume more than what the aggregate deadline leaves.
        let hop_timeout_ms = timeout_ms.min(remaining_ms);
        let exchange = sender
            .send(&current_url, &decision, hop_timeout_ms)
            .map_err(PinnedFetchError::Transport)?;

        if !exchange.is_redirect() {
            return Ok(exchange);
        }
        // `is_redirect()` guarantees a non-empty Location header.
        let location = exchange
            .location()
            .expect("redirect exchange carries a Location header");
        let target = connector
            .resolve_redirect_target(&exchange.url, &location)
            .map_err(|detail| PinnedFetchError::Denied {
                url: location,
                decision: WebAccessDecision::Deny {
                    reason: WebAccessDenyReason::InvalidUrl(detail),
                    hop: hops_followed + 1,
                },
            })?;
        let next = connector.validate_redirect(&target, hops_followed);
        if !next.is_allowed() {
            return Err(PinnedFetchError::Denied {
                url: target,
                decision: next,
            });
        }
        current_url = target;
        decision = next;
        hops_followed += 1;
    }
}


/// Sends one validated, pinned HTTP request hop. The production implementation
/// ([`ReqwestPinnedSender`]) pins the connection to the decision's resolved addresses via a
/// per-hop reqwest client whose DNS for the URL's hostname is overridden to those addresses; a
/// fake is used in hermetic tests to exercise the redirect re-validation logic without a socket.
pub(super) trait PinnedSender: Send + Sync {
    fn send(
        &self,
        url: &str,
        decision: &WebAccessDecision,
        timeout_ms: u64,
    ) -> Result<PinnedHttpExchange, WebFetchTransportError>;
}
impl PinnedSender for ReqwestPinnedSender {
    fn send(
        &self,
        url: &str,
        decision: &WebAccessDecision,
        timeout_ms: u64,
    ) -> Result<PinnedHttpExchange, WebFetchTransportError> {
        let parsed = Url::parse(url)
            .map_err(|error| WebFetchTransportError::Request(format!("URL 无法解析：{error}")))?;
        let hostname = parsed.host_str().unwrap_or("").to_string();
        let client = build_pinned_web_client(decision, &hostname, timeout_ms)
            .map_err(WebFetchTransportError::ClientBuild)?;
        // Wave-4 C3：工具出网在活跃 turn 内级联 traceparent（无活跃 turn 时零改动）。
        let request = crate::agent::otel::with_current_traceparent(client.get(parsed));
        let response = block_on(async move { request.send().await }).map_err(|error| {
            if is_reqwest_timeout_error(&error) {
                WebFetchTransportError::Request(format!("timeout: 抓取 URL 超时：{}。", error))
            } else {
                WebFetchTransportError::Request(format!("抓取 URL 失败：{}。", error))
            }
        })?;

        // Peer-IP check: the DNS override already restricts the connection to the validated
        // addresses; confirm the actually-established peer belongs to them as defense in depth.
        let peer_ip = response.remote_addr().map(|address| address.ip());
        verify_pinned_peer(peer_ip, decision)?;

        let status = response.status();
        let headers = response.headers().clone();
        let requested_url = response.url().to_string();

        // Redirect hop: the body is never read; the caller re-validates and re-pins the next hop
        // before any further connection (design Decision 8: bounded redirects, each hop validated).
        if status.is_redirection() && headers.get(reqwest::header::LOCATION).is_some() {
            return Ok(PinnedHttpExchange {
                url: requested_url,
                status,
                headers,
                body: Vec::new(),
                body_truncated: false,
                peer_ip,
            });
        }

        // Terminal hop: gate the declared content type before reading the body, then stream with a
        // hard byte cap and surface truncation honestly (design Decision 8 / phase-4..7 review
        // P1-4; unchanged from the legacy path).
        if let Some(content_type) = headers.get(reqwest::header::CONTENT_TYPE) {
            if !is_text_content_type(content_type.to_str().unwrap_or("")) {
                return Err(WebFetchTransportError::UnsupportedContentType {
                    content_type: content_type.to_str().unwrap_or("unknown").to_string(),
                });
            }
        }
        let (body, body_truncated) = block_on(read_bounded_body(response, MAX_WEB_BODY_BYTES))
            .map_err(WebFetchTransportError::ReadBody)?;
        Ok(PinnedHttpExchange {
            url: requested_url,
            status,
            headers,
            body,
            body_truncated,
            peer_ip,
        })
    }
}

/// Human-readable message for a whole-pipeline [`PinnedFetchError`].
pub(super) fn pinned_fetch_error_message(error: &PinnedFetchError) -> String {
    match error {
        PinnedFetchError::Denied {
            decision: WebAccessDecision::Deny { reason, .. },
            ..
        } => web_access_deny_message(reason),
        PinnedFetchError::Denied { .. } => "URL 被 Web 访问策略拒绝。".to_string(),
        PinnedFetchError::Transport(transport) => web_fetch_transport_message(transport),
    }
}

/// Human-readable message for a [`WebFetchTransportError`].
pub(super) fn web_fetch_transport_message(error: &WebFetchTransportError) -> String {
    match error {
        WebFetchTransportError::ClientBuild(message) => message.clone(),
        WebFetchTransportError::Request(message) => message.clone(),
        WebFetchTransportError::UnsupportedContentType { content_type } => {
            format!("目标返回的内容类型 `{content_type}` 不是可读文本，已拒绝解码。")
        }
        WebFetchTransportError::ReadBody(message) => format!("读取响应正文失败：{message}。"),
        WebFetchTransportError::PeerMismatch { expected, actual } => {
            format!("连接对端地址 {actual} 不在已校验地址列表 {expected:?} 中，已拒绝连接。")
        }
        WebFetchTransportError::DeadlineExceeded { budget_ms } => {
            format!("抓取 URL 超过链级总 deadline（总预算 {budget_ms} ms）。")
        }
    }
}

pub(super) fn is_reqwest_timeout_error(error: &reqwest::Error) -> bool {
    error.is_timeout()
        || error
            .source()
            .map(|source| {
                source
                    .to_string()
                    .to_ascii_lowercase()
                    .contains("timed out")
            })
            .unwrap_or(false)
}

pub(super) fn is_tool_timeout_message(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("timeout")
        || normalized.contains("timed out")
        || normalized.contains("deadline has elapsed")
}

/// Retry a tool operation up to [`TOOL_TIMEOUT_RETRY_MAX_ATTEMPTS`] times when it fails with a
/// timeout-class error. The whole call — retries plus the in-flight work of each attempt — is
/// bounded by one aggregate wall-clock deadline derived from `total_budget_ms` (design Decision 8
/// / tasks 6.2-6.3): the operation receives that absolute deadline so a chained operation such as
/// `pinned_web_fetch` clamps its own work to the same budget, and a retry never restarts the
/// budget (each attempt does not get its own fresh `total_budget_ms`).
pub(super) fn retry_tool_timeout<T, F>(
    tool_name: &str,
    total_budget_ms: u64,
    mut operation: F,
) -> Result<T, String>
where
    F: FnMut(Instant) -> Result<T, String>,
{
    let config = crate::agent::retry::BackoffConfig {
        max_retries: TOOL_TIMEOUT_RETRY_MAX_ATTEMPTS.saturating_sub(1),
        initial_delay_ms: 500,
        multiplier: 2.0,
        max_delay_ms: 8000,
        total_budget_ms,
        jitter_kind: crate::agent::retry::JitterKind::None,
    };
    let deadline = Instant::now() + Duration::from_millis(total_budget_ms);
    let mut last_error = String::new();
    for attempt in 0..=config.max_retries {
        if attempt > 0 {
            // Backoff, capped by what the shared deadline leaves; when nothing is left, stop.
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(last_error);
            }
            let delay = crate::agent::retry::compute_delay(attempt, &config, 0.5).min(remaining);
            thread::sleep(delay);
            if Instant::now() >= deadline {
                return Err(last_error);
            }
        }
        match operation(deadline) {
            Ok(value) => return Ok(value),
            Err(error) => {
                last_error = error;
                if !is_tool_timeout_message(&last_error) {
                    return Err(last_error);
                }
                if attempt == config.max_retries || Instant::now() >= deadline {
                    return Err(last_error);
                }
                let _ = (tool_name, preview_text(&last_error, 180));
            }
        }
    }
    Err(last_error)
}

/// Stream a response body up to `cap` bytes; returns `(bytes, truncated)` where `truncated` is true
/// when the body exceeded the cap and was cut short (design Decision 8, phase-4..7 review P1-4).
async fn read_bounded_body(
    mut response: reqwest::Response,
    cap: usize,
) -> Result<(Vec<u8>, bool), String> {
    let mut buf = Vec::with_capacity(cap.min(4096));
    let mut truncated = false;
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if buf.len().saturating_add(chunk.len()) > cap {
            let remaining = cap.saturating_sub(buf.len());
            buf.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        buf.extend_from_slice(&chunk);
    }
    Ok((buf, truncated))
}

/// Whether a declared `Content-Type` is text we may decode. Anything clearly binary (images,
/// audio, video, archives, PDF, octet-stream) is refused before decoding (Decision 8).
pub(super) fn is_text_content_type(content_type: &str) -> bool {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    mime.starts_with("text/")
        || matches!(
            mime.as_str(),
            "application/json"
                | "application/javascript"
                | "application/xml"
                | "application/xhtml+xml"
                | "application/x-www-form-urlencoded"
                | "application/graphql"
                | "application/ld+json"
                | "application/sql"
                | "application/yaml"
                | "application/x-yaml"
                | "application/toml"
                | "application/x-httpd-php"
                | "application/ecmascript"
        )
        || content_type.is_empty()
}

pub(super) fn decode_content(headers: &reqwest::header::HeaderMap, bytes: &[u8]) -> String {
    let encoding = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|ct| {
            ct.split(';')
                .nth(1)
                .and_then(|s| s.split('=').nth(1))
                .map(|s| s.trim().trim_matches('"').to_lowercase())
                .filter(|s| !s.is_empty())
        })
        .and_then(|charset| Encoding::for_label(charset.as_bytes()));

    if let Some(enc) = encoding {
        let (cow, _, _) = enc.decode(bytes);
        return cow.into_owned();
    }

    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }

    let (cow, _, _) = GBK.decode(bytes);
    cow.into_owned()
}

/// Default `WebResolver` for the legacy `ToolRouter`: cannot resolve any host, so every hostname
/// URL fails closed (design Decision 8). A real resolver is only injected together with a pinned
/// connector in the governed path.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct FailClosedResolver;

/// Fail-closed `web_fetch_url` result for a URL rejected by the web access policy. Carries the
/// structured deny reason (design Decision 8) so callers and telemetry can surface
/// machine-readable evidence of why the fetch was refused.
pub(super) fn web_fetch_denied(url: &str, decision: &WebAccessDecision) -> ToolResult {
    let message = match decision {
        WebAccessDecision::Allow { .. } => String::new(), // unreachable: only called on Deny
        WebAccessDecision::Deny { reason, .. } => web_access_deny_message(reason),
    };
    ToolResult {
        tool_name: TOOL_WEB_FETCH_URL.to_string(),
        status: "error".to_string(),
        output: json_string(json!({
            "ok": false,
            "tool": TOOL_WEB_FETCH_URL,
            "url": url,
            "error": {
                "code": "web_access_denied",
                "kind": "web_access_denied",
                "message": message,
                "hint": "请检查 URL 是否为公开的 http/https 地址，且未指向内网、回环或受限端口。",
                "accessDecision": serde_json::to_value(decision).unwrap_or(Value::Null),
            },
            "summary": {
                "text": message
            }
        })),
        duration_ms: 0,
    }
}

impl WebResolver for FailClosedResolver {
    fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, String> {
        Err(format!(
            "no production DNS resolver is wired into the legacy ToolRouter; cannot resolve `{host}`"
        ))
    }
}

/// Fail-closed `web_fetch_url` result for a transport/pinning failure in the pinned fetch
/// pipeline (connect error, timeout, non-text content type, body read failure, or a peer that
/// does not belong to the validated address set).
pub(super) fn web_fetch_transport_error(error: &WebFetchTransportError) -> ToolResult {
    let (code, hint): (&str, Option<String>) = match error {
        WebFetchTransportError::ClientBuild(_) => ("client_build_failed", None),
        WebFetchTransportError::Request(message) => {
            if is_tool_timeout_message(message) {
                (
                    "timeout",
                    Some("请确认目标地址可访问，或稍后重试。".to_string()),
                )
            } else {
                (
                    "request_failed",
                    Some("请确认目标地址可访问，或稍后重试。".to_string()),
                )
            }
        }
        WebFetchTransportError::UnsupportedContentType { .. } => (
            "unsupported_content_type",
            Some(
                "请改用能返回 text/html、text/plain、application/json 等文本内容的地址。"
                    .to_string(),
            ),
        ),
        WebFetchTransportError::ReadBody(_) => (
            "read_body_failed",
            Some("请确认目标地址返回的是可读取文本内容。".to_string()),
        ),
        WebFetchTransportError::PeerMismatch { .. } => (
            "connection_pin_violation",
            Some("连接对端地址与已校验地址不一致，已拒绝本次连接。".to_string()),
        ),
        WebFetchTransportError::DeadlineExceeded { .. } => (
            "web_fetch_deadline_exceeded",
            Some(
                "目标服务器响应链过慢，已超过本次抓取的总时间预算。请稍后重试或缩小抓取范围。"
                    .to_string(),
            ),
        ),
    };
    error_result(
        TOOL_WEB_FETCH_URL,
        code,
        web_fetch_transport_message(error),
        hint,
    )
}

/// Human-readable summary for a structured web access deny reason.
pub(super) fn web_access_deny_message(reason: &WebAccessDenyReason) -> String {
    match reason {
        WebAccessDenyReason::InvalidUrl(detail) => format!("URL 无法解析：{detail}。"),
        WebAccessDenyReason::UnsupportedScheme { scheme } => {
            format!("不支持的 URL 协议：`{scheme}`，仅允许 http/https。")
        }
        WebAccessDenyReason::MissingHost => "URL 缺少主机名。".to_string(),
        WebAccessDenyReason::CredentialsNotAllowed => "URL 不允许携带用户名/密码凭据。".to_string(),
        WebAccessDenyReason::HostForbidden { host, detail } => {
            format!("主机 `{host}` 被策略禁止：{detail}。")
        }
        WebAccessDenyReason::RestrictedPort { port } => {
            format!("端口 {port} 在受限端口列表中。")
        }
        WebAccessDenyReason::ForbiddenLiteralAddress { address, detail } => {
            format!("字面 IP `{address}` 属于禁止地址类：{detail}。")
        }
        WebAccessDenyReason::ResolvesToForbiddenAddress {
            host,
            address,
            detail,
        } => format!("主机 `{host}` 解析到禁止地址 `{address}`：{detail}。"),
        WebAccessDenyReason::NoAddresses { host } => {
            format!("主机 `{host}` 没有可用 DNS 记录。")
        }
        WebAccessDenyReason::ResolutionFailed { host, error } => {
            format!("无法解析主机 `{host}`：{error}。")
        }
        WebAccessDenyReason::TooManyRedirects { limit } => {
            format!("重定向次数超过上限 {limit}。")
        }
    }
}
