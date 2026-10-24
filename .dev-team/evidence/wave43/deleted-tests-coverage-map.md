# 被删 3 个存量测试的覆盖对照表（Wave43 · task-6 item 4）

> 依据：`git show 1dea72e:crates/pony-agent-core/tests/{ponysentry_trace_test,trace_auto_report_test}.rs`
> + `ponysentry_manual_upload_test.rs`（已重写为 OTel 契约，见文件头注释）。
> 判定口径：①**继承** = 等价断言已存在于存活测试；②**合理消失** = 能力随 D6 下线，
> 语义不复存在或已由新契约的等价面覆盖；③**不该消失** = 保留面仍在，需补测。

## A. `ponysentry_trace_test.rs`（675 行，已删除）

| 被删用例 | 覆盖意图 | 现状归属 | 判定 |
|---|---|---|---|
| `test_eval_status_state_machine_serialization` | `EvalStatus` 状态机序列化 | 类型随 D6 下线（plan Non-Goals：评估标注工作流不迁移） | 合理消失 |
| `test_agent_trace_payload_serialization_contract` | `AgentTracePayload` 序列化契约 | 类型下线；新的 wire 契约由 OT-6（body 字节断言）+ `otel_trace_adversarial.rs` AT-1/AT-2 覆盖 | 合理消失（等价面已迁移到 OTLP） |
| `test_agent_trace_payload_optional_fields_fault_tolerance` | 可选字段容错 | 同上；`TurnTraceRecord` 的 serde/parity 由 `src/agent/runtime/tests.rs`（in-crate）覆盖 | 合理消失 |
| `test_zero_trust_redaction_for_trace_and_secrets` | 脱敏：用户路径 / 凭据 / JSON 递归 | `ponysentry_telemetry_test.rs`：`test_sanitizer_path_redaction` / `test_sanitizer_credential_redaction` / `test_sanitizer_json_recursive_redaction` | **继承（保留面）** |
| `test_client_capture_agent_trace_api_red_phase` | `capture_agent_trace` API 形状 | API 下线 | 合理消失 |
| `test_queue_overflow_and_avalanche_defense` | 有界队列（1024）+ 溢出丢弃 + 入队非阻塞 | **原缺口**：`client.rs` 的 `QUEUE_CAPACITY`/`try_send`/`TrySendError::Full` 仍服务 issue 上报 | **不该消失 → 已补**（新增 `ponysentry_issue_resilience_test.rs`） |
| `test_large_payload_limit_handling` | 大载荷序列化容错 | OTLP 侧 `otel_trace_adversarial.rs::at2_oversized_payload_stays_bounded`；持久化侧 `TurnTraceRecord` 在 `src/agent/runtime/tests.rs` 有 parity 覆盖 | 继承（等价面） |
| `test_contract_started_at_ms_population` | `compute_started_at_ms` | 函数随 trace 映射下线（仅 trace 使用，已无 `fn` 定义） | 合理消失 |
| `test_contract_arguments_summary_compression` | `compact_arguments_summary` | 同上 | 合理消失 |
| `test_contract_tags_injection_environment_info` | `default_runtime_tags` 注入 | 同上 | 合理消失 |

## B. `trace_auto_report_test.rs`（395 行，已删除）

| 被删用例 | 覆盖意图 | 现状归属 | 判定 |
|---|---|---|---|
| `test_is_trace_reportable_terminal_contract` | 终态门控（仅终态上报 trace） | 新语义 = turn 终态生成 `agent.turn` root span（phase 属性） | **不该消失，但当前无可测接缝**：`TurnScope` 构造点在 src 中尚未落地（executor 进行中）→ 已上报 Lead，禁写假测 |
| `test_is_trace_reportable_terminal_short_kind_contract` | short kind 终态判定 | 同上 | 同上 |
| `test_build_agent_trace_from_event_complete_mapping` | 事件 → trace 字段映射 | 取代者：`turn_span_attributes` / `llm_span_attributes` / `tool_span_attributes`（turn 属性 + OT-3 + OT-4） | 继承（纯函数面） |
| `test_build_agent_trace_from_event_missing_required_fields` | 缺字段容错 | OT-3/OT-4「缺失值一律不输出该键」（含 `assert_no_empty_string_attr`） | 继承 |
| `test_build_agent_trace_from_event_short_kind_mapping` | short kind 映射 | 同上（属性映射） | 继承 |
| `test_maybe_report_turn_trace_entrypoint` | 自动上报入口 | 入口随 D6 下线；导出行为由 OT-1（关闭态零 egress）/ AT-3（并发翻转）覆盖，turn span 生产由 OT-2（待接线）覆盖 | 合理消失（入口），行为已迁移 |
| `test_tool_error_string_is_bare_text_not_json_encoded` | 工具错误裸文本 | OT-4：`agent.tool.error` == `"exit status 1"`（裸文本，非 JSON 编码） | 继承 |

## C. 结论

- 16 条被删用例：**继承 6**（脱敏 3 条等价 + 映射 3 条）、**合理消失 9**、**不该消失 2 类**：
  1. 有界队列抗雪崩 → 已补 `ponysentry_issue_resilience_test.rs`（burst 2000 入队 <500ms + 静态有界/丢弃锚点）。
  2. 终态门控 → 保留面语义迁移到 turn span，但 src 侧尚无 `TurnScope` 消费点 ⇒ **无可测接缝，报 Lead**（不得写假断言）。
- 另有 1 项不属于被删测试但同属回归防线：`toolRules` load→normalize→save 往返保留（B7 修复）→ 已补
  `tests/settings.store.spec.ts` 用例。
