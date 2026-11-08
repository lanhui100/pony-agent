//! 复合工具（batch / gather_context / 分段读取与 start_line 三态）的行为与证据断言。
use super::*;

#[test]
fn batch_returns_partial_success_without_failing_runtime_status() {
    let workspace = temp_workspace();
    fs::write(workspace.join("demo.txt"), "hello\nworld\n").expect("write demo file");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_BATCH.to_string(),
        arguments: json!({
            "parallel": true,
            "continueOnError": true,
            "calls": [
                {
                    "name": TOOL_WORKSPACE_PATH_INFO,
                    "arguments": { "path": "demo.txt" }
                },
                {
                    "name": TOOL_WORKSPACE_READ_FILE,
                    "arguments": { "path": "missing.txt" }
                }
            ]
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
    assert_eq!(
        payload.get("status").and_then(Value::as_str),
        Some("partial")
    );
    assert_eq!(payload.get("successCount").and_then(Value::as_u64), Some(1));
}

#[test]
fn gather_context_reads_file_info_and_segment() {
    let workspace = temp_workspace();
    fs::write(
        workspace.join("demo.rs"),
        "fn main() {}\nprintln!(\"hi\");\n",
    )
    .expect("write file");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
        arguments: json!({ "path": "demo.rs", "lineCount": 20 }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
    assert_eq!(
        payload
            .get("meta")
            .and_then(Value::as_object)
            .and_then(|meta| meta.get("mode"))
            .and_then(Value::as_str),
        Some("file")
    );
    assert_eq!(payload.get("successCount").and_then(Value::as_u64), Some(2));
}

#[test]
fn gather_context_threads_explicit_start_line_in_file_mode() {
    let workspace = temp_workspace();
    let body = (1..=10)
        .map(|index| format!("line-{index}"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(workspace.join("paged.rs"), body).expect("write paged.rs");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
        arguments: json!({ "path": "paged.rs", "startLine": 6, "lineCount": 3 }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
    assert_eq!(payload["meta"]["mode"], "file");
    let results = payload
        .get("results")
        .and_then(Value::as_array)
        .expect("results");
    assert_eq!(results[1]["tool"], "workspace_read_file_segment");
    // 嵌套回显真实 startLine（PA-100：事故根因即模型模仿回显里的 startLine）。
    assert_eq!(results[1]["arguments"]["startLine"].as_u64(), Some(6));
    let segment_text = results[1]["output"].as_str().expect("segment text output");
    assert!(
        segment_text.contains("第 6 - 8 行") || segment_text.contains("第 6"),
        "segment should start at line 6: {segment_text}"
    );
    assert!(segment_text.contains("line-6"));
    assert!(!segment_text.contains("line-5"));
    let steps = payload["plan"]["steps"].as_array().expect("plan steps");
    let segment_step = steps
        .iter()
        .find(|step| step["name"] == "workspace_read_file_segment")
        .expect("segment step");
    assert_eq!(segment_step["arguments"]["startLine"].as_u64(), Some(6));
}

#[test]
fn gather_context_explicit_start_line_overrides_search_auto_position() {
    let workspace = temp_workspace();
    let mut lines: Vec<String> = (1..=8).map(|index| format!("filler-{index}")).collect();
    lines[4] = "needle-line".to_string();
    fs::write(workspace.join("searched.rs"), lines.join("\n")).expect("write searched.rs");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
        arguments: json!({
            "path": "searched.rs",
            "query": "needle",
            "startLine": 2,
            "lineCount": 3
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
    assert_eq!(payload["meta"]["mode"], "search");
    let results = payload
        .get("results")
        .and_then(Value::as_array)
        .expect("results");
    // 显式 startLine 全模式生效：覆盖"命中行 5 - lineCount/2 → 4"的自动定位。
    let segment_entry = results
        .iter()
        .find(|entry| entry["tool"] == "workspace_read_file_segment")
        .expect("segment entry");
    assert_eq!(segment_entry["arguments"]["startLine"].as_u64(), Some(2));
    let segment_text = segment_entry["output"]
        .as_str()
        .expect("segment text output");
    assert!(segment_text.contains("第 2 - 4 行") || segment_text.contains("第 2"));
}

/// T2-B F1 语义B：search × Invalid 强制 1 并忽略搜索自动定位（与 file 一致）。
/// 命中行 5、lineCount=3 时 auto 本应是 4，Invalid 必须回 1 而非 4。
#[test]
fn gather_context_search_invalid_start_line_forces_one_ignoring_auto() {
    let workspace = temp_workspace();
    let mut lines: Vec<String> = (1..=8).map(|index| format!("filler-{index}")).collect();
    lines[4] = "needle-line".to_string();
    fs::write(workspace.join("searched.rs"), lines.join("\n")).expect("write searched.rs");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
        arguments: json!({
            "path": "searched.rs",
            "query": "needle",
            "startLine": -5,
            "lineCount": 3
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
    assert_eq!(payload["meta"]["mode"], "search");
    let results = payload
        .get("results")
        .and_then(Value::as_array)
        .expect("results");
    let segment_entry = results
        .iter()
        .find(|entry| entry["tool"] == "workspace_read_file_segment")
        .expect("segment entry");
    // 非 auto（auto=4）：Invalid 强制 1。
    assert_eq!(segment_entry["arguments"]["startLine"].as_u64(), Some(1));
    assert_eq!(segment_entry["arguments"]["requestedStartLine"], json!(-5));
    assert_eq!(payload["meta"]["startLineFallback"]["requested"], json!(-5));
    assert_eq!(
        payload["meta"]["startLineFallback"]["applied"].as_u64(),
        Some(1)
    );
    assert_eq!(
        payload["meta"]["startLineFallback"]["reason"].as_str(),
        Some("invalid_start_line_fallback_to_1")
    );
}

/// T2-B F1：directory 模式不消费 startLine——Invalid 也不打标。
#[test]
fn gather_context_directory_invalid_start_line_marks_nothing() {
    let workspace = temp_workspace();
    fs::create_dir_all(workspace.join("subdir")).expect("create subdir");
    fs::write(workspace.join("subdir/inner.rs"), "fn inner() {}\n").expect("write inner");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
        arguments: json!({ "path": "subdir", "startLine": -5 }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
    assert_eq!(payload["meta"]["mode"], "directory");
    assert!(payload["meta"].get("startLineFallback").is_none());
}

/// PA-100 审核 A 缺失测试⑤：legacy 侧多路径 startLine 回显（与 composite 侧
/// gather_context_multi_path_echoes_start_line_in_plan_and_skipped_entries 对称）。
#[test]
fn gather_context_multi_path_echoes_start_line_in_plan() {
    let workspace = temp_workspace();
    fs::write(workspace.join("one.rs"), "fn one() {}\n").expect("write one");
    fs::write(workspace.join("two.rs"), "fn two() {}\n").expect("write two");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
        arguments: json!({
            "paths": ["one.rs", "two.rs"],
            "startLine": 9,
            "lineCount": 20
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
    let steps = payload["plan"]["steps"].as_array().expect("plan steps");
    assert_eq!(steps.len(), 2);
    for step in steps {
        assert_eq!(step["arguments"]["startLine"].as_u64(), Some(9));
    }
    // 每条 per-path 结果的嵌套 arguments 同样回显。
    let results = payload
        .get("results")
        .and_then(Value::as_array)
        .expect("results");
    for entry in results {
        assert_eq!(entry["arguments"]["startLine"].as_u64(), Some(9));
    }
}

#[test]
fn gather_context_can_aggregate_multiple_paths() {
    let workspace = temp_workspace();
    fs::write(workspace.join("one.rs"), "fn one() {}\n").expect("write one");
    fs::write(workspace.join("two.rs"), "fn two() {}\n").expect("write two");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
        arguments: json!({
            "paths": ["one.rs", "two.rs"],
            "lineCount": 20
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
    assert_eq!(
        payload
            .get("meta")
            .and_then(Value::as_object)
            .and_then(|meta| meta.get("mode"))
            .and_then(Value::as_str),
        Some("multi_path")
    );
    assert_eq!(payload.get("successCount").and_then(Value::as_u64), Some(2));
}

#[test]
fn gather_context_limits_too_many_paths_as_partial_result() {
    let workspace = temp_workspace();
    for index in 0..=MAX_GATHER_CONTEXT_PATHS {
        fs::write(workspace.join(format!("demo-{index}.rs")), "fn demo() {}\n")
            .expect("write demo");
    }
    let router = ToolRouter::with_workspace_root(workspace.clone());
    let paths = (0..=MAX_GATHER_CONTEXT_PATHS)
        .map(|index| Value::String(format!("demo-{index}.rs")))
        .collect::<Vec<_>>();

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
        arguments: json!({
            "paths": paths,
            "lineCount": 20
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
    assert_eq!(
        payload
            .get("meta")
            .and_then(|meta| meta.get("requestedPathCount"))
            .and_then(Value::as_u64),
        Some((MAX_GATHER_CONTEXT_PATHS + 1) as u64)
    );
    assert_eq!(
        payload.get("status").and_then(Value::as_str),
        Some("partial")
    );
    assert_eq!(
        payload.get("successCount").and_then(Value::as_u64),
        Some(MAX_GATHER_CONTEXT_PATHS as u64)
    );
    assert_eq!(payload.get("partialCount").and_then(Value::as_u64), Some(1));
    assert_eq!(
        payload
            .get("meta")
            .and_then(|meta| meta.get("skippedPaths"))
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        payload
            .get("results")
            .and_then(Value::as_array)
            .and_then(|results| results.last())
            .and_then(|entry| entry.get("output"))
            .and_then(|output| output.get("reason"))
            .and_then(Value::as_str),
        Some("too_many_paths")
    );
}

// ── T2-A：startLine 三态可观测契约 ────────────────────────────────────────────
// dispatcher 硬门拒浮点/字符串/bool（见 dispatcher.rs 门单测）；legacy direct-call
// 不过硬门，全类型进入 `parse_start_line_arg`：
// 缺席/null→Missing；as_u64 成功→Valid(max(1)，0→1 clamp 无标记)；
// 其余（负数/浮点/字符串/bool）→Invalid，回退 applied=1 + `requestedStartLine` +
// `meta.startLineFallback` 留痕，顶层 status 语义不动。

#[test]
fn parse_start_line_arg_tristate_matrix() {
    // Missing：缺席 / null。
    assert_eq!(parse_start_line_arg(&json!({})), StartLineArg::Missing);
    assert_eq!(
        parse_start_line_arg(&json!({ "startLine": null })),
        StartLineArg::Missing
    );
    // Valid：正整数直通；0 clamp 到 1（无 fallback 标记，由调用方断言）。
    assert_eq!(
        parse_start_line_arg(&json!({ "startLine": 80 })),
        StartLineArg::Valid(80)
    );
    assert_eq!(
        parse_start_line_arg(&json!({ "startLine": 1 })),
        StartLineArg::Valid(1)
    );
    assert_eq!(
        parse_start_line_arg(&json!({ "startLine": 0 })),
        StartLineArg::Valid(1)
    );
    // Invalid：负数/浮点/字符串/bool/数组/对象（raw 原样保留）。
    assert_eq!(
        parse_start_line_arg(&json!({ "startLine": -5 })),
        StartLineArg::Invalid { raw: json!(-5) }
    );
    assert_eq!(
        parse_start_line_arg(&json!({ "startLine": 80.5 })),
        StartLineArg::Invalid { raw: json!(80.5) }
    );
    assert_eq!(
        parse_start_line_arg(&json!({ "startLine": "80" })),
        StartLineArg::Invalid { raw: json!("80") }
    );
    assert_eq!(
        parse_start_line_arg(&json!({ "startLine": true })),
        StartLineArg::Invalid { raw: json!(true) }
    );
    // 兼容薄层零漂移：Valid→Some，Missing/Invalid→None（调用方 unwrap_or(1)）。
    assert_eq!(
        explicit_gather_start_line(&json!({ "startLine": 80 })),
        Some(80)
    );
    assert_eq!(
        explicit_gather_start_line(&json!({ "startLine": 0 })),
        Some(1)
    );
    assert_eq!(explicit_gather_start_line(&json!({})), None);
    assert_eq!(
        explicit_gather_start_line(&json!({ "startLine": -5 })),
        None
    );
    assert_eq!(
        explicit_gather_start_line(&json!({ "startLine": "80" })),
        None
    );
}

/// 非法矩阵 × legacy file 模式：行为（applied）+ fallback 标记有无 +
/// nested `requestedStartLine`。顶层 status 始终不动。
#[test]
fn gather_context_start_line_tristate_fallback_observability() {
    let workspace = temp_workspace();
    let body = (1..=10)
        .map(|index| format!("line-{index}"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(workspace.join("paged.rs"), body).expect("write paged.rs");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    // (输入 startLine, 期望 applied, 是否应有 fallback 标记)
    let matrix: Vec<(Value, u64, bool)> = vec![
        (json!(-5), 1, true),
        (json!(0), 1, false),
        (json!("80"), 1, true),
        (json!(true), 1, true),
        (json!(80.5), 1, true),
        (Value::Null, 1, false),
    ];
    for (start_line, applied, fallback) in matrix {
        let mut args = serde_json::Map::new();
        args.insert("path".to_string(), Value::String("paged.rs".to_string()));
        args.insert("lineCount".to_string(), json!(3));
        if !start_line.is_null() {
            args.insert("startLine".to_string(), start_line.clone());
        }
        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
            arguments: Value::Object(args),
            plan: None,
        });
        assert_eq!(result.status, "ok", "input: {start_line}");
        let payload =
            serde_json::from_str::<Value>(&result.output).expect("gather output json");
        assert_eq!(payload["meta"]["mode"], "file", "input: {start_line}");
        let segment = payload["results"]
            .as_array()
            .expect("results")
            .iter()
            .find(|entry| entry["tool"] == "workspace_read_file_segment")
            .expect("segment entry");
        assert_eq!(
            segment["arguments"]["startLine"].as_u64(),
            Some(applied),
            "input: {start_line}"
        );
        if fallback {
            assert_eq!(
                segment["arguments"]["requestedStartLine"], start_line,
                "input: {start_line}"
            );
            assert_eq!(
                payload["meta"]["startLineFallback"]["requested"], start_line,
                "input: {start_line}"
            );
            assert_eq!(
                payload["meta"]["startLineFallback"]["applied"].as_u64(),
                Some(1),
                "input: {start_line}"
            );
            assert_eq!(
                payload["meta"]["startLineFallback"]["reason"].as_str(),
                Some("invalid_start_line_fallback_to_1"),
                "input: {start_line}"
            );
        } else {
            assert!(
                segment["arguments"].get("requestedStartLine").is_none(),
                "input: {start_line}"
            );
            assert!(
                payload["meta"].get("startLineFallback").is_none(),
                "input: {start_line}"
            );
        }
    }
    // 缺省 startLine：Missing，无标记。
    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
        arguments: json!({ "path": "paged.rs", "lineCount": 3 }),
        plan: None,
    });
    let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
    assert!(payload["meta"].get("startLineFallback").is_none());
}

/// segment 直读与 gather 三实现一致：0→第1行；非法值→回退第1行。
#[test]
fn read_file_segment_start_line_parity_with_gather() {
    let workspace = temp_workspace();
    fs::write(workspace.join("tiny.rs"), "aaa\nbbb\nccc\n").expect("write tiny.rs");
    let router = ToolRouter::with_workspace_root(workspace.clone());
    for start_line in [json!(0), json!(-5), json!("80"), json!(true)] {
        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
            arguments: json!({ "path": "tiny.rs", "startLine": start_line, "lineCount": 2 }),
            plan: None,
        });
        assert_eq!(result.status, "ok", "input: {start_line}");
        assert!(
            result.output.contains("aaa"),
            "invalid startLine must fall back to line 1: {start_line} -> {}",
            result.output
        );
    }
}

/// EOF 正交组合覆盖（T2-B F3 修订：此前“共存”措辞过声明——实际是两个 payload
/// 各断一半）：Valid 大值→partial 无标；Invalid→有标无 partial；
/// multi_path 混合 payload 可同现两者条目。顶层 status 语义不动（ok + 聚合 partial）。
#[test]
fn gather_context_eof_out_of_range_orthogonal_matrix() {
    let workspace = temp_workspace();
    fs::write(workspace.join("short.rs"), "one\ntwo\n").expect("write short.rs");
    fs::write(workspace.join("paged.rs"), "a\nb\nc\nd\n").expect("write paged.rs");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    // 单路径 Valid 越界：partial + firstError line_out_of_range，无 fallback 标记。
    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
        arguments: json!({ "path": "short.rs", "startLine": 99, "lineCount": 10 }),
        plan: None,
    });
    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
    assert_eq!(payload["status"], "partial");
    assert_eq!(
        payload["summary"]["firstError"]["error"]["code"].as_str(),
        Some("line_out_of_range")
    );
    assert!(payload["meta"].get("startLineFallback").is_none());

    // multi_path 正交：外层 Valid(99) 无外层 fallback 标记，per-path 内层越界
    // partial；与外层 Invalid 有标无 partial 的分支正交互补（见下段）。
    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
        arguments: json!({ "paths": ["paged.rs", "short.rs"], "startLine": 99, "lineCount": 10 }),
        plan: None,
    });
    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
    assert_eq!(payload["status"], "partial");
    // 注意：multi_path 外层 startLine=99 是 Valid（整型），无外层 fallback 标记；
    // 越界发生在 per-path 递归内层。此处断言共存语义：partial + firstError 透出。
    assert_eq!(
        payload["summary"]["firstError"]["error"]["code"].as_str(),
        Some("line_out_of_range")
    );

    // multi_path 外层 Invalid：meta.startLineFallback + 各 nested requestedStartLine。
    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
        arguments: json!({ "paths": ["paged.rs"], "startLine": -5, "lineCount": 10 }),
        plan: None,
    });
    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
    assert_eq!(payload["meta"]["startLineFallback"]["requested"], json!(-5));
    assert_eq!(
        payload["meta"]["startLineFallback"]["reason"].as_str(),
        Some("invalid_start_line_fallback_to_1")
    );
    for step in payload["plan"]["steps"].as_array().expect("plan steps") {
        assert_eq!(step["arguments"]["startLine"].as_u64(), Some(1));
        assert_eq!(step["arguments"]["requestedStartLine"], json!(-5));
    }
}

#[test]
fn workspace_batch_accepts_seventeen_subcalls_for_repo_exploration() {
    let workspace = temp_workspace();
    for index in 0..17 {
        fs::write(
            workspace.join(format!("demo-{index}.txt")),
            format!("file {index}\n"),
        )
        .expect("write demo");
    }
    let router = ToolRouter::with_workspace_root(workspace.clone());
    let calls = (0..17)
        .map(|index| {
            json!({
                "name": TOOL_WORKSPACE_PATH_INFO,
                "arguments": { "path": format!("demo-{index}.txt") }
            })
        })
        .collect::<Vec<_>>();

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_BATCH.to_string(),
        arguments: json!({
            "parallel": true,
            "continueOnError": true,
            "calls": calls
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
    assert_eq!(
        payload.get("successCount").and_then(Value::as_u64),
        Some(17)
    );
    assert_eq!(payload.get("status").and_then(Value::as_str), Some("ok"));
}

#[test]
fn gather_context_search_file_includes_segment_even_without_match() {
    let workspace = temp_workspace();
    fs::write(
        workspace.join("demo.rs"),
        "fn main() {}\nprintln!(\"hi\");\n",
    )
    .expect("write demo");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
        arguments: json!({
            "path": "demo.rs",
            "query": "missing_symbol",
            "lineCount": 20
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
    assert_eq!(payload.get("plannedCount").and_then(Value::as_u64), Some(3));
    assert!(payload
        .get("results")
        .and_then(Value::as_array)
        .map(|results| {
            results.iter().any(|entry| {
                entry.get("tool").and_then(Value::as_str)
                    == Some(TOOL_WORKSPACE_READ_FILE_SEGMENT)
            })
        })
        .unwrap_or(false));
}

#[test]
fn gather_context_search_directory_falls_back_to_listing_on_empty_match() {
    let workspace = temp_workspace();
    fs::create_dir_all(workspace.join("src")).expect("create src dir");
    fs::write(workspace.join("src/lib.rs"), "pub struct ToolRouter;\n").expect("write lib");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
        arguments: json!({
            "path": "src",
            "query": "missing_symbol",
            "limit": 10
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
    assert_eq!(payload.get("plannedCount").and_then(Value::as_u64), Some(3));
    assert!(payload
        .get("results")
        .and_then(Value::as_array)
        .map(|results| {
            results.iter().any(|entry| {
                entry.get("tool").and_then(Value::as_str) == Some(TOOL_WORKSPACE_LIST_FILES)
            })
        })
        .unwrap_or(false));
}

#[test]
fn read_file_segment_repairs_unique_same_name_file_path() {
    let workspace = temp_workspace();
    fs::create_dir_all(workspace.join("src-tauri")).expect("create src-tauri dir");
    fs::write(
        workspace.join("tauri.conf.json"),
        "{\n  \"productName\": \"Pony Agent\"\n}\n",
    )
    .expect("write tauri config");
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
        arguments: json!({
            "path": "src-tauri/tauri.conf.json",
            "startLine": 1,
            "lineCount": 5
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    assert!(result.output.contains("tauri.conf.json"));
    assert!(result.output.contains("productName"));
}

#[test]
fn read_file_segment_keeps_error_when_same_name_file_is_ambiguous() {
    let workspace = temp_workspace();
    fs::create_dir_all(workspace.join("src-tauri")).expect("create src-tauri dir");
    fs::create_dir_all(workspace.join("nested")).expect("create nested dir");
    fs::write(workspace.join("tauri.conf.json"), "{}\n").expect("write root tauri config");
    fs::write(workspace.join("nested/tauri.conf.json"), "{}\n")
        .expect("write nested tauri config");
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
        arguments: json!({
            "path": "src-tauri/tauri.conf.json",
            "startLine": 1,
            "lineCount": 5
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    assert!(result.output.contains("多个同名文件"));
}

#[test]
fn gather_context_repairs_unique_missing_extension_file_path() {
    let workspace = temp_workspace();
    fs::create_dir_all(workspace.join("src/agent")).expect("create agent dir");
    fs::write(
        workspace.join("src/agent/context.rs"),
        "pub struct AgentContext;\n",
    )
    .expect("write context file");
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
        arguments: json!({
            "path": "src/agent/context"
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
    assert_eq!(payload.get("status").and_then(Value::as_str), Some("ok"));
    assert!(payload
        .get("results")
        .and_then(Value::as_array)
        .map(|results| {
            results.iter().any(|entry| {
                entry.get("tool").and_then(Value::as_str)
                    == Some(TOOL_WORKSPACE_READ_FILE_SEGMENT)
            })
        })
        .unwrap_or(false));
}

#[test]
fn gather_context_keeps_error_when_missing_extension_path_is_ambiguous() {
    let workspace = temp_workspace();
    fs::create_dir_all(workspace.join("src/agent")).expect("create agent dir");
    fs::create_dir_all(workspace.join("nested")).expect("create nested dir");
    fs::write(
        workspace.join("src/agent/context.rs"),
        "pub struct AgentContext;\n",
    )
    .expect("write first context file");
    fs::write(
        workspace.join("nested/context.rs"),
        "pub struct OtherContext;\n",
    )
    .expect("write second context file");
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
        arguments: json!({
            "path": "context"
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    assert!(result.output.contains("多个缺扩展名候选文件"));
}

#[test]
fn batch_rejects_nested_workspace_batch_calls() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_BATCH.to_string(),
        arguments: json!({
            "calls": [
                {
                    "name": "workspace_batch",
                    "arguments": {
                        "calls": [
                            {
                                "name": "workspace_list_files",
                                "arguments": { "path": "." }
                            }
                        ]
                    }
                }
            ]
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("unsupported_composite_child")
    );
    let message = payload
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .expect("rejection message");
    assert!(
        message.contains("workspace_batch"),
        "rejection message should name the recursive child: {}",
        message
    );
    assert!(
        message.contains("composite"),
        "rejection message should name the recursive child scope: {}",
        message
    );
}

#[test]
fn batch_executes_read_only_children() {
    let workspace = temp_workspace();
    fs::write(workspace.join("demo.txt"), "hello\nworld\n").expect("write demo");
    fs::create_dir_all(workspace.join("src")).expect("create src");
    fs::write(workspace.join("src/lib.rs"), "pub fn demo() {}\n").expect("write lib");
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_BATCH.to_string(),
        arguments: json!({
            "parallel": true,
            "continueOnError": true,
            "calls": [
                { "name": TOOL_WORKSPACE_LIST_FILES, "arguments": { "path": "." } },
                { "name": TOOL_WORKSPACE_READ_FILE, "arguments": { "path": "demo.txt" } },
                {
                    "name": TOOL_WORKSPACE_READ_FILE_SEGMENT,
                    "arguments": { "path": "demo.txt", "startLine": 1, "lineCount": 2 }
                },
                { "name": TOOL_WORKSPACE_PATH_INFO, "arguments": { "path": "demo.txt" } },
                { "name": TOOL_WORKSPACE_SEARCH_TEXT, "arguments": { "query": "hello", "path": "." } },
                { "name": TOOL_WORKSPACE_GLOB_FILES, "arguments": { "pattern": "src/*.rs", "path": "." } },
            ]
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
    assert_eq!(payload.get("status").and_then(Value::as_str), Some("ok"));
    assert_eq!(payload.get("successCount").and_then(Value::as_u64), Some(6));
    assert_eq!(payload.get("errorCount").and_then(Value::as_u64), Some(0));
}

#[test]
fn batch_executes_read_only_product_aliases() {
    let workspace = temp_workspace();
    fs::write(workspace.join("demo.txt"), "needle\n").expect("write demo");
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_BATCH.to_string(),
        arguments: json!({
            "calls": [
                { "name": "List", "arguments": { "path": "." } },
                { "name": "Glob", "arguments": { "pattern": "*.txt", "path": "." } },
                { "name": "Search", "arguments": { "query": "needle", "path": "." } },
            ]
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
    assert_eq!(payload.get("status").and_then(Value::as_str), Some("ok"));
    assert_eq!(payload.get("successCount").and_then(Value::as_u64), Some(3));
}

#[test]
fn batch_rejects_write_child_without_executing_it() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace.clone());
    let payload = assert_batch_child_rejected(
        &router,
        json!({
            "name": TOOL_WORKSPACE_WRITE_FILE,
            "arguments": { "path": "x.txt", "content": "should not land" }
        }),
        "workspace.write",
    );
    let message = payload
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .expect("message");
    assert!(
        message.contains(TOOL_WORKSPACE_WRITE_FILE),
        "message should name the child: {}",
        message
    );
    assert!(
        !workspace.join("x.txt").exists(),
        "write child must not execute under the read-only gate"
    );
}

#[test]
fn batch_rejects_edit_child_with_unsupported_composite_child() {
    let workspace = temp_workspace();
    fs::write(workspace.join("demo.txt"), "original").expect("write demo");
    let router = ToolRouter::with_workspace_root(workspace.clone());
    assert_batch_child_rejected(
        &router,
        json!({
            "name": TOOL_WORKSPACE_EDIT_FILE,
            "arguments": { "path": "demo.txt", "oldText": "original", "newText": "changed" }
        }),
        "workspace.write",
    );
    assert_eq!(
        fs::read_to_string(workspace.join("demo.txt")).expect("read demo"),
        "original",
        "edit child must not execute under the read-only gate"
    );
}

#[test]
fn batch_rejects_run_child_with_unsupported_composite_child() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);
    assert_batch_child_rejected(
        &router,
        json!({
            "name": TOOL_WORKSPACE_RUN_COMMAND,
            "arguments": { "command": "echo should-not-run" }
        }),
        "workspace.execute",
    );
}

#[test]
fn batch_rejects_web_children_with_unsupported_composite_child() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);
    assert_batch_child_rejected(
        &router,
        json!({
            "name": TOOL_WEB_FETCH_URL,
            "arguments": { "url": "https://example.com" }
        }),
        "web",
    );
    assert_batch_child_rejected(
        &router,
        json!({
            "name": TOOL_WEB_SEARCH_QUERY,
            "arguments": { "query": "pony" }
        }),
        "web",
    );
}

#[test]
fn batch_rejects_mcp_child_with_unsupported_composite_child() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);
    assert_batch_child_rejected(
        &router,
        json!({
            "name": TOOL_MCP_RESOURCE_READ,
            "arguments": { "capabilityId": "mcp:resource:repo-index" }
        }),
        "mcp",
    );
}

#[test]
fn batch_rejects_tool_search_child_with_unsupported_composite_child() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);
    assert_batch_child_rejected(
        &router,
        json!({ "name": TOOL_TOOL_SEARCH, "arguments": { "query": "workspace" } }),
        "capability.discovery",
    );
}

#[test]
fn batch_rejects_gather_context_child_with_unsupported_composite_child() {
    let workspace = temp_workspace();
    fs::write(workspace.join("demo.txt"), "demo\n").expect("write demo");
    let router = ToolRouter::with_workspace_root(workspace);
    // Primitive name, and the product alias "Read" that canonicalizes to the
    // same composite primitive (used today by the local planner).
    assert_batch_child_rejected(
        &router,
        json!({
            "name": TOOL_WORKSPACE_GATHER_CONTEXT,
            "arguments": { "path": "demo.txt" }
        }),
        "composite",
    );
    assert_batch_child_rejected(
        &router,
        json!({ "name": "Read", "arguments": { "path": "demo.txt" } }),
        "composite",
    );
}

#[test]
fn batch_rejects_interactive_children_with_unsupported_composite_child() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);
    assert_batch_child_rejected(
        &router,
        json!({ "name": TOOL_ECHO_INPUT, "arguments": { "text": "hi" } }),
        "interactive",
    );
}

#[test]
fn batch_rejects_external_tool_child_with_unsupported_composite_child() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);
    assert_batch_child_rejected(
        &router,
        json!({ "name": "some_external_tool", "arguments": {} }),
        "external",
    );
}

#[test]
fn batch_rejects_offending_child_before_any_sibling_executes() {
    let workspace = temp_workspace();
    fs::write(workspace.join("safe.txt"), "read me\n").expect("write safe");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_BATCH.to_string(),
        arguments: json!({
            "calls": [
                { "name": TOOL_WORKSPACE_READ_FILE, "arguments": { "path": "safe.txt" } },
                {
                    "name": TOOL_WORKSPACE_WRITE_FILE,
                    "arguments": { "path": "should-not-land.txt", "content": "nope" }
                }
            ]
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("unsupported_composite_child")
    );
    assert!(
        !workspace.join("should-not-land.txt").exists(),
        "no sibling may execute once a batch is rejected"
    );
}

#[test]
fn batch_requires_at_least_one_call() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_BATCH.to_string(),
        arguments: json!({ "calls": [] }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("missing_argument")
    );
}

#[test]
fn batch_rejects_too_many_calls() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);
    let calls = (0..=MAX_WORKSPACE_BATCH_CALLS)
        .map(|index| {
            json!({
                "name": TOOL_WORKSPACE_PATH_INFO,
                "arguments": { "path": format!("demo-{index}.txt") }
            })
        })
        .collect::<Vec<_>>();

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_BATCH.to_string(),
        arguments: json!({ "calls": calls }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("too_many_calls")
    );
}
