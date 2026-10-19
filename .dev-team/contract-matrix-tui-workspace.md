# 契约矩阵 —— TUI 启动目录即默认工作区（wave-38，Lead 冻结）

## 0. 事实基线（Lead 只读侦察，path:line 可复核）
- `crates/pony-agent-core/src/bin/tui.rs:259-272`：`TuiState::default()` 的 `workspace` = `canonicalize(current_dir)`，**仅用于 header 展示**。
- `tui.rs:1151-1189` `build_control_plane`：`SessionStore::new()` + `AgentRuntimeBuilder::desktop().workspace_root(cwd)`。
- `crates/pony-agent-core/src/agent/session/store.rs:190-192`：`SessionStore::with_backend` 调 `bootstrap_default_workspace`，默认工作区 = `~/pony_agent`（Unix）。
- `tui.rs:3280-3310`：TUI 发送 turn 时 `TurnInput.workspace_id: None`，会话从未盖章。
- `crates/pony-agent-core/src/agent/runtime/mod.rs:244-306` `apply_governed_turn_context`：`workspace_id` 为空 → 不盖章；`stamped_owner = None` → governed root = `self.workspace_root`（cwd）。
- `crates/pony-agent-core/src/agent/runtime/turn_sync.rs:59 / turn_stream.rs:186`：工具执行根 = `resolve_session_workspace_root(sid)`；`runtime/mod.rs:309-323` 中 owner `None` → `resolve_workspace_root(None)` → **注册表默认项 `~/pony_agent`**。
- `crates/pony-agent-core/src/agent/session/store.rs:330-345` `workspace_env_cwd`：env note 的 cwd 同样走注册表默认项 → `~/pony_agent`。
- `crates/pony-agent-core/src/agent/control_plane/mod.rs:1249-1268` `get_workspace_root()`：优先注册表 default（`~/pony_agent`），runtime root 仅次级回退。

**结论（根因）**：TUI 把 cwd 只写进 header 与 runtime builder 种子，但会话没有 workspace 归属，工具执行/env note 统一走注册表默认工作区 `~/pony_agent` → 启动目录不是工作区。

## 1. 修复契约（R1，本期唯一实现范围）

### R1.1 纯函数（`agent/workspace.rs`）
```rust
/// 确保 root 在注册表中（幂等）：已存在同 root 记录 → 原样返回；否则登记新记录。
pub fn ensure_workspace_for_root(
    records: &mut Vec<WorkspaceRecord>,
    root_path: &str,
) -> Result<WorkspaceRecord, String>;
```
- root 规范化复用 `normalize_workspace_root`（canonicalize + 必须是目录 + Windows 去 `\\?\`）。
- 同 root 已存在（`roots_match`）→ 返回既有记录，`records` 不变（幂等，不写盘）。
- 新登记复用 `create_workspace_entry`（id 规则/名称校验单源），显示名 = canonical root 的 `file_name`：
  - 空/非 UTF-8/根路径（如 `/`）→ `"workspace"`；
  - 截断到 ≤64 字符（按 char）；
  - 与既有记录重名（不同 root）→ 依次追加 `" (2)"`、`" (3)"`… 最多 99 次；全部占用 → `Err`。
- 不改动既有 default 记录、不做任何迁移。

### R1.2 SessionStore 写路径（`agent/session/store.rs`）
```rust
pub fn ensure_workspace_for_root(&mut self, root: &str) -> Result<WorkspaceRecord, String>;
```
- 调 R1.1；仅当**新增**记录时 `save_to_backend()`（幂等路径零写盘）。

### R1.3 HostControlPlane 入口（`agent/control_plane/workspace_commands.rs`）
```rust
pub fn ensure_workspace_for_root(&self, root: &str) -> Result<WorkspaceRecord, String>;
```
- 与 `create_workspace` 同锁纪律（`sessions_rwlock.write()` + poison 恢复）。

### R1.4 TUI（`src/bin/tui.rs`）
- `TuiState` 新增字段 `pub active_workspace_id: Option<String>`，`Default` 为 `None`。
- `main`：构造 `control_plane` 之后、进入 event loop 之前：
  ```rust
  state.active_workspace_id = control_plane
      .ensure_workspace_for_root(&state.workspace.display().to_string())
      .map(|record| record.id)
      .map_err(|error| eprintln!("[pony-tui] 启动目录登记为 workspace 失败：{error}"))
      .ok();
  ```
  失败不 panic、不退出，回退为 `None`（保持既有默认工作区行为）。
- TUI 发送 turn 时 `TurnInput.workspace_id = state.active_workspace_id.clone()`（`start_turn_stream` 唯一发送点）。
- 可测接缝：抽纯函数
  ```rust
  pub fn build_turn_input(state: &TuiState, message: String, provider_id: String, model_id: String) -> TurnInput
  ```
  其余字段保持现状（`display_message: None`、`reasoning_effort: state.reasoning_effort.clone()`、`workspace_mode: None`、`session_id`、`node_id: None`、空 history/images、`plan_execution_approval: None`），发送点改用该函数。

### R1.5 语义边界（非目标 Non-Goals）
- **不改** `bootstrap_default_workspace` / `compute_default_workspace_root` / `get_workspace_root` 的语义与默认登记（桌面端默认工作区仍是 `~/pony_agent`，既有 default 永不迁移）。
- **不改** 前端、`ponycli`（`src/bin/cli.rs`）、Tauri 适配层。
- **不改** 会话 stamping/normalize 的既有核心语义（TUI 通过 `workspace_id` 走既有盖章链；恢复旧会话后再发送会按 TUI cwd 盖章，属显式语义，文档记录）。
- 不新增第三方依赖；不重构无关代码。

## 2. 验收契约（红相先行，Test Agent 编写，Executor 只读）

### L2-T 功能契约（Rust 集成测试 `crates/pony-agent-core/tests/wave38-tui-workspace-contract.rs`）
- **E1** `ensure_workspace_for_root` 新 root：返回记录 root == `normalize_workspace_root(root)`，id 合法（`ws-` 前缀或 default 规则），records 长度 +1。
- **E2** 幂等：同 root 二次调用返回同 id，records 长度不变。
- **E3** 既有同 root 记录（名称不同）→ 原样返回，不新增、不改名。
- **E4** 同名不同 root：第一个占名，第二个自动去重为 `name (2)`（或 ` (3)`），两者 id 不同；所有记录名 ≤64 char。
- **E5** 非法 root（不存在/不是目录/空串）→ `Err`，records 不变。
- **E6** `TuiState`：`build_turn_input` 在 `active_workspace_id = Some("ws-demo-1")` 时产出 `TurnInput.workspace_id == Some("ws-demo-1")`；为 `None` 时为 `None`；`session_id`/`message`/`provider_id`/`model_id` 正确透传。
- **E7** 回归：`TuiState::default().workspace` == `canonicalize(current_dir)`。

### L2-AT 对抗（同脚本第二段 / 独立用例）
- **AT1** 只读 HOME（`HOME` 目录 chmod 只读或指向不可写）：`ensure_workspace_for_root` 落盘失败必须返回 `Err` 或幂等成功，**不得 panic**；TUI 启动仍进入 alt screen 且不崩溃（无 101）。
- **AT2** 同名目录对抗：两个不同 root 同名（`a/proj`、`b/proj`）并发/连续 ensure → 名称去重确定性（顺序执行两次），无重复 id、无重复名。
- **AT3** 超长/含怪异字符目录名（≥100 char）：登记后名称 ≤64 char 且 root 正确。

### 真实 TUI E2E（物理收据，`tests/acceptance/l2at-tui-workspace.sh`）
- 隔离 `HOME`/`XDG_*`，mock OpenAI SSE server（脚本内自包含，不依赖 tools/ 未跟踪文件），provider 注册表指向 mock。
- 场景：`$ISO/project/package.json` 写入唯一标记 `TUI_WORKSPACE_MARKER_<uuid>`；`$ISO/home/pony_agent/package.json` 写入 `DEFAULT_WS_MARKER_<uuid>`；在 `$ISO/project` 下用 tmux 启动 `pony`，发送触发 mock 返回 `Read package.json` tool_call 的提示词。
- 断言：屏幕出现 `TUI_WORKSPACE_MARKER_`（工具读到 cwd 文件），且**不出现** `DEFAULT_WS_MARKER_`；真实 stdout/日志落盘 `tests/acceptance/receipts/l2at-tui-workspace/`。
- 环境缺失（无 tmux/script/mock 端口）→ 显式 `[RED DEGRADED: STATIC-ONLY]` 并记录，不得静默放行。

## 3. 门禁
- 红相：`cargo test -p pony-agent-core --test wave38-tui-workspace-contract` 按 `unimplemented!()` 失败（EXIT≠0），E2E 脚本失败；Lead 打红相锚定 commit（仅测试文件干净路径）。
- 绿相：Rust 契约测试全绿；`RUSTFLAGS="-D warnings" cargo check -p pony-agent-core --lib --bins` EXIT=0；E2E 脚本 EXIT=0 并附物理收据。
- 写域审计：Executor 不得修改测试文件；`git diff --name-only` 限于本契约声明范围（含 `tui.rs` 内既有未提交无关改动，提交时只暂存本任务 hunk）。
- 环境事实：工作区存在**他人未提交的 /thinking 相关改动**（`tui.rs`、`config.rs`、`Cargo.lock`）。本任务绝不提交这些改动，也绝不回滚/覆盖它们；所有 git 提交使用定向暂存。

## 4. NFR 基准（wave-38 增量，不覆盖冻结基线）
- 既有 `.dev-team/nfr-baseline.json`（PA-118 16 键冻结基准 + schema）**不修改**；实跑 `dev-nfr-lint.py` 取 EXIT 0。
- 本期增量 `.dev-team/nfr-baseline-wave38.json`（5 必填：`external_call_timeout_ms`≤3000 / `retry` / `concurrency_lock` / `logging`(json+trace_id) / `resource_release`），实跑 `dev-nfr-lint.py` EXIT 0。
