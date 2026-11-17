# Agent Note: 技能根读取默认放行——技能是产品只读资产，审批只留给越界写与未知外部路径

Status: implemented

## Problem

模型读取**全局技能**（`~/.agents/skills/<name>/SKILL.md`、`~/.claude/skills/...`）时会弹出工作区外路径审批，整轮挂起等人点确认；审批面板第一行还把运行时身份 `builtin:workspace_read_file` 当工具名上屏（工程信息，对人无意义），说明行与目标行重复同一路径。现场由 B234 记录：`skill_load{name:dev-team}` 返回 `unknown_skill` 后，模型退化为对全局技能正文裸 `Read`，于是撞上 `path_permission::classify` 的「工作区外读取一律要授权」并挂起。

关键事实：技能根（全局 `~/.{dsh,agents,claude}/skills`、`$PONY_AGENT_HOME/skills`、自定义根，以及仓库/会话根的 `.dsh|.agents|.claude/skills`）是**产品自身扫描、加载、注入提示词的只读资产**，用户从未把它们当作需要逐个授权的数据目录。把「技能可加载」留在 `skill_load` 的白名单里、却让同一条路径经 `Read` 变成审批，是同一事实两套判定（P1-6「列得出、载不进」的镜像形态：**列得出、读不进**）。

## Decision

1. **读用途按受信技能根默认放行**：`path_permission::classify` 的 `PathPurpose::Read` 分支在 workspace/tmp 之后、外部授权之前，新增受信技能根判定，命中返回新分区 `PermissionZone::TrustedSkillRoot`（`as_str = "trusted_skill_root"`）。
2. **分域策略与加载侧逐字一致**：复用 `trusted_skill_roots_with_policy`（与 `skill_body` 的加载白名单同一枚举、同一 `allow_symlink_escape` 语义）——
   - 全局根（用户自有域，`allow_symlink_escape = true`）：按**词法**归属判定，允许技能目录/文件为符号链接并解析到根外（`npx skills` 与 Claude Code 的标准安装形态，Wave-58 D1）；
   - 项目级根（仓库内容不可信，`allow_symlink_escape = false`）：只认 **canonical 包含**，`<ws>/.claude/skills -> /tmp/evil` 整根不采纳，逃逸目标仍走审批。
3. **只放宽读**：`PathPurpose::Write` 完全不变（技能目录不是可写域，越界写仍 `outside_workspace_write_denied`）；非技能路径的外部读仍 `requires_authorization`。
4. **工具层放行新分区**：`tools::paths::classify_workspace_path` 把 `TrustedSkillRoot` 与 workspace/tmp 同列放行（该函数原本只认 `AuthorizedExternal` 之外的三个分区）；`governed_executor` 的审批门禁因 classify 已返回 `Ok` 而自然不再挂起，无需新增旁路。
5. **审批面板去工程信息、去重**：
   - 工具名只取结构化 `options.tool`（模型名，如 `Read`）；仅当来源不是内建工具（MCP/插件，id 本身是产品可见工具名）才用 `descriptorId` 兜底，`builtin:` 前缀的运行时身份**永不上屏**；
   - 宿主审批文案改为「读取工作区外路径：<path>」/「该工具调用需要审批后才能执行（策略或外部来源要求）。」——工具身份由标题单独呈现，正文不再重复工具名；
   - TUI 面板在说明已包含目标路径时不再渲染重复的「目标」行（高度计算与渲染共用同一判定 `show_target_line`）。

## Alternatives considered

- **只修 B234 的 `skill_load` 解析，不放开 `Read`**：能消除本次退化路径，但「全局技能正文可读」仍依赖模型猜对工具；用户手动 `/skill` 或模型读技能附带的 `references/*.md` 时同样会撞审批。根因是「技能资产被当成未知外部路径」，只在入口补一层仍会复发。否决。
- **把技能根并入 workspace 白名单分区（复用 `WorkspaceRoot`/`ControlledTmp`）**：`PermissionZone` 会谎报来源（技能读被记为 `workspace_root`），审计与后续按分区收紧（例如只读域再细分）失去抓手。改为新增 `TrustedSkillRoot`，语义与可观测面都可分辨。否决。
- **读放行只认 `SKILL.md` 文件名（直接复用 `is_within_trusted_skill_roots`）**：最保守，但技能正文常引用同目录 `references/*.md` / 模板；用户看到的仍是「读技能目录还要审批」。技能根本身就是只读资产域，按目录放行收益更大。否决。
- **全局根也要求 canonical 包含（拒绝一切符号链接逃逸）**：与 Wave-58 D1 冲突——`npx skills` 的标准安装形态就是符号链接，会把大量正常安装打成每次必审批。维持「全局按词法、项目按 canonical」的分域策略。否决。
- **在 TUI 面板本地剥掉 `builtin:` 前缀再显示**：会让 `workspace_read_file` 这类内部原语名出现在用户面前，仍是工程信息；正确做法是从结构化 `options.tool` 取人类可读名，缺失时宁可不显示。否决。

## Consequences

- 行为变化（用户可见）：模型/用户读取全局或项目技能根内的文件（`SKILL.md`、技能自带参考文件）不再触发审批；越界写、非技能路径读的审批面逐字不变。
- 安全边界：项目技能根仍 fail-closed（符号链接逃逸整根不采纳）；全局根沿用 D1 的「用户自有域」语义，不新增跨域能力（此前 `skill_load` 已能读同一路径）。
- B234 本体（`skill_load` 对全局技能返回 `unknown_skill`）**未在本条修复**：那是解析/扫描侧缺陷，仍需独立定位；本条只消除它造成的「裸 Read 挂起」后果。
- 回归口径：
  - `cargo test -p pony-agent-core --lib path_permission`（新增 `read_within_project_skill_root_is_trusted_but_write_is_not` / `project_skill_root_symlink_escape_still_requires_authorization` / `skill_root_read_policy_matches_load_side_domains`）；
  - `cargo test -p pony-agent-core --bin pony wave43_unit_tests`（新增 `permission_request_uses_human_tool_name_not_builtin_descriptor_id` / `permission_panel_hides_duplicate_target_when_description_carries_path`）；
  - 集成面：`approval_resume_e2e` / `permission_fail_closed_contract` / `tui_interaction_acceptance` / `tui_interaction_adversarial` / `tui_task_visibility` / `b223_write_denied_hint_acceptance`。
- 变更面：`crates/pony-agent-core/src/agent/path_permission.rs`、`crates/pony-agent-core/src/agent/tools/paths.rs`、`crates/pony-agent-core/src/agent/dispatcher.rs`、`crates/pony-agent-core/src/bin/tui/projection.rs`、`crates/pony-agent-core/src/bin/tui/permission_panel.rs`。

## Related

- B236（本条）；B234（`skill_load` 全局解析缺陷，未修）
- `implemented/bug-fix/2026-11-11-skill-root-discovery-scope.md`（技能根单一枚举与分域信任的前案）
- `implemented/architecture/2026-11-17-sandbox-denied-no-unbacked-escalation.md`（越界写文案与审批落点的前案）
