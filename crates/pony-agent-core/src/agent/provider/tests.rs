//! provider 单元测试（从 `provider/mod.rs` 拆出，控制单文件规模）。
//!
//! 模块身份不变：仍是 `provider::tests`，`use super::*` 指向 `provider`。


    use super::*;
    use crate::agent::config::ProviderModelCapabilities;
    use crate::agent::config::ResolvedProviderSelection;
    use crate::agent::input::TurnInputImage;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[test]
    fn test_reasoning_model_omits_temperature() {
        let reasoning_config = ResolvedProviderSelection {
            requested_name: "test-reasoning".to_string(),
            provider_name: "openai".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: "https://api.openai.com/v1".to_string(),
            auth_type: ProviderAuthType::Bearer,
            api_key_env_var: "OPENAI_API_KEY".to_string(),
            api_key: Some("test-key".to_string()),
            model: "o1".to_string(),
            temperature: 0.2,
            max_output_tokens: 1024,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                supports_reasoning: true,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::None,
        };

        let non_reasoning_config = ResolvedProviderSelection {
            capabilities: ProviderModelCapabilities {
                supports_reasoning: false,
                ..Default::default()
            },
            ..reasoning_config.clone()
        };

        let initial_body = json!({
            "model": "test-model",
            "messages": [],
            "temperature": 0.2,
            "max_tokens": 1024,
        });

        let reasoning_body = with_openai_request_options(initial_body.clone(), &reasoning_config);
        assert!(
            reasoning_body.get("temperature").is_none(),
            "Expected 'temperature' to be omitted when supports_reasoning is true, but got: {:?}",
            reasoning_body.get("temperature")
        );

        let non_reasoning_body = with_openai_request_options(initial_body, &non_reasoning_config);
        assert!(
            non_reasoning_body.get("temperature").is_some(),
            "Expected 'temperature' to be present when supports_reasoning is false"
        );
        assert_eq!(
            non_reasoning_body.get("temperature").and_then(Value::as_f64),
            Some(0.2)
        );
    }

    #[test]
    fn openai_request_messages_encode_image_blocks_for_last_user_message() {
        let request = ProviderRequest {
            model: "gpt-5.4".to_string(),
            input: vec![
                ProviderMessage::developer("Keep answers short."),
                ProviderMessage::user("请描述这张图"),
            ],
            images: vec![TurnInputImage {
                data_url: "data:image/png;base64,Zm9v".to_string(),
                mime_type: "image/png".to_string(),
                name: Some("demo.png".to_string()),
            }],
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };

        let messages = openai_request_messages(&request);
        let content = messages[1]
            .get("content")
            .and_then(Value::as_array)
            .expect("last user message should use content blocks");

        assert_eq!(content[0].get("type").and_then(Value::as_str), Some("text"));
        assert_eq!(
            content[1]
                .get("image_url")
                .and_then(|value| value.get("url"))
                .and_then(Value::as_str),
            Some("data:image/png;base64,Zm9v")
        );
    }

    #[test]
    fn anthropic_conversation_messages_encode_base64_image_blocks_for_last_user_message() {
        let request = ProviderRequest {
            model: "claude-3-7-sonnet".to_string(),
            input: vec![ProviderMessage::user("请看图回答")],
            images: vec![TurnInputImage {
                data_url: "data:image/png;base64,Zm9v".to_string(),
                mime_type: "image/png".to_string(),
                name: Some("demo.png".to_string()),
            }],
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };

        let messages = anthropic_conversation_messages(&request);
        let content = messages[0]
            .get("content")
            .and_then(Value::as_array)
            .expect("anthropic user message should use content blocks");

        assert_eq!(content[0].get("type").and_then(Value::as_str), Some("text"));
        assert_eq!(
            content[1].get("type").and_then(Value::as_str),
            Some("image")
        );
        assert_eq!(
            content[1]
                .get("source")
                .and_then(|value| value.get("data"))
                .and_then(Value::as_str),
            Some("Zm9v")
        );
    }

    #[test]
    fn build_context_observation_exposes_layered_request_segments() {
        let request = ProviderRequest {
            model: "gpt-4.1-mini".to_string(),
            input: vec![
                ProviderMessage::system("Stable system instruction"),
                ProviderMessage::developer("Stable capability prefix"),
                ProviderMessage::user("Recent context summary"),
                ProviderMessage::user("Actual volatile request"),
            ],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };

        let observation = build_context_observation(
            &request,
            &[ToolDefinition {
                name: "workspace.read_file",
                description: "read file",
                input_schema: json!({ "type": "object" }),
            }],
        );

        assert_eq!(observation.request_format, "normalized-input");
        assert_eq!(observation.message_count, 4);
        assert!(observation
            .stable_prefix_text
            .contains("Stable system instruction"));
        assert!(observation
            .stable_prefix_text
            .contains("Stable capability prefix"));
        assert!(!observation
            .stable_prefix_text
            .contains("Actual volatile request"));
        assert!(observation
            .semi_stable_context_text
            .contains("Recent context summary"));
        assert!(observation
            .volatile_input_text
            .contains("Actual volatile request"));
        assert_eq!(observation.tool_count, 1);
    }

    #[test]
    fn openai_tools_payload_uses_product_tool_surface_for_builtin_tools() {
        let payload = openai_tools_payload(&crate::agent::tools::builtin_tools());
        let names = payload
            .iter()
            .filter_map(|entry| {
                entry
                    .get("function")
                    .and_then(|value| value.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "Run".to_string(),
                "Ask".to_string(),
                "Read".to_string(),
                "ReadDocument".to_string(),
                "List".to_string(),
                "Search".to_string(),
                "Glob".to_string(),
                "WebFetch".to_string(),
                "WebSearch".to_string(),
                "MCPResource".to_string(),
                "ToolSearch".to_string(),
                "Write".to_string(),
                "Edit".to_string(),
                "BatchExecute".to_string(),
                "Plan".to_string(),
                "ViewImage".to_string(),
                // Wave-61 C1：9 项团队工具进入模型可见面（按 builtin_tools 定义序插在
                // ViewImage 之后、TodoWrite 之前；`workflow` 桩实现保持 Internal）。
                "subagent".to_string(),
                "spawn_teammate".to_string(),
                "list_agents".to_string(),
                "send_message".to_string(),
                "interrupt_agent".to_string(),
                "drain_inbox".to_string(),
                "team_task_create".to_string(),
                "team_task_update".to_string(),
                "team_task_list".to_string(),
                "TodoWrite".to_string(),
                "TodoList".to_string(),
                // W20 goal 工具为模型可见面成员（实测 surface 尾部）。
                "create_goal".to_string(),
                "get_goal".to_string(),
                "update_goal".to_string(),
                // W37-C1：七件新模型可见工具（present/fuzzy_file_search/git_diff_remote/job_*）。
                "Present".to_string(),
                "Find".to_string(),
                "GitDiff".to_string(),
                "JobStart".to_string(),
                "JobOutput".to_string(),
                "JobKill".to_string(),
                "JobList".to_string(),
                // W42/PA-149：技能正文按需加载。
                "skill_load".to_string(),
                "acp_delegate".to_string() // B150/P1：ACP 委派（模型面 38 → 39）
            ]
        );
    }

    #[test]
    fn fallback_tool_payload_filters_internal_builtin_but_keeps_dynamic_tools() {
        // W28 附带必修 + Wave-61 C1/C1.7 迁移（契约 C4.4「面④」）。
        //
        // 迁移理由（真值源）：C1 之后团队工具并入 `ModelVisible`，但 `workflow` 因
        // `orchestration::workflow()` 仍是桩实现（只回显 step.id + status="success"）被
        // L3-P 裁决**保持 Internal**。因此 `builtin_tools()` 的 Internal 集合恰为
        // {`workflow`}：既保留了本用例「Internal builtin 必须被 fallback 摘除」的原始立意，
        // 又把集合钉成精确值（回归守卫：任何 builtin 的 exposure 漂移都会让本用例变红）。
        let builtin_names: Vec<String> = crate::agent::tools::builtin_tools()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        let internal_builtins: Vec<&String> = builtin_names
            .iter()
            .filter(|name| is_internal_builtin_tool_name(name))
            .collect();
        let internal_names: Vec<&str> = internal_builtins.iter().map(|name| name.as_str()).collect();
        assert_eq!(
            internal_names,
            vec!["workflow"],
            "Wave-61 L3-P 后 builtin_tools() 的 exposure=Internal 集合必须恰为 [workflow]\
             （模型面只允许通过 fallback 摘除这一个桩实现）；实际：{internal_names:?}"
        );

        let tools = vec![
            ToolDefinition {
                name: "workflow",
                description: "wave-61：仍为 Internal 的桩实现",
                input_schema: json!({"type": "object"}),
            },
            ToolDefinition {
                name: "subagent",
                description: "wave-61：已 ModelVisible 的 builtin",
                input_schema: json!({"type": "object"}),
            },
            ToolDefinition {
                name: "mcp__demo__echo",
                description: "dynamic tool",
                input_schema: json!({"type": "object"}),
            },
        ];
        let payload = anthropic_tools_payload(&tools);
        let names: Vec<&str> = payload
            .iter()
            .filter_map(|entry| entry.get("name").and_then(Value::as_str))
            .collect();
        assert_eq!(names, vec!["subagent", "mcp__demo__echo"]);
    }

    #[test]
    fn anthropic_tools_payload_uses_product_tool_surface_for_builtin_tools() {
        let payload = anthropic_tools_payload(&crate::agent::tools::builtin_tools());
        let names = payload
            .iter()
            .filter_map(|entry| {
                entry
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "Run".to_string(),
                "Ask".to_string(),
                "Read".to_string(),
                "ReadDocument".to_string(),
                "List".to_string(),
                "Search".to_string(),
                "Glob".to_string(),
                "WebFetch".to_string(),
                "WebSearch".to_string(),
                "MCPResource".to_string(),
                "ToolSearch".to_string(),
                "Write".to_string(),
                "Edit".to_string(),
                "BatchExecute".to_string(),
                "Plan".to_string(),
                "ViewImage".to_string(),
                // Wave-61 C1：9 项团队工具进入模型可见面（按 builtin_tools 定义序插在
                // ViewImage 之后、TodoWrite 之前；`workflow` 桩实现保持 Internal）。
                "subagent".to_string(),
                "spawn_teammate".to_string(),
                "list_agents".to_string(),
                "send_message".to_string(),
                "interrupt_agent".to_string(),
                "drain_inbox".to_string(),
                "team_task_create".to_string(),
                "team_task_update".to_string(),
                "team_task_list".to_string(),
                "TodoWrite".to_string(),
                "TodoList".to_string(),
                // W20 goal 工具为模型可见面成员（实测 surface 尾部）。
                "create_goal".to_string(),
                "get_goal".to_string(),
                "update_goal".to_string(),
                // W37-C1：七件新模型可见工具。
                "Present".to_string(),
                "Find".to_string(),
                "GitDiff".to_string(),
                "JobStart".to_string(),
                "JobOutput".to_string(),
                "JobKill".to_string(),
                "JobList".to_string(),
                // W42/PA-149：技能正文按需加载。
                "skill_load".to_string(),
                "acp_delegate".to_string() // B150/P1：ACP 委派（模型面 38 → 39）
            ]
        );
        assert!(payload.iter().all(|tool| {
            tool.get("description")
                .and_then(Value::as_str)
                .is_some_and(|description| !description.trim().is_empty())
        }));
        // PA-076 P2-6：尾部追加序（W37-C1 job_* → W42 skill_load → B150/P1 acp_delegate）；
        // 前缀顺序不变、仅尾部变化（cache-friendly stable prefix invariant，Decision 10）。
        let last_two = &names[names.len() - 2..];
        assert_eq!(
            last_two,
            &["skill_load".to_string(), "acp_delegate".to_string()]
        );
    }

    #[test]
    fn anthropic_native_messages_preserve_tool_use_result_and_thinking_signature() {
        let tool_call = ToolCall {
            call_id: Some("toolu_123".to_string()),
            name: "List".to_string(),
            arguments: json!({ "path": ".", "description": "列出当前目录" }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "List".to_string(),
            status: "ok".to_string(),
            output: "Cargo.toml\nsrc".to_string(),
            duration_ms: 1,
        };
        let thinking = json!([{
            "type": "thinking",
            "thinking": "先读取目录。",
            "signature": "signature_123"
        }]);

        let assistant = provider_native_assistant_tool_call_message_for_protocol(
            &ProviderProtocol::AnthropicMessages,
            Some("我先列出目录。"),
            Some(&thinking),
            &tool_call,
        );
        let result = provider_native_tool_result_message_for_protocol(
            &ProviderProtocol::AnthropicMessages,
            &tool_call,
            &tool_result,
        );
        let request = ProviderRequest {
            model: "claude-sonnet-5".to_string(),
            input: vec![ProviderMessage::user("当前文件夹下有哪些文件？")],
            images: Vec::new(),
            native_messages: vec![assistant.clone(), result.clone()],
            observation: ProviderRequestObservation::default(),
            temperature: 0.0,
            max_output_tokens: 1024,
        };

        assert_eq!(
            anthropic_conversation_messages(&request),
            vec![assistant, result.clone()]
        );
        assert_eq!(result["content"][0]["type"].as_str(), Some("tool_result"));
        assert_eq!(
            result["content"][0]["tool_use_id"].as_str(),
            Some("toolu_123")
        );
        assert_eq!(
            request.native_messages[0]["content"][0]["signature"].as_str(),
            Some("signature_123")
        );
    }

    #[test]
    fn anthropic_conversation_messages_skip_openai_native_transcripts() {
        let request = ProviderRequest {
            model: "claude-sonnet-5".to_string(),
            input: vec![ProviderMessage::user("当前文件夹下有哪些文件？")],
            images: Vec::new(),
            native_messages: vec![
                json!({
                    "role": "assistant",
                    "content": "我先列出目录。",
                    "reasoning_content": null,
                    "tool_calls": []
                }),
                json!({
                    "role": "tool",
                    "tool_call_id": "call_list",
                    "content": "Cargo.toml\nsrc"
                }),
            ],
            observation: ProviderRequestObservation::default(),
            temperature: 0.0,
            max_output_tokens: 1024,
        };

        assert_eq!(
            anthropic_conversation_messages(&request),
            vec![json!({
                "role": "user",
                "content": "当前文件夹下有哪些文件？"
            })]
        );
    }

    /// P1-1：多调用被丢弃时必须有**结构化告警行**（不再静默截断）。
    #[test]
    fn openai_multiple_tool_calls_emit_structured_dropped_warning() {
        let message = json!({
            "tool_calls": [
                { "id": "call_1", "type": "function",
                  "function": { "name": "Read", "arguments": "{\"path\":\"a.txt\"}" } },
                { "id": "call_2", "type": "function",
                  "function": { "name": "List", "arguments": "{}" } },
            ]
        });
        let kept = extract_openai_tool_call(&message);
        let line = openai_dropped_tool_calls_warning(
            "openai_decision",
            &message,
            kept.as_ref(),
        )
        .expect("P1-1：2 个 tool_calls 必须产出告警行");
        // 真实日志通道（与生产同一路径），便于 --nocapture 取证。
        provider_log(line.clone());
        eprintln!("[D] dropped-warning line = {line}");

        assert!(line.contains("received=2"), "P1-1：必须带 received=2，实际 {line}");
        assert!(line.contains("dropped=1"), "P1-1：必须带 dropped=1，实际 {line}");
        assert!(
            line.contains("List"),
            "P1-1：dropped_names 必须列出被丢弃的调用名，实际 {line}"
        );
        assert!(
            line.contains(r#"kept=Some("Read")"#),
            "P1-1：kept 必须标明实际执行的首个调用，实际 {line}"
        );

        // 单调用/无调用 ⇒ 不得产生噪音告警。
        let single = json!({ "tool_calls": [ { "id": "call_1", "type": "function",
            "function": { "name": "Read", "arguments": "{}" } } ] });
        assert!(openai_dropped_tool_calls_warning("s", &single, None).is_none());
        assert!(openai_dropped_tool_calls_warning("s", &json!({}), None).is_none());
    }

    // ───────────── D1b：批量回填构造器（纯函数）的形状锁 ─────────────

    fn batch_fixture() -> (Vec<Value>, Value, Vec<ToolCall>, Vec<ToolResult>) {
        let base = vec![json!({ "role": "user", "content": "do three things" })];
        let calls = vec![
            ToolCall {
                call_id: Some("call_1".to_string()),
                name: "Read".to_string(),
                arguments: json!({ "path": "a.txt" }),
                plan: None,
            },
            ToolCall {
                call_id: Some("call_2".to_string()),
                name: "List".to_string(),
                arguments: json!({}),
                plan: None,
            },
            ToolCall {
                call_id: Some("call_3".to_string()),
                name: "Search".to_string(),
                arguments: json!({ "query": "x" }),
                plan: None,
            },
        ];
        let results = ["A", "B", "C"]
            .iter()
            .map(|text| ToolResult {
                tool_name: "t".to_string(),
                status: "ok".to_string(),
                output: text.to_string(),
                duration_ms: 0,
            })
            .collect();
        let assistant = json!({
            "role": "assistant",
            "content": null,
            "reasoning_content": "thinking",
            "tool_calls": calls.iter().map(|call| json!({
                "id": call.call_id.clone().unwrap(),
                "type": "function",
                "function": {
                    "name": call.name.clone(),
                    "arguments": call.arguments.to_string()
                }
            })).collect::<Vec<_>>()
        });
        (base, assistant, calls, results)
    }

    /// OpenAI：恰 1 条 assistant（含全部 tool_calls）+ N 条 role=tool（各自 tool_call_id）。
    #[test]
    fn openai_batch_tool_results_shape_is_one_assistant_and_n_tool_messages() {
        let (base, assistant, calls, results) = batch_fixture();
        let messages = openai_messages_with_tool_results(&base, &assistant, &calls, &results)
            .expect("D1b：长度一致必须成功");
        eprintln!(
            "[D1b] openai batch messages = {}",
            serde_json::to_string(&messages).expect("serialize")
        );

        assert_eq!(
            messages.len(),
            1 + 1 + calls.len(),
            "D1b：= 基础消息 + 1 assistant + N tool"
        );
        let assistants = messages
            .iter()
            .filter(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
            .count();
        assert_eq!(assistants, 1, "D1b：assistant 必须恰 1 条（N 条即为上游 400 事故）");
        let tool_messages: Vec<&Value> = messages
            .iter()
            .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
            .collect();
        assert_eq!(tool_messages.len(), calls.len(), "D1b：role=tool 必须恰 N 条");
        for (index, (message, result)) in tool_messages.iter().zip(results.iter()).enumerate() {
            assert_eq!(
                message.get("tool_call_id").and_then(Value::as_str),
                calls[index].call_id.as_deref(),
                "D1b：tool_call_id 必须与调用一一对应（保序）"
            );
            assert_eq!(
                message.get("content").and_then(Value::as_str),
                Some(result.output.as_str()),
                "D1b：tool 消息 content 必须是该调用结果"
            );
        }
        let assistant_message = messages
            .iter()
            .find(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
            .expect("assistant 存在");
        assert_eq!(
            assistant_message
                .get("tool_calls")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(calls.len()),
            "D1b：assistant 必须承载全部 tool_calls"
        );
    }

    /// Anthropic：恰 1 条 assistant（N 个 tool_use）+ 恰 1 条 user（N 个 tool_result）。
    #[test]
    fn anthropic_batch_tool_results_shape_is_one_assistant_and_one_user() {
        let (base, assistant, calls, results) = batch_fixture();
        let messages = anthropic_messages_with_tool_results(&base, &assistant, &calls, &results)
            .expect("D1b：长度一致必须成功");
        eprintln!(
            "[D1b] anthropic batch messages = {}",
            serde_json::to_string(&messages).expect("serialize")
        );

        assert_eq!(messages.len(), 1 + 2, "D1b：= 基础消息 + 1 assistant + 1 user");
        let assistants = messages
            .iter()
            .filter(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
            .count();
        assert_eq!(
            assistants, 1,
            "D1b（负向护栏）：anthropic 批量回填的 assistant 必须恰 1 条，N 条会触发上游 400"
        );
        let users: Vec<&Value> = messages
            .iter()
            .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
            .collect();
        assert_eq!(users.len(), 2, "D1b：基础 user + 回填 user");
        let tool_results: Vec<&Value> = users
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
            .expect("回填 user 的 content 必须是数组")
            .iter()
            .collect();
        assert_eq!(tool_results.len(), calls.len(), "D1b：tool_result 块必须恰 N 个");
        let tool_uses = messages
            .iter()
            .find(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
            .expect("assistant content 必须是数组");
        assert_eq!(
            tool_uses
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
                .count(),
            calls.len(),
            "D1b：assistant 必须承载全部 tool_use"
        );
        for (index, block) in tool_results.iter().enumerate() {
            assert_eq!(
                block.get("tool_use_id").and_then(Value::as_str),
                calls[index].call_id.as_deref(),
                "D1b：tool_use_id 必须与调用一一对应（保序）"
            );
        }
    }

    /// 长度不匹配 ⇒ Err（带两个真实长度）。
    #[test]
    fn batch_tool_results_reject_length_mismatch() {
        let (base, assistant, calls, mut results) = batch_fixture();
        results.pop();
        let openai_error = openai_messages_with_tool_results(&base, &assistant, &calls, &results)
            .expect_err("D1b：长度不匹配必须 Err");
        let anthropic_error =
            anthropic_messages_with_tool_results(&base, &assistant, &calls, &results)
                .expect_err("D1b：长度不匹配必须 Err");
        eprintln!("[D1b] mismatch errors = {openai_error} | {anthropic_error}");
        for error in [openai_error, anthropic_error] {
            assert!(error.contains("calls=3"), "D1b：错误必须带 calls 长度：{error}");
            assert!(error.contains("results=2"), "D1b：错误必须带 results 长度：{error}");
        }
    }

    /// N=1 时与既有 1:1 构造器的形状语义等价（避免第二套口径）。
    #[test]
    fn batch_tool_results_single_call_matches_legacy_shape() {
        let (base, assistant, calls, results) = batch_fixture();
        let single_call = vec![calls[0].clone()];
        let single_result = vec![results[0].clone()];

        let batch = openai_messages_with_tool_results(
            &base,
            &assistant,
            &single_call,
            &single_result,
        )
        .expect("D1b：单调用批量构造必须成功");
        let legacy_assistant = openai_assistant_message_for_tool_result(
            Some(&assistant),
            &calls[0],
        );
        let legacy_tool = openai_followup_tool_result_message(&calls[0], &results[0]);
        eprintln!(
            "[D1b] single-call openai: batch_assistant={} legacy_assistant={}",
            batch[batch.len() - 2], legacy_assistant
        );
        assert_eq!(
            batch[batch.len() - 2].get("tool_calls"),
            Some(&json!([legacy_assistant
                .get("tool_calls")
                .and_then(Value::as_array)
                .and_then(|arr| arr.first())
                .cloned()
                .expect("legacy assistant 含 1 个 tool_call")])),
            "D1b：单调用时 assistant 的 tool_calls 必须与 legacy 单值构造一致"
        );
        assert_eq!(
            batch[batch.len() - 1], legacy_tool,
            "D1b：单调用时 tool 消息必须与 legacy 单值构造逐字一致"
        );

        let anthropic_batch = anthropic_messages_with_tool_results(
            &base,
            &assistant,
            &single_call,
            &single_result,
        )
        .expect("D1b：单调用批量构造必须成功");
        let legacy_content = anthropic_assistant_content_for_tool_result(Some(&assistant), &calls[0]);
        assert_eq!(
            anthropic_batch[anthropic_batch.len() - 2].get("content"),
            Some(&Value::Array(legacy_content)),
            "D1b：单调用时 anthropic assistant content 必须与 legacy 一致"
        );
    }

    /// D3b 收尾（原 `anthropic_rejects_multiple_tool_calls`，缺 `#[test]` 时从未运行）：
    /// 批量执行已落地 ⇒ 多 `tool_use` **不再拒绝**，解析层按出现顺序返回整批。
    #[test]
    fn anthropic_accepts_multiple_tool_calls() {
        let content = vec![
            json!({ "type": "tool_use", "id": "toolu_1", "name": "List", "input": {} }),
            json!({ "type": "tool_use", "id": "toolu_2", "name": "Read", "input": {} }),
        ];

        let calls = extract_anthropic_tool_calls(&content);
        assert_eq!(
            calls.len(),
            2,
            "D3b：2 个 tool_use 必须原样返回 2 个调用（不再 Err），实际 {calls:?}"
        );
        assert_eq!(calls[0].name, "List");
        assert_eq!(calls[0].call_id.as_deref(), Some("toolu_1"));
        assert_eq!(calls[1].name, "Read");
        assert_eq!(calls[1].call_id.as_deref(), Some("toolu_2"));
    }

    #[test]
    fn render_tool_definitions_uses_product_tool_surface_for_builtin_tools() {
        let rendered = render_tool_definitions(&crate::agent::tools::builtin_tools());

        for tool_name in [
            "Run",
            "Ask",
            "Read",
            "List",
            "Search",
            "Glob",
            "WebFetch",
            "WebSearch",
            "MCPResource",
            "ToolSearch",
            "Write",
            "Edit",
            "BatchExecute",
        ] {
            assert!(
                rendered.contains(&format!("] {}", tool_name)),
                "missing tool heading for {tool_name}: {rendered}"
            );
        }

        for legacy_name in [
            "time_now",
            "echo_input",
            "workspace_list_files",
            "workspace_gather_context",
            "workspace_search_text",
            "workspace_glob_files",
            "web_fetch_url",
            "web_search_query",
            "mcp_resource_read",
            "tool_search",
            "workspace_write_file",
            "workspace_edit_file",
            "workspace_run_command",
            "workspace_batch",
        ] {
            assert!(
                !rendered.contains(&format!("] {}", legacy_name)),
                "legacy tool heading leaked into rendered definitions: {legacy_name}"
            );
        }
    }

    /// ADR 0065 decision 4 / T2 交界不变量一：**纯内建数组**的输出必须与旧
    /// `is_builtin_tool_definition_set == true` 分支逐字节相同（39 项产品面、去重、registry 序）。
    #[test]
    fn surface_contract_views_is_byte_identical_for_the_pure_builtin_array() {
        let builtin = crate::agent::tools::builtin_tools();
        assert_eq!(
            surface_contract_views(&builtin),
            builtin_provider_contract_views(),
            "内建数组必须原样走 registry 投影（不得因重构丢去重/顺序/字段）"
        );
        assert_eq!(
            surface_contract_views(&builtin).len(),
            39,
            "模型可见产品面 39 项（Wave-61 的 38 项 + B150/P1 acp_delegate）"
        );
    }

    /// 交界不变量二：外部工具**只追加、不改序**。追加一个 `mcp__*` 条目后，内建 39 项必须
    /// 逐字节不变且保持前缀，外部条目按数组顺序排在最后（KV-cache 前缀稳定）。
    #[test]
    fn surface_contract_views_appends_external_tools_after_an_unchanged_builtin_prefix() {
        let mut tools = crate::agent::tools::builtin_tools();
        tools.push(ToolDefinition {
            name: "mcp__demo__echo",
            description: "external echo",
            input_schema: json!({ "type": "object", "properties": {} }),
        });
        tools.push(ToolDefinition {
            name: "mcp__demo__zebra",
            description: "external zebra",
            input_schema: json!({ "type": "object", "properties": {} }),
        });

        let views = surface_contract_views(&tools);
        let builtin_views = builtin_provider_contract_views();
        assert_eq!(
            views.len(),
            builtin_views.len() + 2,
            "外部条目只能追加"
        );
        assert_eq!(
            &views[..builtin_views.len()],
            &builtin_views[..],
            "内建前缀必须逐字节不变（顺序/去重/字段）"
        );
        assert_eq!(
            views[builtin_views.len()..]
                .iter()
                .map(|view| view.name.as_str())
                .collect::<Vec<_>>(),
            vec!["mcp__demo__echo", "mcp__demo__zebra"],
            "外部工具按数组顺序追加，且名字原样透传（不得被映射成内建产品名）"
        );

        // 单个外部工具也不得把内建面拖进 fallback（旧的二选一分支会产出 44 条重复视图）。
        assert_ne!(
            views.len(),
            crate::agent::tools::builtin_tools().len(),
            "绝不能退化成逐条 legacy fallback（那会重复 Run/Read 等产品名）"
        );
    }

    #[test]
    fn provider_does_not_infer_builtin_surface_from_tool_count() {
        let decoys = (0..crate::agent::tools::builtin_tools().len())
            .map(|index| ToolDefinition {
                name: "decoy",
                description: "decoy",
                input_schema: json!({
                    "type": "object",
                    "properties": { "index": { "const": index } }
                }),
            })
            .collect::<Vec<_>>();

        let payload = openai_tools_payload(&decoys);
        assert_eq!(payload.len(), decoys.len());
        assert!(payload.iter().all(|entry| {
            entry
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                == Some("decoy")
        }));
    }

    /// ADR 0065 decision 4：集合身份来自 registry 快照事实，不再来自数组长度/内容比对。
    #[test]
    fn builtin_tool_set_identity_comes_from_registry_facts_not_array_shape() {
        let builtin = crate::agent::tools::builtin_tools();
        assert!(
            is_builtin_tool_definition_set(&builtin),
            "the registry's own definition list must be recognized as the builtin surface"
        );

        // 内容（description / schema）不参与身份判定：名字由快照承认即可。
        let rewritten = builtin
            .iter()
            .map(|definition| ToolDefinition {
                name: definition.name,
                description: "rewritten by a caller",
                input_schema: json!({ "type": "object", "properties": {} }),
            })
            .collect::<Vec<_>>();
        assert!(
            is_builtin_tool_definition_set(&rewritten),
            "identity is a snapshot fact, so caller-rewritten descriptions/schemas must not flip the branch"
        );

        // 子集不算内建面：否则会把 planner 的部分工具视图静默扩张成完整工具面。
        let subset = builtin.iter().take(5).cloned().collect::<Vec<_>>();
        assert!(!is_builtin_tool_definition_set(&subset));

        // 快照不认识的动态名字一律不属于内建面。
        let mut with_dynamic = builtin.clone();
        with_dynamic.push(ToolDefinition {
            name: "mcp__demo__echo",
            description: "dynamic tool",
            input_schema: json!({ "type": "object" }),
        });
        assert!(!is_builtin_tool_definition_set(&with_dynamic));

        assert!(!is_builtin_tool_definition_set(&[]));
    }

    #[test]
    fn build_context_observation_prefers_explicit_layer_metadata() {
        let request = ProviderRequest {
            model: "gpt-5.4".to_string(),
            input: vec![ProviderMessage::user("fallback text should not win")],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation {
                stable_prefix_text: "stable prefix".to_string(),
                semi_stable_context_text: "semi-stable context".to_string(),
                volatile_input_text: "actual request".to_string(),
                prefix_mutation_reasons: vec![PrefixMutationReason::SessionSummaryChanged],
                context_refresh_reason: None,
                instruction_scope_sources: Vec::new(),
                conversation_carry_mode: None,
            },
            temperature: 0.2,
            max_output_tokens: 1024,
        };

        let observation = build_context_observation(&request, &[]);

        assert_eq!(observation.stable_prefix_text, "stable prefix");
        assert_eq!(observation.semi_stable_context_text, "semi-stable context");
        assert_eq!(observation.volatile_input_text, "actual request");
        assert!(!observation
            .stable_prefix_text
            .contains("fallback text should not win"));
        assert_eq!(
            observation.prefix_mutation_reasons,
            vec![PrefixMutationReason::SessionSummaryChanged]
        );
    }

    #[test]
    fn build_context_observation_fallback_keeps_dynamic_system_and_developer_text_out_of_stable_prefix(
    ) {
        let request = ProviderRequest {
            model: "gpt-5.4".to_string(),
            input: vec![
                ProviderMessage::system("Stable system instruction"),
                ProviderMessage::developer(
                    "Capability profile: contextWindowTokens=128000 / supportsImageInput=true.",
                ),
                ProviderMessage::developer(
                    "Session summary: volatile summary / graph=graph-a / session=session-1",
                ),
                ProviderMessage::user("actual request"),
            ],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };

        let observation = build_context_observation(&request, &[]);

        assert!(observation
            .stable_prefix_text
            .contains("Stable system instruction"));
        assert!(observation
            .stable_prefix_text
            .contains("Capability profile:"));
        assert!(!observation
            .stable_prefix_text
            .contains("Session summary: volatile summary"));
        assert!(observation
            .semi_stable_context_text
            .contains("Session summary: volatile summary"));
    }

    #[test]
    fn tool_capability_disabled_removes_openai_tool_fields() {
        let config = ResolvedProviderSelection {
            requested_name: "openai".to_string(),
            provider_name: "openai".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: "https://api.openai.com/v1".to_string(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "OPENAI_API_KEY".to_string(),
            api_key: Some("test".to_string()),
            model: "gpt-4.1-mini".to_string(),
            temperature: 0.2,
            max_output_tokens: 8192,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(128_000),
                supports_tools: false,
                supports_streaming: true,
                supports_image_input: true,
                supports_reasoning: false,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::None,
        };

        let body = apply_openai_tool_capability(
            json!({
                "tool_choice": "auto",
                "tools": [{ "type": "function" }]
            }),
            &[ToolDefinition {
                name: "workspace.read_file",
                description: "read file",
                input_schema: json!({ "type": "object" }),
            }],
            &config,
        );

        assert!(body.get("tool_choice").is_none());
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn deepseek_v4_keeps_openai_tool_choice() {
        let config = ResolvedProviderSelection {
            requested_name: "deepseek".to_string(),
            provider_name: "deepseek".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: "https://api.deepseek.com/v1".to_string(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "DEEPSEEK_API_KEY".to_string(),
            api_key: Some("test".to_string()),
            model: "deepseek-v4-pro".to_string(),
            temperature: 0.2,
            max_output_tokens: 8192,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(128_000),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: true,
                supports_reasoning: true,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::EffortWithNone,
        };

        let body = apply_openai_tool_capability(
            json!({
                "tool_choice": "auto",
                "tools": [{ "type": "function", "function": { "name": "workspace_read_file" } }]
            }),
            &[ToolDefinition {
                name: "workspace.read_file",
                description: "read file",
                input_schema: json!({ "type": "object" }),
            }],
            &config,
        );

        assert!(body.get("tool_choice").is_some());
        assert!(body.get("tools").is_some());
    }

    #[test]
    fn tool_capability_disabled_removes_anthropic_tool_fields() {
        let config = ResolvedProviderSelection {
            requested_name: "anthropic".to_string(),
            provider_name: "anthropic".to_string(),
            protocol: ProviderProtocol::AnthropicMessages,
            base_url: "https://api.anthropic.com/v1".to_string(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "ANTHROPIC_API_KEY".to_string(),
            api_key: Some("test".to_string()),
            model: "claude-3-7-sonnet-latest".to_string(),
            temperature: 0.2,
            max_output_tokens: 8192,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(200_000),
                supports_tools: false,
                supports_streaming: true,
                supports_image_input: true,
                supports_reasoning: true,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::AnthropicThinking,
        };

        let body = apply_anthropic_tool_capability(
            json!({
                "tools": [{ "name": "workspace.read_file" }],
                "tool_choice": { "type": "auto" }
            }),
            &[ToolDefinition {
                name: "workspace.read_file",
                description: "read file",
                input_schema: json!({ "type": "object" }),
            }],
            &config,
        );

        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
    }

    #[test]
    fn openai_tool_followup_replays_reasoning_content() {
        let request = ProviderRequest {
            model: "deepseek-v4-pro".to_string(),
            input: vec![ProviderMessage::user(
                "检查 src-tauri/src/agent/provider.rs",
            )],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };
        let tool_call = ToolCall {
            call_id: Some("call_123".to_string()),
            name: "workspace.read_file".to_string(),
            arguments: json!({ "path": "src-tauri/src/agent/provider.rs" }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "workspace.read_file".to_string(),
            status: "ok".to_string(),
            output: "file content".to_string(),
            duration_ms: 12,
        };

        let messages = openai_messages_with_tool_result(
            &request,
            &mut vec![],
            Some(&provider_native_assistant_tool_call_message(
                Some("先读取文件，再给出结论。"),
                Some("先确认 provider 协议分支，再继续调用工具。"),
                &tool_call,
            )),
            &tool_call,
            &tool_result,
        );

        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages[1].get("reasoning_content").and_then(Value::as_str),
            Some("先确认 provider 协议分支，再继续调用工具。")
        );
    }

    #[test]
    fn openai_tool_followup_preserves_structured_reasoning_content() {
        let request = ProviderRequest {
            model: "deepseek-v4-pro".to_string(),
            input: vec![ProviderMessage::user("继续处理工具结果")],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };
        let tool_call = ToolCall {
            call_id: Some("call_structured".to_string()),
            name: "workspace.read_file".to_string(),
            arguments: json!({ "path": "src-tauri/src/agent/provider.rs" }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "workspace.read_file".to_string(),
            status: "ok".to_string(),
            output: "file content".to_string(),
            duration_ms: 12,
        };
        let structured_reasoning = json!([
            { "type": "reasoning", "text": "先确认工具结果。" },
            { "type": "reasoning", "text": "再决定下一步。" }
        ]);
        let assistant_message = json!({
            "role": "assistant",
            "content": "先读取文件，再给出结论。",
            "reasoning_content": structured_reasoning.clone(),
            "tool_calls": [
                {
                    "id": "call_structured",
                    "type": "function",
                    "function": {
                        "name": "workspace_read_file",
                        "arguments": "{\"path\":\"src-tauri/src/agent/provider.rs\"}"
                    }
                }
            ]
        });

        let messages = openai_messages_with_tool_result(
            &request,
            &mut vec![],
            Some(&assistant_message),
            &tool_call,
            &tool_result,
        );

        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages[1].get("reasoning_content"),
            Some(&structured_reasoning)
        );
    }

    #[test]
    fn anthropic_tool_followup_replays_reasoning_content_as_thinking_blocks() {
        let tool_call = ToolCall {
            call_id: Some("toolu_reasoning".to_string()),
            name: "List".to_string(),
            arguments: json!({ "path": "." }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "List".to_string(),
            status: "ok".to_string(),
            output: "README.md\nsrc".to_string(),
            duration_ms: 0,
        };
        let request = ProviderRequest {
            model: "deepseek-v4-flash".to_string(),
            input: vec![ProviderMessage {
                role: ProviderRole::User,
                content: "当前文件夹下有哪些文件？".to_string(),
            }],
            images: vec![],
            native_messages: vec![],
            observation: ProviderRequestObservation::default(),
            temperature: 0.0,
            max_output_tokens: 1024,
        };
        let assistant_message = json!({
            "role": "assistant",
            "content": "我先列出当前目录。",
            "reasoning_content": [
                {
                    "type": "thinking",
                    "thinking": "先调用目录列表工具。"
                }
            ]
        });

        let messages = anthropic_messages_with_tool_result(
            &request,
            &mut Vec::new(),
            Some(&assistant_message),
            &tool_call,
            &tool_result,
        );

        let assistant_content = messages[1]
            .get("content")
            .and_then(Value::as_array)
            .expect("assistant content should be blocks");
        assert_eq!(
            assistant_content[0].get("type").and_then(Value::as_str),
            Some("thinking")
        );
        assert_eq!(
            assistant_content[0].get("thinking").and_then(Value::as_str),
            Some("先调用目录列表工具。")
        );
        assert!(assistant_content
            .iter()
            .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_use")));
    }

    #[test]
    fn anthropic_tool_followup_accumulates_prior_tool_results_across_hops() {
        let request = ProviderRequest {
            model: "claude-sonnet-5".to_string(),
            input: vec![ProviderMessage::user("当前文件夹下有哪些文件？")],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.0,
            max_output_tokens: 1024,
        };
        let list_call = ToolCall {
            call_id: Some("toolu_list".to_string()),
            name: "List".to_string(),
            arguments: json!({ "path": ".", "description": "列出当前目录" }),
            plan: None,
        };
        let read_call = ToolCall {
            call_id: Some("toolu_read".to_string()),
            name: "Read".to_string(),
            arguments: json!({ "path": "Cargo.toml", "description": "读取项目配置" }),
            plan: None,
        };
        let list_message = json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "toolu_list",
                "name": "List",
                "input": list_call.arguments
            }]
        });
        let read_message = json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "toolu_read",
                "name": "Read",
                "input": read_call.arguments
            }]
        });
        let list_result = ToolResult {
            tool_name: "List".to_string(),
            status: "ok".to_string(),
            output: "Cargo.toml\nsrc".to_string(),
            duration_ms: 1,
        };
        let read_result = ToolResult {
            tool_name: "Read".to_string(),
            status: "ok".to_string(),
            output: "[workspace]\nmembers = []".to_string(),
            duration_ms: 1,
        };
        let mut accumulated = Vec::new();

        let first = anthropic_messages_with_tool_result(
            &request,
            &mut accumulated,
            Some(&list_message),
            &list_call,
            &list_result,
        );
        let second = anthropic_messages_with_tool_result(
            &request,
            &mut accumulated,
            Some(&read_message),
            &read_call,
            &read_result,
        );

        assert_eq!(first.len(), 3);
        assert_eq!(second.len(), 5);
        assert_eq!(second[1]["content"][0]["id"].as_str(), Some("toolu_list"));
        assert_eq!(
            second[2]["content"][0]["tool_use_id"].as_str(),
            Some("toolu_list")
        );
        assert_eq!(second[3]["content"][0]["id"].as_str(), Some("toolu_read"));
        assert_eq!(
            second[4]["content"][0]["tool_use_id"].as_str(),
            Some("toolu_read")
        );
    }

    #[test]
    fn anthropic_live_followup_sends_tool_use_and_result_in_one_conversation() {
        let runtime = tokio::runtime::Runtime::new().expect("create Tokio runtime");
        let _guard = runtime.enter();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock Anthropic server");
        let address = listener.local_addr().expect("read mock server address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_for_thread = Arc::clone(&requests);
        let responses = vec![
            json!({
                "content": [{
                    "type": "tool_use",
                    "id": "toolu_list",
                    "name": "List",
                    "input": { "path": ".", "description": "列出当前目录" }
                }],
                "usage": { "input_tokens": 1, "output_tokens": 1 }
            }),
            json!({
                "content": [{ "type": "text", "text": "Cargo.toml 和 src。" }],
                "usage": { "input_tokens": 2, "output_tokens": 2 }
            }),
        ];
        let handle = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().expect("accept mock request");
                let mut bytes = Vec::new();
                let mut chunk = [0_u8; 4096];
                let mut header_end = None;
                let mut content_length = 0usize;
                loop {
                    let read = stream.read(&mut chunk).expect("read mock request");
                    if read == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..read]);
                    if header_end.is_none() {
                        header_end = bytes
                            .windows(4)
                            .position(|window| window == b"\r\n\r\n")
                            .map(|index| index + 4);
                        if let Some(end) = header_end {
                            let headers = String::from_utf8_lossy(&bytes[..end]);
                            content_length = headers
                                .lines()
                                .find_map(|line| {
                                    let (name, value) = line.split_once(':')?;
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().ok())
                                        .flatten()
                                })
                                .expect("content length");
                        }
                    }
                    if let Some(end) = header_end {
                        if bytes.len() >= end + content_length {
                            break;
                        }
                    }
                }
                let end = header_end.expect("HTTP header end");
                requests_for_thread
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&bytes[end..end + content_length]).to_string());
                let body = response.to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
                stream.flush().expect("flush response");
            }
        });
        let config = ResolvedProviderSelection {
            requested_name: "anthropic-test".to_string(),
            provider_name: "anthropic-test".to_string(),
            protocol: ProviderProtocol::AnthropicMessages,
            base_url: format!("http://{}", address),
            auth_type: ProviderAuthType::XApiKey,
            api_key_env_var: "ANTHROPIC_API_KEY".to_string(),
            api_key: Some("test".to_string()),
            model: "claude-sonnet-5".to_string(),
            temperature: 0.0,
            max_output_tokens: 1024,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                supports_tools: true,
                supports_streaming: false,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::None,
        };
        let manager = ProviderManager::new(config);
        let request = ProviderRequest {
            model: "claude-sonnet-5".to_string(),
            input: vec![ProviderMessage::user("当前文件夹下有哪些文件？")],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.0,
            max_output_tokens: 1024,
        };
        let tools = vec![ToolDefinition {
            name: "List",
            description: "列出当前目录。",
            input_schema: json!({ "type": "object", "properties": {} }),
        }];

        let decision = manager
            .decide_with_tools(&request, &tools)
            .expect("first tool decision");
        let tool_call = decision.tool_call.expect("List tool call");
        let response = manager
            .continue_with_tool_result(
                &request,
                &tools,
                &mut Vec::new(),
                decision.assistant_message.as_ref(),
                &tool_call,
                &ToolResult {
                    tool_name: "List".to_string(),
                    status: "ok".to_string(),
                    output: "Cargo.toml\nsrc".to_string(),
                    duration_ms: 1,
                },
            )
            .expect("follow-up response");
        handle.join().expect("join mock server");

        let requests = requests.lock().unwrap();
        let followup_body: Value = serde_json::from_str(&requests[1]).expect("follow-up JSON");
        let messages = followup_body["messages"]
            .as_array()
            .expect("follow-up messages");
        assert_eq!(response.output_text, "Cargo.toml 和 src。");
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"].as_str(), Some("user"));
        assert_eq!(messages[1]["role"].as_str(), Some("assistant"));
        assert_eq!(messages[1]["content"][0]["type"].as_str(), Some("tool_use"));
        assert_eq!(messages[1]["content"][0]["id"].as_str(), Some("toolu_list"));
        assert_eq!(messages[2]["role"].as_str(), Some("user"));
        assert_eq!(
            messages[2]["content"][0]["tool_use_id"].as_str(),
            Some("toolu_list")
        );
        assert_eq!(
            messages[2]["content"][0]["content"][0]["text"].as_str(),
            Some("Cargo.toml\nsrc")
        );
    }

    #[test]
    fn openai_tool_followup_filters_unexecuted_parallel_tool_calls() {
        let request = ProviderRequest {
            model: "gpt-5.4".to_string(),
            input: vec![ProviderMessage::user("检查 capabilities 目录")],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };
        let tool_call = ToolCall {
            call_id: Some("call_path_info".to_string()),
            name: "workspace_path_info".to_string(),
            arguments: json!({ "path": "capabilities" }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "workspace_path_info".to_string(),
            status: "ok".to_string(),
            output: "{\"kind\":\"directory\"}".to_string(),
            duration_ms: 10,
        };
        let assistant_message = json!({
            "role": "assistant",
            "content": "",
            "reasoning_content": "需要查看目录信息。",
            "tool_calls": [
                {
                    "id": "call_path_info",
                    "type": "function",
                    "function": {
                        "name": "workspace_path_info",
                        "arguments": "{\"path\":\"capabilities\"}"
                    }
                },
                {
                    "id": "call_list_files",
                    "type": "function",
                    "function": {
                        "name": "workspace_list_files",
                        "arguments": "{\"path\":\"capabilities\"}"
                    }
                }
            ]
        });

        let messages = openai_messages_with_tool_result(
            &request,
            &mut vec![],
            Some(&assistant_message),
            &tool_call,
            &tool_result,
        );

        let assistant_tool_calls = messages[1]
            .get("tool_calls")
            .and_then(Value::as_array)
            .expect("assistant tool calls");
        assert_eq!(assistant_tool_calls.len(), 1);
        assert_eq!(
            assistant_tool_calls[0].get("id").and_then(Value::as_str),
            Some("call_path_info")
        );
        assert_eq!(
            messages[2].get("tool_call_id").and_then(Value::as_str),
            Some("call_path_info")
        );
        assert!(!serde_json::to_string(&messages)
            .expect("messages serialize")
            .contains("call_list_files"));
    }

    #[test]
    fn openai_tool_followup_fallback_rebuild_keeps_structured_reasoning_content() {
        let tool_call = ToolCall {
            call_id: Some("call_repaired".to_string()),
            name: "workspace_read_file".to_string(),
            arguments: json!({ "path": "Cargo.toml" }),
            plan: None,
        };
        let structured_reasoning = json!([
            { "type": "reasoning", "text": "先修正工具调用，再继续。" }
        ]);
        let assistant_message = json!({
            "role": "assistant",
            "content": "继续读取文件",
            "reasoning_content": structured_reasoning.clone(),
            "tool_calls": [
                {
                    "id": "call_other",
                    "type": "function",
                    "function": {
                        "name": "workspace_list_files",
                        "arguments": "{\"path\":\".\"}"
                    }
                }
            ]
        });

        let rebuilt =
            openai_assistant_message_for_tool_result(Some(&assistant_message), &tool_call);

        assert_eq!(
            rebuilt.get("reasoning_content"),
            Some(&structured_reasoning)
        );
        assert_eq!(
            rebuilt
                .get("tool_calls")
                .and_then(Value::as_array)
                .and_then(|calls| calls.first())
                .and_then(|call| call.get("function"))
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str),
            Some("workspace_read_file")
        );
    }

    #[test]
    fn openai_tool_followup_keeps_small_tool_output_raw() {
        let tool_call = ToolCall {
            call_id: Some("call_small".to_string()),
            name: "workspace.read_file".to_string(),
            arguments: json!({ "path": "src/main.rs" }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "workspace.read_file".to_string(),
            status: "ok".to_string(),
            output: "small output".to_string(),
            duration_ms: 3,
        };

        let message = openai_followup_tool_result_message(&tool_call, &tool_result);

        assert_eq!(
            message.get("content").and_then(Value::as_str),
            Some("small output")
        );
    }

    /// DeepSeek thinking 模式：本地 planner 决策（assistant_message=None）的
    /// follow-up 必须回传非空 reasoning_content，否则 API 返回 400。
    #[test]
    fn openai_tool_followup_without_provider_assistant_message_still_echoes_reasoning() {
        let tool_call = ToolCall {
            call_id: Some("call_planner_batch".to_string()),
            name: "workspace_batch".to_string(),
            arguments: json!({
                "calls": [
                    { "name": "workspace_gather_context", "arguments": { "path": "src" } }
                ]
            }),
            plan: None,
        };

        let rebuilt = openai_assistant_message_for_tool_result(None, &tool_call);

        assert_eq!(
            rebuilt.get("role").and_then(Value::as_str),
            Some("assistant")
        );
        let reasoning = rebuilt
            .get("reasoning_content")
            .and_then(Value::as_str)
            .expect("planner follow-up must echo non-empty reasoning_content");
        assert!(!reasoning.trim().is_empty());
        assert!(rebuilt.get("tool_calls").is_some());
    }

    /// DeepSeek thinking 模式：assistant 消息带 tool_calls 但 reasoning_content
    /// 缺失或为空时，normalize 必须兜底填充非空占位文本。
    #[test]
    fn openai_normalize_fills_reasoning_placeholder_for_tool_call_message_without_reasoning() {
        let message = json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [
                {
                    "id": "call_x",
                    "type": "function",
                    "function": {
                        "name": "WebSearch",
                        "arguments": "{\"query\":\"migration\"}"
                    }
                }
            ]
        });

        let normalized = normalize_openai_assistant_message(&message);

        let reasoning = normalized
            .get("reasoning_content")
            .and_then(Value::as_str)
            .expect("tool-call assistant message must carry non-empty reasoning_content");
        assert!(!reasoning.trim().is_empty());
        assert_eq!(reasoning, TOOL_CALL_REASONING_PLACEHOLDER);
    }

    #[test]
    fn openai_tool_followup_truncates_large_tool_output() {
        let tool_call = ToolCall {
            call_id: Some("call_large".to_string()),
            name: "workspace.gather_context".to_string(),
            arguments: json!({ "paths": ["src"] }),
            plan: None,
        };
        let large_output = format!(
            "{{\"files\":\"{}\",\"tail\":\"{}\"}}",
            "a".repeat(PruneBudget::PROVIDER_FOLLOWUP.threshold_chars),
            "z".repeat(4_000)
        );
        let tool_result = ToolResult {
            tool_name: "workspace.gather_context".to_string(),
            status: "ok".to_string(),
            output: large_output.clone(),
            duration_ms: 42,
        };

        let message = openai_followup_tool_result_message(&tool_call, &tool_result);
        let content = message
            .get("content")
            .and_then(Value::as_str)
            .expect("content string");
        let payload: Value = serde_json::from_str(content).expect("compressed json payload");

        assert!(content.chars().count() < large_output.chars().count());
        assert_eq!(
            payload
                .get("compression")
                .and_then(|value| value.get("applied"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            payload
                .get("summary")
                .and_then(|value| value.get("kind"))
                .and_then(Value::as_str),
            Some("json_object")
        );
        assert_eq!(
            payload
                .get("summary")
                .and_then(|value| value.get("keys"))
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(2)
        );
        assert!(payload.get("head").and_then(Value::as_str).is_some());
        assert!(payload.get("tail").and_then(Value::as_str).is_some());
    }

    #[test]
    fn openai_sse_reader_streams_reasoning_and_text_in_order() {
        let raw_text = concat!(
            ": keep-alive\n\n",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"先看工具输出。\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"第一段\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"第二段\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut deltas = Vec::new();

        let message = collect_openai_sse_message_from_reader(
            std::io::Cursor::new(raw_text.as_bytes()),
            "unit-test",
            &mut |delta| deltas.push(delta),
        )
        .expect("stream message should parse");

        assert_eq!(message.output_text, "第一段第二段");
        assert_eq!(message.reasoning_content.as_deref(), Some("先看工具输出。"));
        assert_eq!(
            deltas,
            vec![
                ProviderStreamChunk::Reasoning("先看工具输出。".to_string()),
                ProviderStreamChunk::Text("第一段".to_string()),
                ProviderStreamChunk::Text("第二段".to_string())
            ]
        );
    }

    #[test]
    fn openai_sse_string_collector_still_parses_full_payload() {
        let raw_text = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"alpha\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"beta\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut deltas = Vec::new();

        let message = collect_openai_sse_message(raw_text, &mut |delta| deltas.push(delta))
            .expect("string helper should parse");

        assert_eq!(message.output_text, "alphabeta");
        assert_eq!(message.reasoning_content, None);
        assert_eq!(
            deltas,
            vec![
                ProviderStreamChunk::Text("alpha".to_string()),
                ProviderStreamChunk::Text("beta".to_string())
            ]
        );
    }

    #[test]
    fn openai_sse_reader_extracts_usage_from_terminal_chunk() {
        let raw_text = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"alpha\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":120,\"prompt_cache_hit_tokens\":48,\"completion_tokens\":32,\"total_tokens\":152}}\n\n",
            "data: [DONE]\n\n"
        );

        let message = collect_openai_sse_message_from_reader(
            std::io::Cursor::new(raw_text.as_bytes()),
            "unit-test",
            &mut |_delta| {},
        )
        .expect("stream message should parse");

        let usage = message.token_usage.expect("usage should be captured");
        assert_eq!(usage.input_tokens, Some(120));
        assert_eq!(usage.cache_hit_input_tokens, Some(48));
        assert_eq!(usage.output_tokens, Some(32));
        assert_eq!(usage.total_tokens, Some(152));
    }

    #[test]
    fn openai_reasoning_tool_followup_stream_attempts_live_stream_before_fallback() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let config = ResolvedProviderSelection {
            requested_name: "ppx".to_string(),
            provider_name: "ppx".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: "http://127.0.0.1:1/v1".to_string(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "PPX_API_KEY".to_string(),
            api_key: Some("test".to_string()),
            model: "gpt-5.4".to_string(),
            temperature: 0.2,
            max_output_tokens: 8192,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(200_000),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: true,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::EffortStandard,
        };
        let manager = ProviderManager::new(config);
        let request = ProviderRequest {
            model: "gpt-5.4".to_string(),
            input: vec![ProviderMessage::user("继续总结工具结果")],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };
        let tool_call = ToolCall {
            call_id: Some("call_followup".to_string()),
            name: "workspace.gather_context".to_string(),
            arguments: json!({ "paths": ["src-tauri/src/agent/provider.rs"] }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "workspace.gather_context".to_string(),
            status: "ok".to_string(),
            output: "{\"summary\":{\"text\":\"已读取 provider follow-up 相关实现\"}}".to_string(),
            duration_ms: 18,
        };
        let mut deltas = Vec::new();

        let response = manager
            .continue_with_tool_result_stream(
                &request,
                &[ToolDefinition {
                    name: "workspace.gather_context",
                    description: "gather context",
                    input_schema: json!({ "type": "object" }),
                }],
                &mut vec![],
                None,
                &tool_call,
                &tool_result,
                |delta| deltas.push(delta),
            )
            .expect("follow-up should degrade after stream failure");

        assert_eq!(response.provider_source, "provider_followup_local_fallback");
        assert_eq!(response.provider_mode, "fallback");
        assert!(response
            .fallback_reason
            .as_deref()
            .unwrap_or_default()
            .contains("provider_followup_failed"));
        assert!(deltas.is_empty());
    }

    #[test]
    fn provider_timeout_classifier_matches_timeout_errors() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        assert!(policy
            .classify("调用 provider 失败：operation timed out；type=timeout；elapsed=45000ms")
            .is_retryable());
        assert!(!policy.classify("provider 返回错误状态：500").is_retryable());
    }

    #[test]
    fn retry_provider_timeout_stops_after_non_timeout_error() {
        let attempts = std::cell::Cell::new(0);
        let result: Result<(), String> = retry_provider_timeout("decision", || {
            attempts.set(attempts.get() + 1);
            Err("provider returned 500".to_string())
        });

        assert!(result.is_err());
        assert_eq!(attempts.get(), 1);
    }

    #[test]
    fn retry_provider_timeout_retries_until_success() {
        let attempts = std::cell::Cell::new(0);
        let result = retry_provider_timeout("decision", || {
            let next = attempts.get() + 1;
            attempts.set(next);
            if next < 3 {
                Err("调用 provider 失败：operation timed out；type=timeout".to_string())
            } else {
                Ok("ok")
            }
        })
        .expect("timeout retry should eventually succeed");

        assert_eq!(result, "ok");
        assert_eq!(attempts.get(), 3);
    }

    // ── PA-100：rate-limit 感知的 followup 长退避 ───────────────────────────────────────────

    #[test]
    fn is_rate_limit_error_matches_provider_wording() {
        // PA-143：命中必须来自**措辞**（tpm），而不是 JSON 错误码里的数字子串。
        assert!(is_rate_limit_error(
            "{\"error\":{\"message\":\"inference tpm exhausted\",\"code\":\"1001\"}}"
        ));
        // PA-143：`"code":"429001"` 只是更长数字串内的 `429` 片段，不构成限流契约
        // （旧断言把该子串命中当契约，正是缺陷所在）。
        assert!(!is_rate_limit_error(
            "{\"error\":{\"message\":\"invalid request\",\"code\":\"429001\"}}"
        ));
        assert!(is_rate_limit_error("HTTP 429 Too Many Requests"));
        assert!(is_rate_limit_error("rate limit exceeded for org"));
        assert!(is_rate_limit_error("monthly quota exhausted"));
        assert!(!is_rate_limit_error("provider returned 500"));
        assert!(!is_rate_limit_error("operation timed out"));
    }

    #[test]
    fn followup_rate_limit_switches_to_long_backoff_schedule() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let (result, report): (Result<(), String>, SingleHopRetryReport) =
            retry_provider_scoped_with_report(
                "followup_sync",
                ProviderRetryScope::Followup,
                || {
                    attempts.set(attempts.get() + 1);
                    Err(
                        "{\"error\":{\"message\":\"inference tpm exhausted\",\"code\":\"429001\"}}"
                            .to_string(),
                    )
                },
                &sleeper,
                None,
            );

        assert!(result.is_err());
        // 切换后调度：sleep 20s → 重试 → sleep 40s → 重试 → 预算耗尽。
        assert_eq!(attempts.get(), 3);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            60_000
        );
        // T2-B：单跳报告口径（attempts=operation 调用次数；slept=sleep-only 累计）。
        assert_eq!(report.attempts_total, 3);
        assert_eq!(report.slept_ms, 60_000);
        assert!(report.switched_to_rl);
        assert_eq!(report.scope, "followup");
        assert_eq!(report.exhausted_reason, Some("attempt_limit_exhausted"));
    }

    #[test]
    fn decision_scope_aborts_immediately_on_rate_limit_errors() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let (result, report): (Result<(), String>, SingleHopRetryReport) =
            retry_provider_scoped_with_report(
                "decision",
                ProviderRetryScope::Decision,
                || {
                    attempts.set(attempts.get() + 1);
                    Err("HTTP 429 too many requests".to_string())
                },
                &sleeper,
                None,
            );

        assert!(result.is_err());
        // PA-171 C3-P2（反转 PA-100 的「首跳短退避」）：限流/配额首跳即 Abort——
        // 只 1 次尝试、0 次 sleep（冷却窗口由 provider HTTP 出口登记，不再白烧 7.5s）。
        assert_eq!(attempts.get(), 1);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        // T2-B：Decision 作用域从不切换 RL 配置。
        assert_eq!(report.attempts_total, 1);
        assert_eq!(report.slept_ms, 0);
        assert!(!report.switched_to_rl);
        assert_eq!(report.scope, "decision");
    }

    /// PA-171 C3-P6 反向钉子：Decision 作用域的非限流瞬时错误（503）必须保留既有重试。
    #[test]
    fn decision_scope_keeps_retries_for_non_rate_limit_transient_errors() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let result: Result<(), String> = retry_provider_scoped_with_sleeper(
            "decision",
            ProviderRetryScope::Decision,
            || {
                attempts.set(attempts.get() + 1);
                Err("provider 返回错误状态：503 Service Unavailable".to_string())
            },
            &sleeper,
            None,
        );

        assert!(result.is_err());
        // 503 不是限流 ⇒ 走默认短退避曲线。2026-11-14 默认尝试次数 5 → 10（含首次），
        // 退避 sleep = 0.5+1+2+4+8×5 = 47.5s（默认预算 60s 装得下）。
        assert_eq!(attempts.get(), 10);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            47_500
        );
    }

    #[path = "retry_status_tests.rs"]
    mod retry_status_tests;

    #[test]
    fn followup_timeout_failures_do_not_switch_to_rate_limit_schedule() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let result: Result<(), String> = retry_provider_scoped_with_sleeper(
            "followup_sync",
            ProviderRetryScope::Followup,
            || {
                attempts.set(attempts.get() + 1);
                Err("调用 provider 失败：operation timed out；type=timeout".to_string())
            },
            &sleeper,
            None,
        );

        assert!(result.is_err());
        // 2026-11-14：默认尝试次数 5 → 10（含首次）⇒ 10 次调用、退避合计 47.5s。
        assert_eq!(attempts.get(), 10);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            47_500
        );
    }

    /// 裁决记录（PA-100 审核）：报文同时含 timeout 与 rate-limit 特征时按
    /// rate-limit 处理——长退避对两类瞬时故障都安全，反向必然撞限流窗口。
    #[test]
    fn mixed_timeout_and_rate_limit_message_takes_rate_limit_schedule_in_followup() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let result: Result<(), String> = retry_provider_scoped_with_sleeper(
            "followup_stream",
            ProviderRetryScope::Followup,
            || {
                attempts.set(attempts.get() + 1);
                Err(
                    "request to upstream timed out; upstream said: 429 inference tpm exhausted"
                        .to_string(),
                )
            },
            &sleeper,
            None,
        );

        assert!(result.is_err());
        assert_eq!(attempts.get(), 3);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            60_000
        );
    }

    /// PA-100 审核 A 缺失测试①：RL 切换发生在 base 阶段已消耗部分时间预算之后，
    /// 结转的 elapsed 必须计入 RL budget（时间口径跨配置累计）。
    #[test]
    fn followup_rate_limit_switch_carries_elapsed_budget_across_configs() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let result: Result<(), String> = retry_provider_scoped_with_sleeper(
            "followup_sync",
            ProviderRetryScope::Followup,
            || {
                let next = attempts.get() + 1;
                attempts.set(next);
                if next <= 2 {
                    Err("调用 provider 失败：operation timed out；type=timeout".to_string())
                } else {
                    Err(
                        "{\"error\":{\"message\":\"inference tpm exhausted\",\"code\":\"429001\"}}"
                            .to_string(),
                    )
                }
            },
            &sleeper,
            None,
        );

        assert!(result.is_err());
        // 序列：t/o(无睡) → t/o(sleep0.5s) → 429 切换 → sleep20s → 429 → sleep40s → 429 耗尽。
        assert_eq!(attempts.get(), 5);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            61_500
        );
    }

    /// PA-100 code-review B P1-2：流式增量已发出后，重试必须在 sleep 前放弃，
    /// 且不得切换 RL 长退避（否则白睡 60s 后仍必然失败）。
    #[test]
    fn followup_stream_hint_stops_retries_without_rate_limit_backoff() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let streamed = std::sync::atomic::AtomicBool::new(false);
        let attempts = std::cell::Cell::new(0);
        let result: Result<(), String> = retry_provider_scoped_with_sleeper(
            "followup_stream",
            ProviderRetryScope::Followup,
            || {
                let next = attempts.get() + 1;
                attempts.set(next);
                if next == 1 {
                    streamed.store(true, std::sync::atomic::Ordering::Relaxed);
                    Err(
                        "{\"error\":{\"message\":\"inference tpm exhausted\",\"code\":\"429001\"}}"
                            .to_string(),
                    )
                } else {
                    Err("stream already emitted delta; auto-retry forbidden".to_string())
                }
            },
            &sleeper,
            Some(&|| streamed.load(std::sync::atomic::Ordering::Relaxed)),
        );

        assert!(result.is_err());
        // hint 阻断切换与 sleep：立即返回首跳错误进 sync fallback。
        assert_eq!(attempts.get(), 1);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    /// T2-B：quota-only 报文不触发 RL 长退避——`classify` 不含 quota 特征，
    /// 直接 Abort（安全方向：不在永久配额耗尽上烧退避）。
    #[test]
    fn single_hop_report_quota_only_aborts_without_retry() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let (result, report): (Result<(), String>, SingleHopRetryReport) =
            retry_provider_scoped_with_report(
                "followup_sync",
                ProviderRetryScope::Followup,
                || {
                    attempts.set(attempts.get() + 1);
                    Err("monthly quota exhausted".to_string())
                },
                &sleeper,
                None,
            );

        assert!(result.is_err());
        assert_eq!(attempts.get(), 1);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(report.attempts_total, 1);
        assert_eq!(report.slept_ms, 0);
        assert!(!report.switched_to_rl);
        assert_eq!(report.scope, "followup");
        assert_eq!(report.exhausted_reason, None);
    }

    /// T2-B F2：quota+429 混合走 RL 长退避——报文同时含 quota 与 429 特征，
    /// `classify` 先判 TransientRetryable，再由 `is_rate_limit_error` 命中切 RL。
    /// 与 quota-only Abort 测试并列，锁住混合语义（残余：永久配额场景白烧 ~60s）。
    #[test]
    fn single_hop_report_quota_and_429_mixed_follows_rate_limit_schedule() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let (result, report): (Result<(), String>, SingleHopRetryReport) =
            retry_provider_scoped_with_report(
                "followup_sync",
                ProviderRetryScope::Followup,
                || {
                    attempts.set(attempts.get() + 1);
                    Err("HTTP 429 quota exceeded for org".to_string())
                },
                &sleeper,
                None,
            );

        assert!(result.is_err());
        // RL 调度：sleep 20s → 重试 → sleep 40s → 重试 → 预算耗尽。
        assert_eq!(attempts.get(), 3);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            60_000
        );
        assert_eq!(report.attempts_total, 3);
        assert_eq!(report.slept_ms, 60_000);
        assert!(report.switched_to_rl);
        assert_eq!(report.scope, "followup");
        assert_eq!(report.exhausted_reason, Some("attempt_limit_exhausted"));
    }

    /// T2-B：首跳 429 切 RL（sleep 20s），第二跳前 hint 变真则在 sleep 前放弃——
    /// attempts==2、sleep==20_000、switched==true。
    #[test]
    fn single_hop_report_rl_then_hint_aborts_before_second_sleep() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let hint = std::sync::atomic::AtomicBool::new(false);
        let attempts = std::cell::Cell::new(0);
        let (result, report): (Result<(), String>, SingleHopRetryReport) =
            retry_provider_scoped_with_report(
                "followup_stream",
                ProviderRetryScope::Followup,
                || {
                    let next = attempts.get() + 1;
                    attempts.set(next);
                    if next == 2 {
                        // 第二跳已进入 RL 调度（首跳后 sleep 20s），此时增量发出，
                        // hint 变真 → 循环顶在第二次 sleep 前放弃。
                        hint.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    Err(
                        "{\"error\":{\"message\":\"inference tpm exhausted\",\"code\":\"429001\"}}"
                            .to_string(),
                    )
                },
                &sleeper,
                Some(&|| hint.load(std::sync::atomic::Ordering::Relaxed)),
            );

        assert!(result.is_err());
        assert_eq!(attempts.get(), 2);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            20_000
        );
        assert_eq!(report.attempts_total, 2);
        assert_eq!(report.slept_ms, 20_000);
        assert!(report.switched_to_rl);
        assert_eq!(report.scope, "followup");
        assert_eq!(report.exhausted_reason, None);
    }

    /// T2-B F4：首试成功——覆盖 report 成功分支 `attempts_total + 1` 口径
    /// （attempts==1、slept==0、switched==false、exhausted==None）。
    #[test]
    fn single_hop_report_first_attempt_success() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let (result, report): (Result<&str, String>, SingleHopRetryReport) =
            retry_provider_scoped_with_report(
                "decision",
                ProviderRetryScope::Decision,
                || {
                    attempts.set(attempts.get() + 1);
                    Ok("ok")
                },
                &sleeper,
                None,
            );

        assert_eq!(result.expect("first attempt should succeed"), "ok");
        assert_eq!(attempts.get(), 1);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(report.attempts_total, 1);
        assert_eq!(report.slept_ms, 0);
        assert!(!report.switched_to_rl);
        assert_eq!(report.scope, "decision");
        assert_eq!(report.exhausted_reason, None);
    }

    /// T2-B F4：重试后成功——timeout 一次后成功（attempts==2、slept==500、scope 正确）。
    #[test]
    fn single_hop_report_success_after_timeout_retry() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let (result, report): (Result<&str, String>, SingleHopRetryReport) =
            retry_provider_scoped_with_report(
                "decision",
                ProviderRetryScope::Decision,
                || {
                    let next = attempts.get() + 1;
                    attempts.set(next);
                    if next < 2 {
                        Err("调用 provider 失败：operation timed out；type=timeout".to_string())
                    } else {
                        Ok("ok")
                    }
                },
                &sleeper,
                None,
            );

        assert_eq!(result.expect("retry should eventually succeed"), "ok");
        assert_eq!(attempts.get(), 2);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            500
        );
        assert_eq!(report.attempts_total, 2);
        assert_eq!(report.slept_ms, 500);
        assert!(!report.switched_to_rl);
        assert_eq!(report.scope, "decision");
        assert_eq!(report.exhausted_reason, None);
    }

    /// 2026-11-12：HTTP **525** 由「码表不命中 ⇒ 立即 Abort」改为 ③′ 边缘 5xx 同组。
    /// 本用例锁的是**行为**而非码表归属：`retry_provider_scoped_with_report` 必须真的
    /// sleep 500ms 后再试一次（不是只看 `classify().is_retryable()`）。
    /// 实证来源：会话 `pa_01a1cf14` 的 `decision_stream:attempt=1/5 … 525 … Abort`
    /// 与 `followup_stream` / `followup_sync_batch` 同款行——两条 scope 都要覆盖。
    #[test]
    fn http_525_enters_retry_loop_and_succeeds_on_second_attempt() {
        for (label, scope) in [
            ("decision_stream", ProviderRetryScope::Decision),
            ("followup_stream", ProviderRetryScope::Followup),
        ] {
            let sleeper = crate::agent::retry::FakeSleeper::new();
            let attempts = std::cell::Cell::new(0);
            let (result, report): (Result<&str, String>, SingleHopRetryReport) =
                retry_provider_scoped_with_report(
                    label,
                    scope,
                    || {
                        let next = attempts.get() + 1;
                        attempts.set(next);
                        if next < 2 {
                            Err(
                                "provider 返回错误状态：525 <unknown status code>；耗时=6097ms；响应正文："
                                    .to_string(),
                            )
                        } else {
                            Ok("ok")
                        }
                    },
                    &sleeper,
                    None,
                );

            assert_eq!(
                result.unwrap_or_else(|e| panic!("{label}: 525 必须可重试，实际 {e}")),
                "ok"
            );
            assert_eq!(attempts.get(), 2, "{label}: 525 必须真的发起第二次请求");
            assert_eq!(
                sleeper
                    .total_slept
                    .load(std::sync::atomic::Ordering::Relaxed),
                500,
                "{label}: 首次退避应为 initial_delay_ms=500"
            );
            assert_eq!(report.attempts_total, 2);
            assert_eq!(report.slept_ms, 500);
            assert!(!report.switched_to_rl, "{label}: 525 不是限流，不得切长退避");
            assert_eq!(report.exhausted_reason, None);
        }
    }

    /// 与上一条互为反向钉子：**526** `Invalid SSL Certificate` 是确定性配置错误，
    /// 必须首跳 Abort——attempts==1、slept==0。两条并列才能证明 525/526 的拆分真的落地。
    #[test]
    fn http_526_aborts_without_retry() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let (result, report): (Result<(), String>, SingleHopRetryReport) =
            retry_provider_scoped_with_report(
                "followup_stream",
                ProviderRetryScope::Followup,
                || {
                    attempts.set(attempts.get() + 1);
                    Err(
                        "provider 返回错误状态：526 <unknown status code>；耗时=6097ms；响应正文："
                            .to_string(),
                    )
                },
                &sleeper,
                None,
            );

        assert!(result.is_err());
        assert_eq!(attempts.get(), 1, "526 必须首跳即 Abort");
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(report.attempts_total, 1);
        assert_eq!(report.slept_ms, 0);
        assert_eq!(report.exhausted_reason, None);
    }

    #[test]
    fn openai_sse_reader_handles_keepalive_and_comment_lines() {
        let raw_text = concat!(
            ": keep-alive\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"alpha\"}}]}\n\n",
            ": some comment\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"beta\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut deltas = Vec::new();

        let message = collect_openai_sse_message_from_reader(
            std::io::Cursor::new(raw_text.as_bytes()),
            "unit-test",
            &mut |delta| deltas.push(delta),
        )
        .expect("stream with comments should parse");

        assert_eq!(message.output_text, "alphabeta");
        assert_eq!(
            deltas,
            vec![
                ProviderStreamChunk::Text("alpha".to_string()),
                ProviderStreamChunk::Text("beta".to_string()),
            ]
        );
    }

    /// DeepSeek 思考模式会逐片流式返回 reasoning_content。
    /// follow-up 请求必须把完整思考文本回传给 API（否则 DeepSeek 返回
    /// "The `reasoning_content` in the thinking mode must be passed back to the API."），
    /// 因此 reasoning_content_value 不能只保留首个分片。
    #[test]
    fn openai_stream_reasoning_value_echoes_full_reasoning_text() {
        let raw_text = concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"第一步\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"检查文件结构\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"。\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"结论：已了解。\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut deltas = Vec::new();

        let message = collect_openai_sse_message_from_reader(
            std::io::Cursor::new(raw_text.as_bytes()),
            "unit-test",
            &mut |delta| deltas.push(delta),
        )
        .expect("stream with reasoning chunks should parse");

        assert_eq!(
            message.reasoning_content.as_deref(),
            Some("第一步检查文件结构。")
        );
        assert_eq!(
            message
                .reasoning_content_value
                .as_ref()
                .and_then(Value::as_str),
            Some("第一步检查文件结构。")
        );
        assert_eq!(
            deltas,
            vec![
                ProviderStreamChunk::Reasoning("第一步".to_string()),
                ProviderStreamChunk::Reasoning("检查文件结构".to_string()),
                ProviderStreamChunk::Reasoning("。".to_string()),
                ProviderStreamChunk::Text("结论：已了解。".to_string()),
            ]
        );
    }

    /// 流式决策后发起 follow-up 时，assistant 消息中的 reasoning_content
    /// 必须携带完整思考文本（DeepSeek thinking 模式的回传校验）。
    #[test]
    fn openai_followup_message_carries_full_reasoning_content() {
        let request = ProviderRequest {
            model: "deepseek-v4-flash".to_string(),
            input: vec![ProviderMessage::user("批量读取文件")],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.0,
            max_output_tokens: 1024,
        };
        let tool_call = ToolCall {
            call_id: Some("call_batch".to_string()),
            name: "workspace_batch".to_string(),
            arguments: json!({
                "calls": [
                    { "name": "workspace_gather_context", "arguments": { "path": "1.0" } }
                ]
            }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "workspace_batch".to_string(),
            status: "error".to_string(),
            output: "{\"ok\":false}".to_string(),
            duration_ms: 5,
        };
        let full_reasoning = "完整思考文本：先看文件结构。";
        let assistant_message = provider_native_assistant_tool_call_message_with_reasoning_value(
            None,
            Some(&Value::String(full_reasoning.to_string())),
            &tool_call,
        );

        let messages = openai_messages_with_tool_result(
            &request,
            &mut vec![],
            Some(&assistant_message),
            &tool_call,
            &tool_result,
        );

        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages[1].get("reasoning_content").and_then(Value::as_str),
            Some(full_reasoning)
        );
        assert_eq!(
            messages[2].get("tool_call_id").and_then(Value::as_str),
            Some("call_batch")
        );
    }

    /// Simulates a fragmented HTTP chunk stream by feeding a BufReader with
    /// a Read implementation that yields data in fixed-size pieces.
    /// The underlying BufReader handles line buffering — this tests that
    /// the SSE parser correctly processes data regardless of chunk boundaries.
    #[test]
    fn openai_sse_reader_parses_across_read_boundaries() {
        let raw_text = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\" World\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut deltas = Vec::new();

        // Use a 3-byte Read chunk to force BufReader to refill mid-line
        struct SmallChunks<'a>(&'a [u8], usize);
        impl std::io::Read for SmallChunks<'_> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.0.is_empty() {
                    return Ok(0);
                }
                let end = self.1.min(self.0.len());
                let chunk = &self.0[..end];
                let len = chunk.len().min(buf.len());
                buf[..len].copy_from_slice(&chunk[..len]);
                self.0 = &self.0[end..];
                Ok(len)
            }
        }

        // Wrap in BufReader so BufRead::lines() works incrementally
        let reader = std::io::BufReader::new(SmallChunks(raw_text.as_bytes(), 3));

        let message = collect_openai_sse_message_from_reader(reader, "unit-test", &mut |delta| {
            deltas.push(delta)
        })
        .expect("fragmented stream should parse");

        assert_eq!(message.output_text, "Hello World");
        assert_eq!(
            deltas,
            vec![
                ProviderStreamChunk::Text("Hello".to_string()),
                ProviderStreamChunk::Text(" World".to_string()),
            ]
        );
    }

    /// Verifies the SSE reader can handle multi-byte UTF-8 content
    /// when the underlying Read yields data in small fragments that
    /// split characters across boundaries.
    #[test]
    fn openai_sse_reader_handles_utf8_across_read_boundaries() {
        let raw_text = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"你好世界\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut deltas = Vec::new();

        // 3-byte fragments: Chinese UTF-8 is 3 bytes per char, so fragments
        // will regularly split characters (handled by BufReader's buffering)
        struct SmallChunks<'a>(&'a [u8], usize);
        impl std::io::Read for SmallChunks<'_> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.0.is_empty() {
                    return Ok(0);
                }
                let end = self.1.min(self.0.len());
                let chunk = &self.0[..end];
                let len = chunk.len().min(buf.len());
                buf[..len].copy_from_slice(&chunk[..len]);
                self.0 = &self.0[end..];
                Ok(len)
            }
        }

        let reader = std::io::BufReader::new(SmallChunks(raw_text.as_bytes(), 3));

        let message = collect_openai_sse_message_from_reader(reader, "unit-test", &mut |delta| {
            deltas.push(delta)
        })
        .expect("UTF-8 split across read boundaries should parse");

        assert_eq!(message.output_text, "你好世界");
        assert_eq!(
            deltas,
            vec![ProviderStreamChunk::Text("你好世界".to_string())]
        );
    }

    #[test]
    fn sanitize_request_strips_images_for_non_vision_model() {
        let manager = ProviderManager::new(ResolvedProviderSelection {
            requested_name: "test-deepseek".to_string(),
            provider_name: "deepseek".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: "https://api.deepseek.com/v1".to_string(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "DEEPSEEK_API_KEY".to_string(),
            api_key: Some("test-key".to_string()),
            model: "deepseek-chat".to_string(),
            temperature: 0.0,
            max_output_tokens: 4096,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                supports_image_input: false,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::None,
        });

        let request = ProviderRequest {
            model: "deepseek-chat".to_string(),
            input: vec![ProviderMessage::user("请看图".to_string())],
            images: vec![TurnInputImage {
                data_url: "data:image/png;base64,abc".to_string(),
                mime_type: "image/png".to_string(),
                name: Some("test.png".to_string()),
            }],
            native_messages: Vec::new(),
            observation: Default::default(),
            temperature: 0.0,
            max_output_tokens: 4096,
        };

        let sanitized = manager.sanitize_request(&request);
        assert!(sanitized.images.is_empty(), "images must be stripped by sanitize_request");

        // 如果模型支持图片，则不剥离
        let vision_manager = ProviderManager::new(ResolvedProviderSelection {
            capabilities: ProviderModelCapabilities {
                supports_image_input: true,
                ..Default::default()
            },
            ..manager.config.clone()
        });
        let vision_sanitized = vision_manager.sanitize_request(&request);
        assert_eq!(vision_sanitized.images.len(), 1, "images must be preserved for vision model");
    }

    /// F7 真实验证：`build_streaming_http_client` 的 180s 语义必须是「读空闲」而不是
    /// 「总 deadline」。用一个小 idle 预算 + 真本地 SSE 服务复现两件事：
    /// ① 总时长 > idle（2.5s vs 1s）但间隙 < idle 的流必须完整读完（旧实现会被总 deadline 截断）；
    /// ② 单次间隙 > idle 的连接必须失败为 timeout（真正卡死仍然有兜底）。
    #[test]
    fn streaming_client_uses_idle_read_timeout_not_total_deadline() {
        fn spawn_dribbling_sse_server(frames: usize, gap: Duration) -> (String, thread::JoinHandle<Vec<u8>>) {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind dribbling sse server");
            let address = listener.local_addr().expect("read address");
            let handle = thread::spawn(move || {
                let (mut stream, _) = listener.accept().expect("accept sse request");
                let mut buf = [0_u8; 4096];
                let mut consumed = Vec::new();
                loop {
                    let read = stream.read(&mut buf).expect("read request");
                    if read == 0 {
                        break;
                    }
                    consumed.extend_from_slice(&buf[..read]);
                    if consumed.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
                    )
                    .expect("write headers");
                stream.flush().expect("flush headers");
                for index in 0..frames {
                    // 先睡后写：模拟「首包前后都有静默」的长推理流。
                    thread::sleep(gap);
                    let frame = format!(
                        "data: {{\"choices\":[{{\"delta\":{{\"content\":\"c{index}\"}}}}]}}\n\n"
                    );
                    stream.write_all(frame.as_bytes()).expect("write frame");
                    stream.flush().expect("flush frame");
                }
                consumed
            });
            (format!("http://{}", address), handle)
        }

        let _guard = crate::agent::runtime_helper::TestRuntimeGuard::new();

        // ① 总耗时 10 × 250ms = 2.5s ≫ idle 1s，但每次间隙只有 250ms → 必须成功。
        let (success_endpoint, success_handle) =
            spawn_dribbling_sse_server(10, Duration::from_millis(250));
        let client = build_streaming_http_client(
            Duration::from_secs(5),
            Duration::from_secs(1),
            &success_endpoint,
        )
        .expect("build streaming client");
        let started = Instant::now();
        let response = block_on(client.get(&success_endpoint).send()).expect("stream must connect");
        assert!(response.status().is_success());
        let body = block_on(async move { response.text().await }).expect("long stream must not be cut by a total deadline");
        let elapsed = started.elapsed();
        assert!(
            elapsed > Duration::from_secs(1),
            "用例必须真的超过 idle 预算才有证明力，实际 {elapsed:?}"
        );
        assert!(
            body.contains("c9"),
            "10 帧必须全部读完（旧的总 deadline 会截断），实际正文={body:?}"
        );
        let _ = success_handle.join();

        // ② 单次静默 1.5s > idle 300ms → 必须 timeout 失败（卡死兜底仍在）。
        let (stalled_endpoint, stalled_handle) =
            spawn_dribbling_sse_server(1, Duration::from_millis(1500));
        let client = build_streaming_http_client(
            Duration::from_secs(5),
            Duration::from_millis(300),
            &stalled_endpoint,
        )
        .expect("build stalled client");
        let error = match block_on(client.get(&stalled_endpoint).send()) {
            Ok(response) => block_on(async move { response.text().await }).expect_err("stalled stream must time out"),
            Err(error) => error,
        };
        assert!(
            error.is_timeout(),
            "180s 语义必须是读空闲超时，卡死的流要报 timeout，实际错误={error:?}"
        );
        let _ = stalled_handle.join();
    }

    // ── PA-171 L3-B：冷却期全 scope 短路（ADVISORY） ────────────────────────────────

    /// L3-B ADVISORY：冷却标记错误在 **Followup** 作用域同样一失败即 Abort（不 sleep、不再发请求）。
    #[test]
    fn followup_cooldown_active_error_aborts_without_sleep() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let cooldown_error = crate::agent::retry::append_cooldown_active_marker(
            "provider test 处于限流冷却中（quota），预计 60s 后恢复；本次未发起请求；retry_after=60",
        );
        let (result, report): (Result<(), String>, SingleHopRetryReport) =
            retry_provider_scoped_with_report(
                "followup_sync",
                ProviderRetryScope::Followup,
                || {
                    attempts.set(attempts.get() + 1);
                    Err(cooldown_error.clone())
                },
                &sleeper,
                None,
            );

        println!(
            "[L3-B Followup abort] attempts={} slept_ms={}",
            attempts.get(),
            sleeper.total_slept.load(std::sync::atomic::Ordering::Relaxed)
        );
        assert!(result.is_err());
        assert_eq!(attempts.get(), 1, "冷却期 Followup 必须只尝试一次（不得 sleep 后重放）");
        assert_eq!(
            sleeper.total_slept.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "冷却期不得发生任何退避 sleep"
        );
        assert_eq!(report.attempts_total, 1);
        assert!(!report.switched_to_rl);
    }

    /// L3-B ADVISORY：冷却窗口内第 2 次调用**0 次 HTTP**（用回环 mock 物理计数）。
    #[test]
    fn cooldown_window_short_circuits_without_http() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind cooldown mock");
        let address = listener.local_addr().expect("read cooldown mock address");
        let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let requests_for_thread = Arc::clone(&requests);
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let shutdown_for_thread = Arc::clone(&shutdown);
        let handle = thread::spawn(move || {
            listener
                .set_nonblocking(true)
                .expect("set cooldown mock nonblocking");
            let mut buffer = [0_u8; 4096];
            while !shutdown_for_thread.load(std::sync::atomic::Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                        let _ = stream.read(&mut buffer);
                        requests_for_thread.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let body = "{\"error\":{\"type\":\"exceeded_current_quota_error\"}}";
                        let response = format!(
                            "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: {}\r\nRetry-After: 60\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                    }
                    Err(_) => thread::sleep(Duration::from_millis(10)),
                }
            }
        });

        let config = ResolvedProviderSelection {
            requested_name: "pa171-cooldown-test".to_string(),
            provider_name: "pa171-cooldown-test".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: format!("http://{address}/v1"),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "PA171_COOLDOWN_TEST_API_KEY".to_string(),
            api_key: Some("pa171-dummy".to_string()),
            model: "pa171-cooldown-model".to_string(),
            temperature: 0.0,
            max_output_tokens: 64,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities::default(),
            thinking_param_pattern: ThinkingParamPattern::None,
        };
        let manager = ProviderManager::new(config);
        let endpoint = format!("http://{address}/v1/chat/completions");
        let body = serde_json::json!({});

        let first = manager.post_openai_json(&endpoint, &body);
        assert!(first.is_err(), "首次 429 必须返回 Err");
        let after_first = requests.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(after_first, 1, "首次 429 只应发 1 次 HTTP");

        let second = manager.post_openai_json(&endpoint, &body);
        let after_second = requests.load(std::sync::atomic::Ordering::SeqCst);
        let error = second.expect_err("冷却窗口内必须直接返回可读失败");
        assert_eq!(
            after_second, after_first,
            "冷却窗口内不得再发起 HTTP，实际新增 {} 次",
            after_second - after_first
        );
        println!(
            "[L3-B Followup] http first={} second={} err={}",
            after_first, after_second, error
        );
        assert!(error.contains("冷却"), "冷却失败文案必须可读：{error}");
        assert!(manager.cooldown_remaining().is_some());
        manager.clear_cooldown();

        shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = handle.join();
    }

    // ============================================================
    // B144：工具面动态来源的缓存突变原因
    // ============================================================

    /// wire 串必须与 TS 侧 `PrefixMutationReason` 联合成员一致（`tool_surface_dynamic`），
    /// 且记账接口幂等（同一轮重复调用只留一条）。
    #[test]
    fn tool_surface_dynamic_reason_wire_string_and_idempotent_note() {
        assert_eq!(
            serde_json::to_string(&PrefixMutationReason::ToolSurfaceDynamic)
                .expect("枚举必须可序列化"),
            "\"tool_surface_dynamic\"",
            "wire 串变更必须同步 src/types/runtime.ts 的联合成员"
        );

        let mut observation = ProviderRequestObservation::default();
        observation.note_dynamic_tool_surface(false);
        assert!(
            observation.prefix_mutation_reasons.is_empty(),
            "工具面静态时不得凭空产生突变原因"
        );

        observation.note_dynamic_tool_surface(true);
        observation.note_dynamic_tool_surface(true);
        assert_eq!(
            observation.prefix_mutation_reasons,
            vec![PrefixMutationReason::ToolSurfaceDynamic],
            "同一轮重复记账必须幂等（只留一条）"
        );
    }

