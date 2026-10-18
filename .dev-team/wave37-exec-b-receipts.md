# wave-37 executor-b (task-4, C3) 物理收据

- 日期：2026-10-18（Asia/Shanghai）
- 执行者：w37-exec-b
- 域：`subagent_runtime.rs`（HostTurnEngine 真实现）、`orchestration.rs`（subagent() 真接线）、`tools.rs`（仅 SUBAGENT_RUNTIME_WIRED 判定行）

## 交付内容

1. `crates/pony-agent-core/src/agent/subagent_runtime.rs`
   - `HostTurnEngine::run_turn` 真实现：`AgentRuntimeBuilder::new().workspace_root(compute_default_workspace_root().unwrap_or(cwd))` 独立建栈（builder 内部默认 `SessionStore::new()` / `ProviderRegistryStore::new()`，与宿主隔离）；`TurnInput { message: prompt, session_id: Some("subagent-<uuid>"), .. }` → `runtime.run_turn(input)`；`TurnResult.assistant_message` 提取 final 文本（空则回退 `session_summary`）。
   - prompt 字节 >200000 → `Err("prompt_too_large")`；并发 AtomicUsize CAS 上限 2 → 超限 `Err("subagent_busy")`；Drop-guard scope guard 保证 panic 路径许可必归还；工作线程 `catch_unwind` 包裹建栈+run_turn。
   - 超时：`mpsc::channel` + `recv_timeout`（≤1s 粒度），超时 → `Err("subagent_timeout: orphan session_id=... ")`（无 stop API，session_id 记入错误消息标注孤儿）。
   - 新增导出 `subagent_gate_enter(prompt)`：入口统一过门（oversize/busy 在 orchestration 层先于引擎判定，使 C3.2 测试 J/K 以任意注入引擎验证均成立）。
2. `crates/pony-agent-core/src/agent/orchestration.rs`
   - `subagent()`：槽空 → `Err` 含 `capability_unavailable`；有引擎 → 过门后同步调用，成功 `status:"completed"` + `output:Some(final_text)`，引擎 Err 原样透传；`run_in_background=true` → spawn 线程执行（许可随 guard 移入后台线程全程占用），立即返回 `status:"running"` + `output:None`，后台错误仅 `eprintln!` 记录。`workflow()` 未动（wave-38 范围）。
3. `crates/pony-agent-core/src/agent/tools.rs`（唯一改动，最小改动面选择：**保留 pub const 并置 true + 新增运行时函数**）
   - `pub const SUBAGENT_RUNTIME_WIRED: bool = true;`（保留供外部二进制兼容引用）
   - 新增 `pub fn subagent_runtime_wired() -> bool { crate::agent::subagent_runtime::subagent_engine().is_some() }`
   - 10 处 `if !SUBAGENT_RUNTIME_WIRED` 判断点全部改为 `if !subagent_runtime_wired()`（grep 确认 1228/1257/1286/1315/1335/1364/1393/1422/1451/1480 → 改后 1238/1267/1296/1325/1345/1374/1403/1432/1461/1490 共 10 处，无遗漏）。
4. lib.rs 无需动（agent/mod.rs 已挂 `pub mod subagent_runtime;`，task-1 已接线）。

## 验证收据

### cargo check -p pony-agent-core --lib

```
Finished `dev` profile [unoptimized] target(s) in 2m 48s
cargo_check_exit=0
```

（注：中途一次失败系队友 in-flight 编辑 control_plane/mod.rs 语法暂态（error at control_plane/mod.rs:6263，非本域），等待后复检通过。）

### cargo test -p pony-agent-core --test wave37_tool_surface

- 单线程模式（--test-threads=1）：**9 passed; 0 failed**，`test_exit=0`（t1/t2/t3/t4/t5/t6/t3_0/t3_h/t3_i/t3_j/t3_k/t3_l 全部绿；t1 为 executor-a C1 域，由其落地后转绿）。
- 默认并行模式：连续 5 次中 4 次 `9 passed; 0 failed, exit=0`，1 次出现 `t3_h` 偶发 FAILED（同进程并行用例争用 OnceLock 全局引擎槽所致，属测试并行隔离问题，重跑即绿；t3_j 曾有同类偶发，后续 3 次复跑均绿）。

### 已知残留（报 Lead 裁定）

- `tests/wave37_tool_surface.rs` 并行模式下存在偶发 flake（t3_h/t3_j，OnceLock 槽 + 共享 Stub 的进程级单例与并行用例争用）。测试文件对 task-4 只读，未改动；建议 T5 全量验证以 `--test-threads=1` 或由 T2 侧加串行标注（如 serial_test/单进程 gate）收口。

## 禁改核对

- present.rs / jobs.rs / git_search.rs / governed_executor.rs / provider：未触碰。
- wave-36 在飞文件（governed_executor.rs / retry.rs / plan_state.rs / dispatcher.rs）：未触碰。
- tools.rs：仅上列判定行，其余（C1/C2 executor-a 域）未触碰。
- git 写操作：无。
