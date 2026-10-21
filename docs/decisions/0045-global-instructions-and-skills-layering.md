# ADR 0045：用户级（全局）指令与 Skills 分层加载

Status: implemented

## 背景

`WorkspaceHotloader`（`crates/pony-agent-core/src/agent/workspace_hotloader.rs`）此前只识别**工作区单层**项目指令：

1. `resolve_constitution` 在 `<workspace_root>` 内按 `AGENTS.md` → `CLAUDE.md` → `.claude/CLAUDE.md` 取**首个命中**即停，用户级偏好（如「默认中文回答」「提交前先跑定向测试」）只能复制进每个项目；
2. `scan_skills` 只扫描 `<workspace_root>/.agents/skills` 与 `<workspace_root>/.claude/skills`，用户级技能库（`~/.agents/skills`、`~/.claude/skills`、产品配置根）完全不可见；
3. 对照 audit（`docs/audit/agent-core-gap-matrix.md` A10）记录为 PARTIAL：「只取工作区单个宪法文件，无用户级/企业级分层覆盖与优先级合并」。

## 决策

1. **用户级根目录集合（全局层，优先级从低到高）**：
   1. `~/.agents`（跨 agent 共享根，与项目级 `.agents` 约定对称）
   2. `~/.claude`（Claude Code 兼容根，与项目级 `.claude` 约定对称）
   3. `config_dir()/pony-agent`（产品自有全局根，与 `settings.json` 同根）
   4. `$PONY_AGENT_HOME`（显式覆盖，最高）

   纯逻辑解析入口 `resolve_global_agent_homes_from(env_home, config_dir, home_dir)`，空环境变量不产生兜底目录，重复路径按首次出现去重。
2. **指令分层合并**：每个全局根内按既有候选顺序取首个存在者（`AGENTS.md` → `CLAUDE.md` → `.claude/CLAUDE.md`），全局层按优先级从低到高排列，**项目层永远在最后（最高优先级）**。多层时每层加 `## Global user-level instructions (<path>)` / `## Project instructions (<path>)` 标题；**仅存在单一项目层时返回原始字节**，既有提示词形态与历史基线不变。
3. **缓存语义**：`ConstitutionCache` 从单文件改为 `layers: Vec<ConstitutionLayer>`；命中判据 = 层级数量、作用域、路径与（mtime, size）全部一致，命中时不重新读盘；层内文件消失只跳过该层，不丢弃整条链路。
4. **Skills 分层合并**：扫描顺序即优先级——全局根 `skills/` 依次在前，项目级 `.agents/skills`、`.claude/skills` 在后；**同名（同目录名）时项目级覆盖全局级**。`HotloadedSkill` 新增 `scope: AgentScope`，逐文件缓存键改为 `{scope}:{目录名}`，合并结果另存为独立缓存以支持「全局被项目同名覆盖」时的正确命中判定。
5. **提示词可见性**：`build_cached_prompt_prefix` 的技能行标注来源，形如 ``- `audit` [global]: ...``；会话基线 skills 哈希把 `scope` 纳入输入，作用域变化（全局→项目晋级）会触发增量 Notice 而不是静默改写头部。
6. **注入点同步**：`context.rs` 的 semistable 段标题由 `=== Project Instructions (AGENTS.md) ===` 改为 `=== Workspace & User Instructions (AGENTS.md / CLAUDE.md) ===`，避免全局层存在时标题失真。

## 备选方案与取舍

- **只认产品自有根 `config_dir()/pony-agent`**：语义最干净，但现有用户在 `~/.agents`、`~/.claude` 的存量技能库无法复用，等于要求迁移；放弃。
- **只认跨 agent 根 `~/.agents` / `~/.claude`**：把产品行为绑定在第三方约定上，且产品配置根（`settings.json` 旁边）无处安放自身规则；放弃。
- **全局与项目指令同目录内「共存并全部加载」（如 DSH 对 `AGENTS.md` + `CLAUDE.md` 的处理）**：会在项目层引入双真源冲突，本仓库既有「首个命中即停」语义被大量提示词基线依赖；保持单层首命中，只在层间做优先级合并。
- **一次性把全局技能注入 capability registry（可执行面）**：全局技能的**执行**仍由控制平面快照（`apply_skill_source_snapshot`）决定，本 ADR 只解决**可见性**；擅自扩大可执行面属于权限面变更，需独立 ADR。
- **对全局技能设置数量上限**：宿主实测 `~/.agents/skills` 89 个 + `~/.claude/skills` 44 个可全部列出（每技能一行），暂不需要截断；等出现性能或注意力回归再按证据加限。

## 后果

- `crates/pony-agent-core/src/agent/workspace_hotloader.rs`：新增 `AgentScope`、`ConstitutionLayer`、`resolve_global_agent_homes{,_from}`、`resolve_constitution_with_globals`、`scan_skills_with_globals`、`resolve_session_prompt_context_with_globals`；`ConstitutionCache` 改为分层结构；`HotloadedSkill` 增 `scope` 字段。
- `crates/pony-agent-core/src/agent/context.rs`：注入标题改为 Workspace & User Instructions。
- `crates/pony-agent-core/tests/global_agent_config_contract.rs`：以 `$PONY_AGENT_HOME` 驱动**默认入口**的契约测试（用户级指令装配 + 用户级技能发现 + 同名项目覆盖）。
- 默认行为变化（用户可见）：宿主机存在 `~/.agents/AGENTS.md`、`~/.claude/CLAUDE.md`、`~/.config/pony-agent/AGENTS.md` 或 `$PONY_AGENT_HOME/AGENTS.md` 时，其内容会进入每轮上下文；同名技能列表会多出 `[global]` 条目。
- 可测性：所有新入口均可注入 `global_homes`，单测不触碰真实用户目录。
