# B254 红相收据（2026-11-19）

契约：`.dev-team/contract-matrix-b254.md`

## R1 核心：流式 followup 断流不得回退同步（C1/C2/C4）

命令：`bash scripts/cargo-lane.sh test -p pony-agent-core --test b254_stream_only_followup`
结果：**FAILED（红相成立）**

```
thread 'truncated_followup_stream_retries_in_place_and_never_falls_back_to_sync' panicked at
crates/pony-agent-core/tests/b254_stream_only_followup.rs:229:5:
assertion `left == right` failed: B254-C1：流式失败后不得回退同步 buffered 通道
实际请求体 = ["...\"stream\":true,...", "...\"stream\":false,..."]
  left: 1
 right: 0
```

现场日志同时给出根因链（与线上会话同形）：
```
followup_stream:attempt=1/2 failed class=NonRetryable { details: "读取 provider SSE 流失败: error decoding response body; parsed_bytes=61" } decision=Abort
followup:stream-fallback → request:openai followup-sync body_preview={... "stream":false ...}
followup_sync_fallback:attempt=1/2 failed class=NonRetryable { details: "读取 provider 返回失败：error decoding response body；type=decode" }
followup:stream-error → local_tool_followup_fallback_response
```

## R2 用户文案：followup 阶段文字含技术词（C5）

命令：`bash scripts/cargo-lane.sh test -p pony-agent-core --bin pony -- b254_plain_copy`
结果：**FAILED（红相成立）**

```
B254-C5：followup_phase_text(SyncAttempt { attempt: 2 }) 含技术词「兜底」，实际 = "同步兜底第 2 次…"
test result: FAILED. 2 passed; 1 failed
```

## R3 重锚：B198 阶段位断言

命令：`bash scripts/cargo-lane.sh test -p pony-agent-core --bin pony -- b198_`
结果：**FAILED（红相成立）**

```
实际 = "流式中断，转同步重试…"（期望「重试中」）
failures: b198_phase_keeps_running_status_visible, b198_phase_texts_are_distinguishable
```

## 回归基线（改动前）

`agent::` lib 1645 passed；`b221_fallback_body_acceptance` 5 passed；
`b219_degraded_turn_acceptance` 4 passed；`recovery_acceptance` 6 passed；
`pa172_provider_failover_contract` 11 passed。全部 exit 0。
