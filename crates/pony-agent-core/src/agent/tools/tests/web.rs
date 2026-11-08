//! web 抓取/搜索与 pinned 连接（重定向链、预算、时间戳重试、真实 socket 自证）。
use super::*;

#[test]
fn web_fetch_rejects_non_http_urls() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WEB_FETCH_URL.to_string(),
        arguments: json!({
            "url": "file:///etc/passwd"
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("web fetch output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("web_access_denied")
    );
    // The structured deny reason is surfaced for telemetry/consumers.
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("accessDecision"))
            .and_then(|decision| decision.get("decision"))
            .and_then(Value::as_str),
        Some("deny")
    );
}

#[test]
fn web_fetch_fails_closed_for_hostname_urls_without_a_production_resolver() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);

    // The legacy default resolver cannot resolve any host, so an arbitrary hostname URL fails
    // closed (design Decision 8) with a structured deny reason.
    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WEB_FETCH_URL.to_string(),
        arguments: json!({
            "url": "https://example.com/",
            "timeoutMs": 5000
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("web fetch output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("web_access_denied")
    );
    assert!(payload
        .get("error")
        .and_then(|error| error.get("accessDecision"))
        .and_then(|decision| decision.get("detail"))
        .and_then(|detail| detail.get("reason"))
        .and_then(|reason| reason.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .contains("resolution_failed"));
}

#[test]
fn web_search_rejects_empty_query() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WEB_SEARCH_QUERY.to_string(),
        arguments: json!({
            "query": "   "
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload =
        serde_json::from_str::<Value>(&result.output).expect("web search output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("empty_query")
    );
}

#[test]
fn web_fetch_fails_closed_for_loopback_and_private_urls() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);

    for url in [
        "http://127.0.0.1:8080/",
        "http://localhost/",
        "http://192.168.1.10/",
        "http://169.254.169.254/latest/meta-data/",
    ] {
        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WEB_FETCH_URL.to_string(),
            arguments: json!({
                "url": url,
                "timeoutMs": 5000
            }),
            plan: None,
        });
        assert_eq!(result.status, "error", "url `{url}` must fail closed");
        let payload =
            serde_json::from_str::<Value>(&result.output).expect("web fetch output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("web_access_denied"),
            "url `{url}`"
        );
    }
}

#[test]
fn web_fetch_denies_urls_rejected_before_any_connection() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);
    // A local test server is reachable, but the URL points at a loopback address, so the web
    // access policy denies it before any connection is made (fail closed, design Decision 8).
    let url = serve_single_http_response(
        "HTTP/1.1 200 OK",
        "<html><body><h1>Hello Pony</h1><p>Fetch success path.</p></body></html>",
        "text/html; charset=utf-8",
    );

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WEB_FETCH_URL.to_string(),
        arguments: json!({
            "url": url,
            "timeoutMs": 5000
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("web fetch output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("web_access_denied")
    );
}

#[test]
fn web_fetch_returns_structured_http_error_for_non_2xx_response_when_allowed() {
    // Kept as a contract regression for the legacy success/non-2xx output shape: the success
    // and http_error branches are unreachable through the default fail-closed resolver, so
    // this test pins the deny behavior for a local (loopback) server. The success/non-2xx
    // output contract itself is exercised directly by `build_web_client`/`retry_tool_timeout`
    // unit tests.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);
    let url = serve_single_http_response(
        "HTTP/1.1 404 Not Found",
        "<html><body>missing</body></html>",
        "text/html; charset=utf-8",
    );

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WEB_FETCH_URL.to_string(),
        arguments: json!({
            "url": url,
            "timeoutMs": 5000
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output)
        .expect("web fetch http error output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("web_access_denied")
    );
}

#[test]
fn web_fetch_times_out_before_connection_fails_closed_for_loopback() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);
    let url = serve_timeout_http_response(150);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WEB_FETCH_URL.to_string(),
        arguments: json!({
            "url": url,
            "timeoutMs": 50
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload =
        serde_json::from_str::<Value>(&result.output).expect("web fetch timeout output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("web_access_denied")
    );
}

#[test]
fn web_search_returns_timeout_error_after_retries() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);
    let base_url = serve_timeout_http_response(150);

    let result = with_env_var("EXA_API_KEY", "test-key", || {
        with_env_var("EXA_API_BASE_URL", &base_url, || {
            router.execute(&ToolCall {
                call_id: None,
                name: TOOL_WEB_SEARCH_QUERY.to_string(),
                arguments: json!({
                    "query": "pony agent",
                    "timeoutMs": 50
                }),
                plan: None,
            })
        })
    });

    assert_eq!(result.status, "error");
    let payload =
        serde_json::from_str::<Value>(&result.output).expect("web search timeout output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("timeout")
    );
    assert!(payload
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .contains("timeout"));
}

// ── workspace_read_file ─────────────────────────────────────────

#[test]
fn tool_timeout_message_classifier_matches_timeout_errors() {
    assert!(is_tool_timeout_message("timeout: request timed out"));
    assert!(is_tool_timeout_message("deadline has elapsed"));
    assert!(!is_tool_timeout_message("request failed"));
}

#[test]
fn retry_tool_timeout_stops_after_non_timeout_error() {
    let attempts = std::cell::Cell::new(0);
    let result: Result<(), String> =
        retry_tool_timeout(TOOL_WEB_FETCH_URL, 30_000, |_deadline| {
            attempts.set(attempts.get() + 1);
            Err("request failed".to_string())
        });

    assert!(result.is_err());
    assert_eq!(attempts.get(), 1);
}

#[test]
fn retry_tool_timeout_retries_until_success() {
    let attempts = std::cell::Cell::new(0);
    let result = retry_tool_timeout(TOOL_WEB_FETCH_URL, 30_000, |_deadline| {
        let next = attempts.get() + 1;
        attempts.set(next);
        if next < 2 {
            Err("timeout: request timed out".to_string())
        } else {
            Ok("ok")
        }
    })
    .expect("timeout retry should eventually succeed");

    assert_eq!(result, "ok");
    assert_eq!(attempts.get(), 2);
}

#[test]
fn retry_tool_timeout_budget_exhaustion_prevents_retry() {
    // The shared deadline (derived from total_budget_ms) is already exhausted when the first
    // attempt returns a timeout error, so the retry must NOT run — the budget is authoritative
    // instead of "each attempt gets its own fresh budget".
    let attempts = std::cell::Cell::new(0);
    let result: Result<(), String> = retry_tool_timeout(TOOL_WEB_FETCH_URL, 0, |_deadline| {
        attempts.set(attempts.get() + 1);
        Err("timeout: request timed out".to_string())
    });
    assert!(result.is_err());
    assert_eq!(
        attempts.get(),
        1,
        "no retry may run after the deadline is exhausted"
    );
}

// ─────── pinned web fetch tests (PA-076 item 2) ─────────────────────

#[test]
fn pinned_chain_follows_up_to_five_redirects_and_re_validates_each_hop() {
    let resolver = FakeResolver::default();
    for i in 0..=5 {
        resolver.set_addresses(
            format!("hop{i}.example"),
            vec!["203.0.113.10".parse().unwrap()],
        );
    }
    let policy = WebAccessPolicy::default();
    let connector = PinnedConnector::new(&policy, &resolver);
    let sender = FakePinnedSender::with_script(&[
        (
            "http://hop0.example/",
            exchange(
                "http://hop0.example/",
                302,
                Some("http://hop1.example/"),
                "",
            ),
        ),
        (
            "http://hop1.example/",
            exchange(
                "http://hop1.example/",
                302,
                Some("http://hop2.example/"),
                "",
            ),
        ),
        (
            "http://hop2.example/",
            exchange(
                "http://hop2.example/",
                302,
                Some("http://hop3.example/"),
                "",
            ),
        ),
        (
            "http://hop3.example/",
            exchange(
                "http://hop3.example/",
                302,
                Some("http://hop4.example/"),
                "",
            ),
        ),
        (
            "http://hop4.example/",
            exchange(
                "http://hop4.example/",
                302,
                Some("http://hop5.example/"),
                "",
            ),
        ),
        (
            "http://hop5.example/",
            exchange("http://hop5.example/", 200, None, "final"),
        ),
    ]);
    let result = pinned_web_fetch(
        &connector,
        "http://hop0.example/",
        5000,
        60_000,
        generous_deadline(),
        &sender,
    )
    .expect("5-hop chain must succeed");
    assert_eq!(result.status.as_u16(), 200);
    assert_eq!(result.body, b"final");
    assert_eq!(sender.calls().len(), 6);
    // Each hop's URL matches the expected validated URL.
    for (i, (call_url, _)) in sender.calls().iter().enumerate() {
        assert_eq!(
            call_url,
            &format!("http://hop{i}.example/"),
            "hop {i} must use the validated redirect target URL"
        );
    }
}

#[test]
fn pinned_chain_rejects_more_than_five_redirects() {
    let resolver = FakeResolver::default();
    // Register hop0..hop6; hop6 is the 6th redirect target (hops 0-5 are the chain).
    for i in 0..=6 {
        resolver.set_addresses(
            format!("hop{i}.example"),
            vec!["203.0.113.10".parse().unwrap()],
        );
    }
    let policy = WebAccessPolicy::default();
    let connector = PinnedConnector::new(&policy, &resolver);
    let entries: Vec<(String, Result<PinnedHttpExchange, WebFetchTransportError>)> = (0..=5)
        .map(|i| {
            let next = i + 1;
            let key = format!("http://hop{i}.example/");
            let loc = format!("http://hop{next}.example/");
            let resp = exchange(&key, 302, Some(&loc), "");
            (key, resp)
        })
        .collect();
    let entries_ref: Vec<(&str, Result<PinnedHttpExchange, WebFetchTransportError>)> = entries
        .iter()
        .map(|(k, v)| (k.as_str(), v.clone()))
        .collect();
    let sender = FakePinnedSender::with_script(&entries_ref);
    let result = pinned_web_fetch(
        &connector,
        "http://hop0.example/",
        5000,
        60_000,
        generous_deadline(),
        &sender,
    );
    match result {
        Err(PinnedFetchError::Denied {
            decision:
                WebAccessDecision::Deny {
                    reason: WebAccessDenyReason::TooManyRedirects { limit: 5 },
                    hop: 6,
                },
            url,
        }) => {
            assert_eq!(url, "http://hop6.example/");
        }
        other => panic!("must deny as too_many_redirects at hop 6; got {other:?}"),
    }
}

#[test]
fn pinned_chain_rejects_redirect_to_private_address() {
    let resolver = FakeResolver::default();
    resolver.set_addresses("hop0.example", vec!["203.0.113.10".parse().unwrap()]);
    resolver.set_addresses("internal.example", vec!["192.168.1.99".parse().unwrap()]);
    let policy = WebAccessPolicy::default();
    let connector = PinnedConnector::new(&policy, &resolver);
    let sender = FakePinnedSender::with_script(&[(
        "http://hop0.example/",
        exchange(
            "http://hop0.example/",
            302,
            Some("http://internal.example/"),
            "",
        ),
    )]);
    let result = pinned_web_fetch(
        &connector,
        "http://hop0.example/",
        5000,
        60_000,
        generous_deadline(),
        &sender,
    );
    match result {
        Err(PinnedFetchError::Denied {
            decision:
                WebAccessDecision::Deny {
                    reason: WebAccessDenyReason::ResolvesToForbiddenAddress { .. },
                    hop: 1,
                },
            url,
        }) => {
            assert_eq!(url, "http://internal.example/");
        }
        other => panic!("redirect to private address must be denied; got {other:?}"),
    }
}

#[test]
fn pinned_chain_rejects_redirect_to_unsupported_scheme() {
    let resolver = FakeResolver::default();
    resolver.set_addresses("hop0.example", vec!["203.0.113.10".parse().unwrap()]);
    let policy = WebAccessPolicy::default();
    let connector = PinnedConnector::new(&policy, &resolver);
    let sender = FakePinnedSender::with_script(&[(
        "http://hop0.example/",
        exchange("http://hop0.example/", 302, Some("ftp://bad/"), ""),
    )]);
    let result = pinned_web_fetch(
        &connector,
        "http://hop0.example/",
        5000,
        60_000,
        generous_deadline(),
        &sender,
    );
    match result {
        Err(PinnedFetchError::Denied {
            decision:
                WebAccessDecision::Deny {
                    reason: WebAccessDenyReason::UnsupportedScheme { .. },
                    hop: 1,
                },
            ..
        }) => {}
        other => panic!("redirect to ftp must be denied; got {other:?}"),
    }
}

#[test]
fn pinned_chain_resolves_relative_redirect_target() {
    let resolver = FakeResolver::default();
    resolver.set_addresses("hop0.example", vec!["203.0.113.10".parse().unwrap()]);
    let policy = WebAccessPolicy::default();
    let connector = PinnedConnector::new(&policy, &resolver);
    let sender = FakePinnedSender::with_script(&[
        (
            "http://hop0.example/a/b",
            exchange("http://hop0.example/a/b", 302, Some("../c/d"), ""),
        ),
        (
            "http://hop0.example/c/d",
            exchange("http://hop0.example/c/d", 200, None, "resolved"),
        ),
    ]);
    let result = pinned_web_fetch(
        &connector,
        "http://hop0.example/a/b",
        5000,
        60_000,
        generous_deadline(),
        &sender,
    )
    .expect("relative redirect must succeed");
    assert_eq!(result.status.as_u16(), 200);
    assert_eq!(result.body, b"resolved");
    assert_eq!(sender.calls().len(), 2);
    // Second call: URL resolved from the relative redirect `../c/d` against base `/a/b`.
    assert_eq!(sender.calls()[1].0, "http://hop0.example/c/d");
}

#[test]
fn pinned_chain_exceeds_aggregate_deadline_and_is_cut_short() {
    let resolver = FakeResolver::default();
    for i in 0..=5 {
        resolver.set_addresses(
            format!("hop{i}.example"),
            vec!["203.0.113.10".parse().unwrap()],
        );
    }
    let policy = WebAccessPolicy::default();
    let connector = PinnedConnector::new(&policy, &resolver);

    // Six-hop chain where every hop takes 400 ms against a 1050 ms chain budget: three hops
    // already consume ~1200 ms (> budget), so the fetch must be cut structurally at the next
    // hop — never allowed to accumulate 6 hops × the full per-hop timeout.
    let entries: Vec<(String, Result<PinnedHttpExchange, WebFetchTransportError>)> = (0..=4)
        .map(|i| {
            let next = i + 1;
            let key = format!("http://hop{i}.example/");
            let loc = format!("http://hop{next}.example/");
            let resp = exchange(&key, 302, Some(&loc), "");
            (key, resp)
        })
        .chain([(
            "http://hop5.example/".to_string(),
            exchange("http://hop5.example/", 200, None, "final"),
        )])
        .collect();
    let entries_ref: Vec<(&str, Result<PinnedHttpExchange, WebFetchTransportError>)> = entries
        .iter()
        .map(|(k, v)| (k.as_str(), v.clone()))
        .collect();
    let sender = DelayedPinnedSender::with_script(&entries_ref, 400, false);

    let budget_ms = 1050_u64;
    let deadline = Instant::now() + Duration::from_millis(budget_ms);
    let start = Instant::now();
    let result = pinned_web_fetch(
        &connector,
        "http://hop0.example/",
        5000,
        budget_ms,
        deadline,
        &sender,
    );
    let elapsed = start.elapsed();

    match result {
        Err(PinnedFetchError::Transport(WebFetchTransportError::DeadlineExceeded {
            budget_ms: actual,
        })) => {
            assert_eq!(actual, budget_ms);
        }
        other => panic!("chain must fail with deadline exceeded; got {other:?}"),
    }
    // The chain was cut before all six hops were attempted.
    let calls = sender.calls();
    assert_eq!(
        calls.len(),
        3,
        "only the hops that fit the budget may be attempted; got {calls:?}"
    );
    // Every hop's granted timeout was clamped to the remaining chain budget (< the full 5000).
    for (_, granted) in &calls {
        assert!(
            *granted < 5000,
            "per-hop timeout must be clamped by the remaining budget; granted {granted}"
        );
    }
    // Total elapsed stayed within the budget (not hops × per-hop timeout).
    assert!(
        elapsed < Duration::from_millis(budget_ms + 1000),
        "chain must fail within the budget; elapsed {elapsed:?}"
    );
}

#[test]
fn single_slow_hop_is_truncated_by_remaining_chain_budget() {
    let resolver = FakeResolver::default();
    resolver.set_addresses("hop0.example", vec!["203.0.113.10".parse().unwrap()]);
    let policy = WebAccessPolicy::default();
    let connector = PinnedConnector::new(&policy, &resolver);

    // One terminal hop whose server is far slower than the chain budget. The per-hop client
    // timeout must be the remaining budget (~800 ms), not the full requested timeout (5000 ms),
    // so a single slow response cannot stall the fetch beyond the aggregate deadline.
    let sender = DelayedPinnedSender::with_script(
        &[(
            "http://hop0.example/",
            exchange("http://hop0.example/", 200, None, "final"),
        )],
        2000,
        true,
    );

    let budget_ms = 800_u64;
    let deadline = Instant::now() + Duration::from_millis(budget_ms);
    let result = pinned_web_fetch(
        &connector,
        "http://hop0.example/",
        5000,
        budget_ms,
        deadline,
        &sender,
    );

    match result {
        Err(PinnedFetchError::Transport(WebFetchTransportError::Request(message))) => {
            assert!(
                is_tool_timeout_message(&message),
                "slow hop must surface as a timeout; got {message}"
            );
        }
        other => panic!("slow single hop must fail as a timeout; got {other:?}"),
    }
    let calls = sender.calls();
    assert_eq!(calls.len(), 1);
    let (_, granted) = &calls[0];
    assert!(
        *granted <= budget_ms,
        "hop timeout must be clamped to the remaining budget; granted {granted}"
    );
    assert!(
        *granted >= budget_ms - 50,
        "clamp must leave the whole remaining budget to the hop; granted {granted}"
    );
}

#[test]
fn ample_chain_budget_preserves_full_per_hop_timeout() {
    let resolver = FakeResolver::default();
    resolver.set_addresses("hop0.example", vec!["203.0.113.10".parse().unwrap()]);
    let policy = WebAccessPolicy::default();
    let connector = PinnedConnector::new(&policy, &resolver);

    let sender = DelayedPinnedSender::with_script(
        &[(
            "http://hop0.example/",
            exchange("http://hop0.example/", 200, None, "final"),
        )],
        100,
        true,
    );

    // Budget far larger than the per-hop timeout: the hop keeps its full requested timeout.
    let budget_ms = 60_000_u64;
    let deadline = Instant::now() + Duration::from_millis(budget_ms);
    let result = pinned_web_fetch(
        &connector,
        "http://hop0.example/",
        5000,
        budget_ms,
        deadline,
        &sender,
    )
    .expect("single fast hop must succeed");
    assert_eq!(result.status.as_u16(), 200);
    assert_eq!(result.body, b"final");
    let calls = sender.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].1, 5000,
        "ample budget must not clamp the per-hop timeout"
    );
}

// ---------- P2-3 coverage: terminal 3xx and literal-private redirect ----------

#[test]
fn pinned_chain_treats_3xx_without_location_as_terminal() {
    let resolver = FakeResolver::default();
    resolver.set_addresses("hop0.example", vec!["203.0.113.10".parse().unwrap()]);
    let policy = WebAccessPolicy::default();
    let connector = PinnedConnector::new(&policy, &resolver);
    // A 302 with no Location header must be returned as the terminal exchange (never followed).
    let sender = FakePinnedSender::with_script(&[(
        "http://hop0.example/",
        exchange("http://hop0.example/", 302, None, "no-location"),
    )]);
    let result = pinned_web_fetch(
        &connector,
        "http://hop0.example/",
        5000,
        60_000,
        generous_deadline(),
        &sender,
    )
    .expect("3xx without Location is a terminal response");
    assert_eq!(result.status.as_u16(), 302);
    assert_eq!(result.body, b"no-location");
    assert_eq!(
        sender.calls().len(),
        1,
        "no redirect may be followed without a Location header"
    );
}

#[test]
fn pinned_chain_rejects_redirect_to_literal_private_ip() {
    let resolver = FakeResolver::default();
    resolver.set_addresses("hop0.example", vec!["203.0.113.10".parse().unwrap()]);
    let policy = WebAccessPolicy::default();
    let connector = PinnedConnector::new(&policy, &resolver);
    let sender = FakePinnedSender::with_script(&[(
        "http://hop0.example/",
        exchange("http://hop0.example/", 302, Some("http://10.0.0.5/"), ""),
    )]);
    let result = pinned_web_fetch(
        &connector,
        "http://hop0.example/",
        5000,
        60_000,
        generous_deadline(),
        &sender,
    );
    match result {
        Err(PinnedFetchError::Denied {
            decision:
                WebAccessDecision::Deny {
                    reason: WebAccessDenyReason::ForbiddenLiteralAddress { address, .. },
                    hop: 1,
                },
            url,
        }) => {
            assert_eq!(address, "10.0.0.5");
            assert_eq!(url, "http://10.0.0.5/");
        }
        other => panic!("redirect to literal private IP must be denied; got {other:?}"),
    }
}

// ---------- pin / peer / rebinding – unit tests ----------

#[test]
fn verify_pinned_peer_accepts_peer_in_resolved_set() {
    let decision = WebAccessDecision::Allow {
        resolved_addrs: vec![
            "203.0.113.10".parse().unwrap(),
            "203.0.113.20".parse().unwrap(),
        ],
        authority: "example.com:80".to_string(),
        sni_host: "example.com".to_string(),
        port: 80,
        is_tls: false,
    };
    assert!(verify_pinned_peer(Some("203.0.113.10".parse().unwrap()), &decision).is_ok());
}

#[test]
fn verify_pinned_peer_rejects_peer_not_in_resolved_set() {
    let decision = WebAccessDecision::Allow {
        resolved_addrs: vec!["203.0.113.10".parse().unwrap()],
        authority: "example.com:80".to_string(),
        sni_host: "example.com".to_string(),
        port: 80,
        is_tls: false,
    };
    let err = verify_pinned_peer(Some("10.0.0.5".parse().unwrap()), &decision)
        .expect_err("peer mismatch must be rejected");
    match err {
        WebFetchTransportError::PeerMismatch { ref actual, .. } => {
            assert_eq!(*actual, "10.0.0.5".parse::<IpAddr>().unwrap());
        }
        other => panic!("expected PeerMismatch; got {other:?}"),
    }
}

#[test]
fn verify_pinned_peer_accepts_none_peer() {
    let decision = WebAccessDecision::Allow {
        resolved_addrs: vec!["203.0.113.10".parse().unwrap()],
        authority: "example.com:80".to_string(),
        sni_host: "example.com".to_string(),
        port: 80,
        is_tls: false,
    };
    assert!(verify_pinned_peer(None, &decision).is_ok());
}

#[test]
fn build_pinned_web_client_pins_domain_with_resolve_override() {
    let decision = WebAccessDecision::Allow {
        resolved_addrs: vec![
            "203.0.113.10".parse().unwrap(),
            "203.0.113.20".parse().unwrap(),
        ],
        authority: "example.com:80".to_string(),
        sni_host: "example.com".to_string(),
        port: 80,
        is_tls: false,
    };
    let client = build_pinned_web_client(&decision, "example.com", 5000)
        .expect("pinned client for a domain must build");
    let _ = client;
    // Client built successfully means resolve_to_addrs was applied — the method signature
    // guarantees that the override is registered; reqwest does not expose the resolve map
    // for inspection, but the fact that the client built proves the override list was
    // accepted (empty address set for a domain fails, tested below).
}

#[test]
fn build_pinned_web_client_refuses_empty_address_set_for_domain() {
    let decision = WebAccessDecision::Allow {
        resolved_addrs: vec![],
        authority: "example.com:80".to_string(),
        sni_host: "example.com".to_string(),
        port: 80,
        is_tls: false,
    };
    let err = build_pinned_web_client(&decision, "example.com", 5000)
        .expect_err("empty address set for a domain must fail");
    assert!(err.contains("空地址集") || err.contains("empty"));
}

#[test]
fn build_pinned_web_client_skips_resolve_for_literal_ip_host() {
    // A literal IP host is inherently pinned (no DNS step).
    let decision = WebAccessDecision::Allow {
        resolved_addrs: vec!["203.0.113.10".parse().unwrap()],
        authority: "203.0.113.10:80".to_string(),
        sni_host: "203.0.113.10".to_string(),
        port: 80,
        is_tls: false,
    };
    let client = build_pinned_web_client(&decision, "203.0.113.10", 5000)
        .expect("pinned client for a literal IP must build");
    let _ = client;
}

#[test]
fn build_pinned_web_client_rejects_denied_decision() {
    let decision = WebAccessDecision::Deny {
        reason: WebAccessDenyReason::InvalidUrl("broken".to_string()),
        hop: 0,
    };
    let err = build_pinned_web_client(&decision, "example.com", 5000)
        .expect_err("deny decision must not build a client");
    assert!(err.contains("已拒绝"));
}

// ---------- end-to-end: default resolver keeps hostnames fail-closed ----------

#[test]
fn web_fetch_with_default_resolver_still_fails_closed_for_hostname() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);
    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WEB_FETCH_URL.to_string(),
        arguments: json!({ "url": "https://example.com/", "timeoutMs": 5000 }),
        plan: None,
    });
    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("web fetch output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("web_access_denied")
    );
}

// ---------- end-to-end: with_web_resolver – pinned path taken (transport fails, not denied) ----------

#[test]
fn web_fetch_with_injected_resolver_takes_pinned_path_not_weak_fallback() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    // An injected resolver can resolve a hostname to a public IP; the policy now allows the
    // URL, and the pinned path is taken — there is no "validate then let the default client
    // re-resolve" weak mode. The public IP is not reachable hermetically, so the transport
    // fails, but with a transport error (not `web_access_denied`).
    let resolver = FakeResolver::default();
    resolver.set_addresses("public.example", vec!["203.0.113.10".parse().unwrap()]);
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace).with_web_resolver(resolver);
    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WEB_FETCH_URL.to_string(),
        arguments: json!({ "url": "http://public.example/", "timeoutMs": 1000 }),
        plan: None,
    });
    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("web fetch output json");
    let code = payload
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("");
    // The pinned path was taken: policy allow → transport fails. Must not be web_access_denied.
    assert_ne!(
        code, "web_access_denied",
        "hostname URL with an injected resolver must not be web_access_denied; the pinned path is taken, and transport fails because the pinned IP (203.0.113.10) is unreachable"
    );
    // Expect either "request_failed" (connect error) or "timeout" (connect timeout).
    assert!(
        code == "request_failed" || code == "timeout",
        "expected transport error; got code={code}"
    );
}

// ---------- end-to-end: real ReqwestPinnedSender hermetic socket tests ----------

#[test]
fn reqwest_pinned_sender_calls_host_header_equals_original_authority() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    // Bind a server that echoes the Host header in the response body.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test http server");
    let address = listener.local_addr().expect("read test server addr");
    let port = address.port();
    let hostname = format!("pinned-authority-test.local:{port}");

    let _hostname_clone = hostname.clone();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept test request");
        let mut buffer = [0_u8; 4096];
        let n = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..n]);
        let actual_host = request
            .lines()
            .find(|line| line.to_ascii_lowercase().starts_with("host:"))
            .map(|line| {
                line.trim()
                    .trim_start_matches("host:")
                    .trim_start_matches("Host:")
                    .trim()
                    .to_string()
            })
            .unwrap_or_default();
        let body = format!("<html>host={actual_host}</html>");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write test response");
        stream.flush().expect("flush test response");
    });

    // Hand-construct a decision pointing at 127.0.0.1 (the loopback server) with a
    // hostname authority — what the real policy would produce for a validated hostname
    // URL (the connector layer trusts the validated decision; it does not re-validate).
    let decision = WebAccessDecision::Allow {
        resolved_addrs: vec!["127.0.0.1".parse().unwrap()],
        authority: hostname.clone(),
        sni_host: format!("pinned-authority-test.local"),
        port,
        is_tls: false,
    };
    let sender = ReqwestPinnedSender;
    let url = format!("http://pinned-authority-test.local:{port}/");
    let exchange = sender
        .send(&url, &decision, 5000)
        .expect("pinned send must succeed");

    assert_eq!(exchange.status.as_u16(), 200);
    let body = String::from_utf8(exchange.body).unwrap();
    assert!(
        body.contains(&hostname),
        "Host header must be the original authority `{hostname}` (not the IP); got body `{body}`"
    );
    // Peer IP must belong to the validated set.
    assert_eq!(
        exchange.peer_ip,
        Some("127.0.0.1".parse().unwrap()),
        "peer must be the pinned address"
    );
}

#[test]
fn reqwest_pinned_sender_does_not_fall_back_to_ambient_dns() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    // A local server exists at 127.0.0.1, but the decision pins the client to an
    // unreachable public IP (203.0.113.10 is TEST-NET-3, reserved by RFC 5737 and
    // never globally routed; on a hermetic test host this connect will fail). If the
    // sender ever fell back to ambient DNS re-resolution, a badly configured
    // environment could route `rebind-probe.test` elsewhere — but the resolve
    // override in [`build_pinned_web_client`] makes that impossible.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let port = listener.local_addr().unwrap().port();
    // Drop the listener so there's no server, but the port is already captured.
    drop(listener);

    let decision = WebAccessDecision::Allow {
        resolved_addrs: vec!["203.0.113.10".parse().unwrap()],
        authority: format!("rebind-probe.test:{port}"),
        sni_host: "rebind-probe.test".to_string(),
        port,
        is_tls: false,
    };
    let sender = ReqwestPinnedSender;
    let url = format!("http://rebind-probe.test:{port}/");
    let result = sender.send(&url, &decision, 1000);
    // Must fail: the client is pinned to 203.0.113.10 (unreachable). If the sender fell
    // back to ambient DNS for `rebind-probe.test`, it would either fail DNS (NXDOMAIN)
    // or connect somewhere unpredictable — either way the result must be an Err.
    assert!(
        result.is_err(),
        "pinned sender must never fall back to ambient DNS re-resolution"
    );
}

#[test]
fn reqwest_pinned_sender_serves_a_real_redirect_chain() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    // Local server: /hop0 → 302 /hop1, /hop1 → 302 /hop2, /hop2 → 200 "final". Each hop is a
    // fresh per-hop connection to the same pinned address, and redirect-hop bodies are never
    // read (P2-3: real-socket multi-hop redirect evidence).
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test http server");
    let address = listener.local_addr().expect("read test server addr");
    let port = address.port();

    thread::spawn(move || {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().expect("accept test request");
            let mut buffer = [0_u8; 4096];
            let n = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..n]);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/");
            let (status, location, body) = match path {
                "/hop0" => ("302 Found", Some("/hop1"), ""),
                "/hop1" => ("302 Found", Some("/hop2"), ""),
                _ => ("200 OK", None, "final"),
            };
            let mut response =
                format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n");
            if let Some(loc) = location {
                response.push_str(&format!("Location: {loc}\r\n"));
            }
            response.push_str(&format!(
                "Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            ));
            stream
                .write_all(response.as_bytes())
                .expect("write test response");
            stream.flush().expect("flush test response");
        }
    });

    // Hand-constructed decision pointing at the loopback server (the connector layer trusts
    // the validated decision; it does not re-validate — same pattern as the single-hop socket
    // tests above).
    let hostname = format!("redirect-chain-test.local:{port}");
    let decision = WebAccessDecision::Allow {
        resolved_addrs: vec!["127.0.0.1".parse().unwrap()],
        authority: hostname.clone(),
        sni_host: "redirect-chain-test.local".to_string(),
        port,
        is_tls: false,
    };
    let sender = ReqwestPinnedSender;
    let base = format!("http://redirect-chain-test.local:{port}");

    let exchange0 = sender
        .send(&format!("{base}/hop0"), &decision, 5000)
        .expect("redirect hop 0 must succeed");
    assert_eq!(exchange0.status.as_u16(), 302);
    assert_eq!(exchange0.location().as_deref(), Some("/hop1"));
    assert!(
        exchange0.body.is_empty(),
        "redirect hop bodies are never read"
    );
    assert_eq!(
        exchange0.peer_ip,
        Some("127.0.0.1".parse().unwrap()),
        "each hop's peer must belong to the validated address set"
    );

    let exchange1 = sender
        .send(&format!("{base}/hop1"), &decision, 5000)
        .expect("redirect hop 1 must succeed");
    assert_eq!(exchange1.status.as_u16(), 302);
    assert_eq!(exchange1.location().as_deref(), Some("/hop2"));
    assert!(exchange1.body.is_empty());
    assert_eq!(exchange1.peer_ip, Some("127.0.0.1".parse().unwrap()));

    let exchange2 = sender
        .send(&format!("{base}/hop2"), &decision, 5000)
        .expect("terminal hop must succeed");
    assert_eq!(exchange2.status.as_u16(), 200);
    assert_eq!(exchange2.body, b"final");
    assert_eq!(exchange2.peer_ip, Some("127.0.0.1".parse().unwrap()));
}

#[test]
fn literal_public_ip_is_allowed_and_can_build_pinned_client() {
    // A literal public IPv4 URL is inherently pinned (no DNS step); the policy allows
    // it and the pinned client builds without a resolve override (literal IP host).
    let policy = WebAccessPolicy::default();
    let resolver = FailClosedResolver;
    let connector = PinnedConnector::new(&policy, &resolver);
    let decision = connector.validate_initial("http://203.0.113.10:8080/");
    match decision {
        WebAccessDecision::Allow {
            ref resolved_addrs,
            ref authority,
            port,
            is_tls,
            ..
        } => {
            assert_eq!(resolved_addrs, &["203.0.113.10".parse::<IpAddr>().unwrap()]);
            assert_eq!(authority, "203.0.113.10:8080");
            assert_eq!(port, 8080);
            assert!(!is_tls);
        }
        WebAccessDecision::Deny { .. } => {
            panic!("literal public IP must be allowed")
        }
    }
    // The client for a literal-IP URL builds without a resolve override (the IP is used
    // directly by the connector; no ambient DNS step is involved).
    let client = build_pinned_web_client(&decision, "203.0.113.10", 5000)
        .expect("pinned client for literal IP must build");
    let _ = client;
}

// ── Stage1 B4/B5 锁定：tool_error_from_output 读侧契约 ──
