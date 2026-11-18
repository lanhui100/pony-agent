# Agent Note: 自动标题跟随会话实际 provider/model（不再固定走默认 provider）

Status: implemented

工单：B201。影响文件：`agent/control_plane/title_commands.rs`、`agent/runtime/title_ops.rs`、
`agent/session/store.rs`、`agent/config.rs`、`agent/runtime/compaction_ops.rs`。

## Problem

现场 `tui-runtime.log` 每个 turn 收尾都出现
`auto_title_generation_failed: session=… error=provider openai missing API key…`，而该轮实际
provider 是 ponyllm。原因：`title_provider()` 走 `runtime.compaction_provider()` →
`resolve_provider_selection(None, None)`，只按注册表的 selected/first provider 解析，与会话实际
使用的 provider/model 无关；自动路径失败时每轮 `eprintln!`，噪声与必然失败的后台调用叠加，
标题能力形同虚设（标题停留在用户输入前 10 字）。

## Decision

1. `SessionStore::session_provider_and_model()`：从会话最近 turn trace（含 history node 的 trace）
   倒序提取 `provider_name/provider_requested_name` 与 `provider_model`——标题是"会话级"能力，
   最近一轮 trace 就是该会话实际 provider 的最小可得真源，不新增持久化字段。
2. `AgentRuntime::title_provider(provider_id, model_id)`：把会话 provider/model 作为**提示**传给
   `resolve_provider_selection`；配置层同时接受 provider 的 `id` 与 `name`、model 的
   `id`/`model`/`name` 别名（trace 记的是展示名而注册表以 id 为键），仍保持"找不到则回落
   selected/first"的既有语义。
3. 自动路径 `maybe_generate_session_title()`：
   - 无 API key / 解析不到可用 provider ⇒ **静默跳过**，每个 session 只记一次 debug（不发起
     必然失败的调用）；
   - 其它错误仍保留 `auto_title_generation_failed` 取证面（网络抖动等真实故障不能被吞掉）；
   - 手动 `/title` 行为完全不变（仍走默认解析、错误照旧上报）。

## Alternatives considered

- **A. 只把默认 provider 改成"最后一个用过的 provider"的进程级全局变量。** 落选：多会话并发时
  会串会话（A 会话的标题用 B 会话的 provider），且重启即丢。
- **B. 给会话新增持久化的 provider/model 字段。** 落选：需要 schema 迁移与所有写入路径同步；
  最近 turn trace 已经记录该事实（`TurnTraceRecord.provider_name/provider_model`），零迁移可得。
- **C. 自动标题完全关闭。** 落选：标题能力是既有产品面，问题只是 provider 选择错；关掉等于
  退化功能。

## Verification

- `cargo-lane test -p pony-agent-core --lib -- title`：4 + 7 例（含
  `test_auto_title_prefers_session_provider_and_model`、`test_auto_title_no_key_skips_without_repeated_failing_calls`、
  `test_manual_title_preserves_behavior`）全绿；
- 定向 `bash scripts/cargo-lane.sh check -p pony-agent-core` exit 0；
- 会话 provider/model 读取路径有单测覆盖（store 侧真实 trace 注入）。
