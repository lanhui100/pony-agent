# PA-148: 用户级（全局）指令与 Skills 分层加载

## 状态
- **状态**: Review（实现与门禁完成，待 closeout 复核）
- **负责人**: Lead / Executor
- **创建日期**: 2026-10-21
- **实施日期**: 2026-10-21

## 需求背景
用户级偏好与用户级技能库此前完全不可见：`WorkspaceHotloader::resolve_constitution` 只在工作区内
取首个命中的 `AGENTS.md`（→ `CLAUDE.md` → `.claude/CLAUDE.md`），`scan_skills` 只扫描
`<workspace>/.agents/skills` 与 `<workspace>/.claude/skills`。对照
`docs/audit/agent-core-gap-matrix.md` A10（PARTIAL：只取工作区单个宪法文件、无用户级分层合并）。

## 核心交付契约
1. **C1 全局根集合**：`~/.agents` < `~/.claude` < `config_dir()/pony-agent` < `$PONY_AGENT_HOME`，
   纯逻辑解析 `resolve_global_agent_homes_from`（空 env 不产生兜底目录、重复路径去重）。
2. **C2 指令分层**：全局层按上述顺序在前，项目层最后（最高优先级）；每层根内按
   `AGENTS.md` → `CLAUDE.md` → `.claude/CLAUDE.md` 首命中；多层渲染带作用域与来源路径标题，
   **单一项目层保持原始字节**（既有提示词基线不变）。
3. **C3 技能分层**：全局根 `skills/` 在前、项目级 `.agents/skills`/`.claude/skills` 在后，
   同名（同目录名）项目级覆盖全局级；`HotloadedSkill.scope` 标注来源，提示词行标注 `[global]`/`[project]`。
4. **C4 缓存正确性**：`ConstitutionCache` 改分层结构（命中 = 层数/作用域/路径/mtime/size 全等）；
   技能逐文件缓存键改 `{scope}:{目录名}` + 合并结果独立缓存，避免「全局被同名项目技能覆盖」时缓存永不命中。
5. **C5 会话基线**：skills 哈希纳入 `scope`，作用域变化以 Append-Only Notice 通知，不改写头部静态前缀。
6. **C6 注入面同步**：`context.rs` 标题改为 `=== Workspace & User Instructions (AGENTS.md / CLAUDE.md) ===`。

## 验收收据
- `cargo test -p pony-agent-core --lib agent::workspace_hotloader`: 10 passed / 0 failed
  （含 `test_global_instructions_precede_project_instructions`、`test_project_skill_overrides_global_skill_with_same_folder_name`、
  `test_global_only_instructions_and_global_cache_invalidation`、`test_global_homes_order_and_dedupe`、
  `test_scan_skills_merges_global_and_project_layers`、`test_claude_compat_instruction_candidate_is_honored` 等）
- `cargo test -p pony-agent-core --test context_cache_red_contract`: 4 passed / 0 failed
- `cargo test -p pony-agent-core --test global_agent_config_contract`: 1 passed / 0 failed
  （走**默认入口**：仅设 `$PONY_AGENT_HOME` 指向临时目录，证明用户级 AGENTS.md 与用户级技能真实装配，且同名技能项目级覆盖全局级）
- `cargo test -p pony-agent-core --lib`: 1084 passed / 1 failed（`agent::config::tests::missing_capabilities_fields_use_stable_defaults`；
  在 `git worktree add /tmp/pony-baseline HEAD` 干净基线单跑复现同一断言，属基线既有红相，与本卡无因果）
- 格式化：`rustfmt --edition 2021 --check crates/pony-agent-core/src/agent/workspace_hotloader.rs` EXIT 0
  （`context.rs` 全仓存在预存 fmt 漂移，本卡只做 5 行标题替换，未整文件重排以免夹带）
- 基线红灯（非本卡引入）：`node scripts/check-rust-test-lints.mjs` FAIL，根因为 `src/bin/tui.rs` 的
  `unused import: supports_keyboard_enhancement` 与 `sync_state_with_registry is never used`（本卡未触达该文件）
- ADR: [`docs/decisions/0045-global-instructions-and-skills-layering.md`](../../docs/decisions/0045-global-instructions-and-skills-layering.md)

## 非目标
- 全局技能进入可执行面（capability registry）；执行面仍由控制平面快照 `apply_skill_source_snapshot` 决定。
- 技能数量上限/截断策略（当前宿主 89 + 44 个可全量列出，按证据再加限）。
