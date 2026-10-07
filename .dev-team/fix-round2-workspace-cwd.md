# workspace-cwd 轮：review findings 分诊与修复契约（Lead 冻结）

> 来源：`.dev-team/review-design-workspace.json`（verdict FAIL，9 findings）。
> 本文件先做**事实分诊**（对照 `5b9ebfc` 实际代码），再冻结残余修复契约。

## 1. 事实分诊（Lead 已逐条核对当前代码）

| Finding | 严重度 | 当前状态（`5b9ebfc`） | 处置 |
| --- | --- | --- | --- |
| R1-1 | 高 | **已修复**：`default_snapshot_for_session(session_key, node_id, self.workspace_env_cwd(None))`（store.rs:361）已穿入 workspace_cwd | 回归验证即可 |
| R1-2 | 高 | **部分修复，仍有残余**：`session_workspace_cwd`（types.rs:463）死 id→默认注册项；但 `stamp_workspace_id` 仅在 `workspace_id == None` 时盖章（死 id 会话为 no-op），runtime `apply_governed_turn_context` 对死 owner 走 fail-closed → **env note 声称默认 root、工具拒绝执行** | **本轮修复**（W1） |
| R2-1 | 高 | **已修复**：`resolve_default_workspace_base(document_dir, home_dir, windows: bool, path_exists)` 平台参数化 | 回归验证 |
| R2-2 | 中 | **已修复**：注入 seam `bootstrap_default_workspace_inner(records, resolved_base)`（None→false 不注册） | 补 E5 验收测试 |
| R2-3 | 中 | **已修复**：`home/Documents` 仅在 `path_exists` 时采用 | 回归验证 |
| R2-4 | 中 | **部分修复**：`control_plane::default_workspace_root()` 已有 eprintln 告警；`ToolRouter::new`（tools.rs:1019-1024）仍**静默**回退进程 cwd | **本轮修复**（W2） |
| R2-5 | info | **已满足**：E3 可测——`session_workspace_cwd` 为纯函数，测试可直接传 `&[]` 空注册表（tests.rs:2529 已存在该断言） | 回归验证 |
| R1-3 | info | 已知语义：env note 的 git 判定为 workspace root 浅查（仓库子目录会显示 not a git repo） | **仅补注释记录**（W3） |
| R1-4 | info/PASS | 审查已证伪自身假设（env_info 不冻结、无漂移） | 无动作 |

## 2. 本轮修复契约

### W1（核心，R1-2）：会话归属死 id 归一收敛为单一真相源
- 在 `crates/pony-agent-core/src/agent/workspace.rs` 新增纯函数：
  `pub fn normalize_session_workspace_id(workspaces: &[WorkspaceRecord], id: &str) -> String`
  —— `id == DEFAULT_WORKSPACE_ID` 或已注册 → 原样返回；否则 → `DEFAULT_WORKSPACE_ID`（保持既有
  控制字符压平与 80 字符截断日志口径）。
- `SessionStore::stamp_workspace_id`（store.rs:2012）改为复用该 helper（消除内联重复），**并放宽
  触发条件**：`workspace_id` 非空且经归一后与当前值不同 → 写回归一值并落盘（死 id 自愈）。
- runtime `apply_governed_turn_context`（runtime/mod.rs）：会话 owner 为死 id 时，**按归一后的
  default id 解析 root**（与 env note 同源），不再 fail-closed；仅当 default id 本身也解析不到时
  才保持既有 fail-closed（None → 工具层阻断）。
- **不变量保持**：PA-080 的 fail-closed 边界不变——**调用方显式传入**的未注册 `workspace_id`
  （工具链 `resolved_workspace_root`）继续返回 `invalid_workspace`；本轮只统一**会话归属**链。

### W2（R2-4）：ToolRouter::new 进程 cwd 兜底告警
- `ToolRouter::new` 的 `compute_default_workspace_root()` 为 None 时，与 control_plane 一致输出
  eprintln 告警（措辞对齐既有 control_plane 告警：瞬时兜底、不持久化）。

### W3（R1-3）：已知语义记录
- `collect_env_info_for_workspace` 文档注释补记：git 判定为 workspace root 浅查（不向上寻仓），
  仓库子目录场景下 env note 显示 not a git repo 属已知语义（与 DSH SessionHeader.cwd 粒度对齐）。

## 3. 验收测试契约（红相先行，Test Agent 编写）

- **F1**：`normalize_session_workspace_id` 纯函数矩阵——default id 原样、已注册 id 原样、
  死 id → default、空白 id → default、超长/控制字符 id → default 且日志截断。
- **F2**：`session_workspace_cwd` 与 `normalize_session_workspace_id` 语义一致——同一死 id 在
  env note 链与归属归一链得到同一 default 解析结果（跨函数一致性断言）。
- **F3**：`stamp_workspace_id` 对**已存在的死 id 会话**执行归一写回并落盘（自愈用例），
  且对已注册 id 幂等 no-op。
- **F4**：`bootstrap_default_workspace_inner(&mut records, None)` → false 且 `records` 为空（E5）。
- **F5**：`resolve_default_workspace_base` Windows 形态在 Linux 可测（`windows=true` 参数化）：
  doc Some → doc；doc None + Documents 存在 → home/Documents；doc None + Documents 不存在 → home；
  全 None → None；Unix 形态（`windows=false`）home Some → home。
- **F6**：`ToolRouter::new` 默认 root 解析为 None 时输出告警（可注入 seam 或断言兜底值语义——
  若无法确定性注入，改为断言 `compute_default_workspace_root` 为 None 时的 eprintln 文本存在）。

## 4. 门禁
- `RUSTFLAGS="-D warnings" cargo check -p pony-agent-core --lib` 通过
- 靶向：`cargo test -p pony-agent-core --lib 'workspace::'`、`'session::'`、`'runtime::'`（串行）、
  `--test default_workspace_red_phase`、`--test session_regression` 0 failed
- 既有 PA-080 路径与本轮外失败不扩散（沿用既有范围外清单）
