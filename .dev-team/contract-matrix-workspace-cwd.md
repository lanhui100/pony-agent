# 契约矩阵 —— 工作区 cwd 一致性 + 默认工作区修复（参考 DSH 实现）

## 问题与 DSH 对照（已冻结）
- P1（问题③）：模型环境提示 env note 的 `cwd` 取进程启动目录（打包 app = AppData 目录），而 Run/读写工具按会话 workspace root 执行 → 提示与实际不一致，误导模型。
  DSH 对照：会话工作目录 = 用户所选 workspace 路径（SessionHeader.cwd 创建时确定），模型提示从不用进程 cwd 冒充工作区。
- P2（默认工作区）：`bootstrap_default_workspace_with_base` / `default_workspace_root()` 在 `dirs::document_dir()`（Windows）或 `dirs::home_dir()`（Unix）解析失败时静默回退**进程 cwd** 并**持久化**为"默认工作区"→ 安装包进程 cwd=AppData → 首次启动默认工作区错误。
  DSH 对照：`initializeDefault` 固定目录名 + Documents 解析；解析失败 = ineligible → 不注册、返回 undefined，目录选择留给用户；目录缺失递归创建。

## 修复契约
### R1（env note 真实性，核心）
- 新增 `collect_env_info_for_workspace(root: &Path) -> EnvironmentInfo`（cwd=root；is_git_repo/branch 基于 root 判定；其余字段同 collect_env_info）。
- `snapshot_from_state`（session/store.rs:2724）新增参数 `workspace_cwd: Option<String>`；4 个调用点（348/2607/2610/2721）由 SessionStore 方法解析后传入。
- 会话 workspace root 解析链：`resolve_workspace_root(&self.workspaces, session.workspace_id)`（默认 id=无 → 默认注册项）→ Err 时 `compute_default_workspace_root()` → None 时进程 cwd（最后兜底，瞬时）。
- `collect_env_info()` 保持原样（其他调用点/测试不变）。

### R2（默认工作区解析强化）
- `compute_default_workspace_root()` 解析链（纯逻辑，不 IO，目录创建由 bootstrap 负责）：
  - Windows：`dirs::document_dir()` → `dirs::home_dir() + "Documents"` → `dirs::home_dir()` → None
  - Unix：`dirs::home_dir()` → None
- 新增可注入纯函数（便于确定性测试）：`resolve_default_workspace_base(document_dir: Option<&Path>, home_dir: Option<&Path>) -> Option<PathBuf>`（返回 base，最终 root = base.join("pony_agent") 语义不变）。
- `bootstrap_default_workspace_with_base`：base 解析失败（None）或 create_dir_all 失败 → **返回 false 且不注册**（替换现 process-cwd 回退）；base_override Some 行为不变；幂等不变。
- `default_workspace_root()`（control_plane/mod.rs:60）最后一级瞬时回退进程 cwd 保留（不持久化）。

## 验收测试契约（红相先行，test-agent 编写）
- E1: 会话 snapshot 的 env_info.cwd == 会话自定义 workspace root（注册表有记录）。
- E2: 会话无 workspace_id → env_info.cwd == 默认注册项 root。
- E3: 注册表空 → env_info.cwd == compute_default_workspace_root()（非进程 cwd）。
- E4: resolve_default_workspace_base 纯函数：Windows 形态（doc None + home Some → home/Documents/pony_agent）；doc Some → doc/pony_agent；全 None → None；Unix 形态（home Some → home/pony_agent）。
- E5: bootstrap 不可解析（注入 None base + 空解析结果）→ false 且 records 为空；可解析 → true 且 root=base/pony_agent（替换旧测试 bootstrap_falls_back_to_process_cwd_when_base_unresolvable 的断言方向）。
- E6: 回归：base_override Some 注册 + 幂等（现有 workspace.rs 测试保持绿）。
- E7: 回归：tests/default_workspace_red_phase.rs 集成测试保持绿（ToolRouter::new/governed_executor 兜底 == compute_default_workspace_root()）。
- 门禁：`RUSTFLAGS=-D warnings cargo check --lib --package pony-agent-core` 通过；靶向子集全绿。

## 范围边界（Non-Goals）
- 不改前端（前端不渲染 envInfo）。
- 不动 run_command/批处理（已完成）。
- 不迁移既有 default 登记（用户既有登记永不静默迁移，沿用）。
- 不新增第三方依赖（std + 现有 dirs）。
