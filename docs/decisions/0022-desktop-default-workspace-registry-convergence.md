# 0022 桌面端默认工作区基座保持与兜底收敛注册表 default

Status: implemented

## 背景

桌面端默认工作区存在基座与兜底两层事实分叉：注册表 `default` 的基座为
Windows `%USERPROFILE%\Documents\pony_agent`（`dirs::document_dir`，尊重 OneDrive
重定向）、mac/Linux `~/pony_agent`（`dirs::home_dir`），见
`agent/workspace.rs::compute_default_workspace_root`；但五处兜底/回退锚定在
`std::env::current_dir`：`control_plane/mod.rs::default_workspace_root`、
`ToolRouter::new`、`build_governed_executor(None)`、`terminal.rs`/`jobs.rs` 的
`validate_and_resolve_cwd`、`session/store.rs::default_storage_path`。
`cwd` 随启动方式（终端/快捷方式/安装器）漂移，导致工作区跳变与安全边界漂移。

## 决策

1. **基座保持**：Windows `Documents\pony_agent`、mac/Linux `~/pony_agent` 不变；
   默认工作区永不使用裸 `Documents/~` 根（防污染用户文档、防 OneDrive 同步风暴）。
2. **子目录隔离保持**：`pony_agent` 子目录隔离符合 Obsidian vault / VS Code
   文件夹模式；AppData 只存内部 state/缓存，不放用户可见工作区。
3. **首次启动幂等创建、既有登记永不静默迁移**：`bootstrap_default_workspace`
   （缺则建、已有不动）即最佳实践，保持不动。
4. **兜底收敛注册表 default**：`get_workspace_root` 优先读
   `sessions.resolve_workspace_root(None)`，失败回退 `compute_default_workspace_root()`，
   `cwd` 仅作 `dirs` 全失败的最后回退；`ToolRouter::new` /
   `build_governed_executor(None)` / 存储链同例。
5. **穿越防护锚定会话 workspace**：`terminal.rs`/`jobs.rs` 新增
   `terminal_open_with_workspace_root` / `job_start_with_workspace_root`（`Some(ws)`
   钉住会话 root，`None` 走 `compute_default` 兼容链）；旧签名委托 `None` 保持兼容。

## 候选方案

- **裸 `Documents/~` 根作默认工作区**——落选：污染用户文档根，触发 OneDrive
  全量同步；业界（VS Code 从不默认裸 Documents 根、Obsidian 建议 Documents 下子目录）
  一致要求子目录隔离。
- **用户可见工作区进 AppData**——落选：Tauri `BaseDirectory` 显式区分
  AppData（内部状态）与 Document（用户内容）；Notion/Electron 系同样分离。
- **兜底保持 `current_dir`**——落选：`cwd` 随启动方式漂移，工作区跳变且
  `terminal/jobs` 安全边界可被启动目录操纵；Docker allowlist / VS Code workspace
  trust 均锚定配置而非 `cwd`。
- **既有 default 登记静默迁移到新基座**——落选：用户已有数据位置是既成事实，
  静默迁移造成"文件失踪"恐慌；首次启动幂等创建已覆盖新用户。

## 已知限制

- `terminal_open` / `job_start` 旧签名（无会话 root 入参）的 2 个 `cwd=None`
  用例保持旧行为（回退 `compute_default` 而非调用方会话 root）：调用方传入会话
  root 的接线留待后续任务；新签名已就绪（`TASK-2-WIRE` 目标）。
- `get_workspace_root` 毒锁恢复沿用本文件既有 `sessions` 读锁范式，
  再取 `runtime` 读锁；两段式不嵌套持锁，无新增死锁面。

## 影响

- 行为：`get_workspace_root`、`ToolRouter::new`、`governed(None)`、
  `terminal/job` 缺省 `cwd` 从"进程启动目录"变为"注册表 default /
  `~/pony_agent`"；已登记 `default` 的老用户无感知（读注册表优先）。
- 测试：`crates/pony-agent-core/tests/default_workspace_red_phase.rs`
  7 用例中 5/7 转绿（含契约 1+2 全绿、2 个跨界拒绝连带转绿），2 个旧签名
  `none` 用例保持红（已知限制）。
- 回归：`agent::workspace` 18/18、`agent::session` 104/104、
  `agent::control_plane` 62/62 全绿；`cargo check -p pony-agent-core` 零警告。
