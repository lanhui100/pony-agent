# PA-069-A 工具文件 IO 从 runtime 锁内移出

## Basic Info
- ID: PA-069-A
- Status: Done
- Priority: P1
- Owner: @agent
- Created At: 2026-06-25
- Updated At: 2026-08-08（实现验证通过并收口）
- Estimated Effort: 4-6h

## Goal
将 `execute_registered_tool_call` 从 `runtime.lock()` 持有范围内移出，使大文件搜索/读取不阻塞其他控制面操作。

## Problem Analysis
调用链：
```
runtime.lock()                         ← control_plane.rs 持有
  └─ start_turn_stream_with_control()  ← runtime/mod.rs
       └─ handle_stream_tool_turn()    ← ~500 行紧密耦合循环
            └─ execute_registered_tool_call()  ← 内含同步文件 IO
                 └─ ToolRouter::execute()      ← read_file/write_file/search_text
```

`spawn_blocking` 和 `tokio::fs` 在持有 `MutexGuard` 时均无效——锁仍然被持有，调用线程仍被阻塞。

## Implementation Plan

### Phase 1: Extract tool execution pre-requisites from lock scope

**Problem**: `execute_registered_tool_call` needs `&mut self` (AgentRuntime) for:
1. `self.capability_registry.resolve_tool_call(tool_call)` — read-only, safe
2. `self.tool_executor.execute(&action.tool_call)` — the actual IO
3. `self.capability_registry` for skill resolution — read-only

**Solution**: Pre-resolve the tool call, extract necessary state, drop the runtime lock, execute, re-acquire.

### Phase 2: Restructure handle_stream_tool_turn

The loop at `runtime/mod.rs:2444-3478` does:
```
loop {
    // self.update_execution_checkpoint() ← needs &mut self
    // self.execute_registered_tool_call() ← needs &mut self (but tool executor is the real IO)
    // self.dispatch_hook_trace_records() ← needs &self
}
```

**Strategy**: Split `AgentRuntime` to expose `tool_executor: Arc<dyn ToolExecutor>` or use a per-turn context that owns the tool executor separately.

### Phase 3: Minimal change approach

Minimal risk: Add a `tool_executor` Arc to `AgentRuntime` that can be extracted:
```rust
// AgentRuntime already has: tool_executor: Box<dyn ToolExecutor>
// Change to: tool_executor: Arc<dyn ToolExecutor + Send + Sync>
```

Then in `handle_stream_tool_turn`:
1. Clone tool_call + extract tool_executor Arc (while holding runtime lock)
2. Drop runtime lock
3. Execute tool (may block, but no lock held)
4. Re-acquire runtime lock for state update

## Current Progress
- 已完成并收口（2026-08-08）。
- 实现形态：`AgentRuntime` 拆分出 `TurnContext`（`runtime/turn_runner.rs:19-27`），工具执行经 `tool_executor: Arc<dyn ToolExecutor + Send>` 独立执行；`execute_registered_tool_call` / `execute_capability_tool_call` 落在独立模块 `runtime/tool_exec.rs`，通过 `self.tool_executor.execute(&action.tool_call)` 执行，不持 runtime 锁。
- 代码验证：`grep runtime\.lock()` 在 `crates/pony-agent-core/src` 生产路径无匹配。

## Next Action
- 无。已完成收口。

## Resume Hint
- 核心文件：`runtime/mod.rs:2444-3478` (`handle_stream_tool_turn`)、`runtime/mod.rs:5572-5620` (`execute_registered_tool_call`)
- 关键约束：`execute_registered_tool_call` 需要 `&mut self` 但真正需要的是 `tool_executor` + `capability_registry`（只读）
