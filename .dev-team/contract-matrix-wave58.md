# Wave-58 验收契约矩阵：上下文构建与 Skills / AGENTS.md 加载修复

> Lead 冻结。Executor 只读，Test Agent 依此编写红相，Reviewer 依此裁决。
> 上游审查报告：`management/task-system/02_REVIEWS/2026-11-01-context-skills-loading-audit.md`

## 0. Lead 已裁决的设计决策（不可由下游自行变更）

- **D1（技能符号链接信任边界）**：分域授权。
  - 用户级全局根（`~/.agents`、`~/.claude`、`$PONY_AGENT_HOME`、`config_dir/pony-agent`）下的技能目录允许为符号链接并解析到根外（`npx skills` / Claude Code 标准安装形态），但**仍须**满足：无 `..` 组件、canonical 目标是普通文件、文件名为 `SKILL.md`、受 `MAX_SKILL_BODY_BYTES` 上限；不可读则 fail-closed。
  - 项目级根（`<ws>/.agents/skills`、`<ws>/.claude/skills`、`<ws>/.dsh/skills`）保持**严格 canonical 包含**判定（workspace 内容是不可信输入，防止仓库内 `SKILL.md -> /etc/shadow` 把宿主文件读进提示词）。
- **D2（扫描/加载一致性）**：扫描必须使用与加载同一信任谓词；被跳过的候选必须产生可观测标记 `omitted skill:<name> (<reason>)`，理由枚举：`outside_root` / `too_large` / `duplicate_name` / `not_a_file`。
- **D3（技能去重键）**：`frontmatter.name`（缺省回落目录名）。同名冲突先比 `rank`（小者胜），同 rank 再比 canonical 路径字典序（确定性，不依赖遍历顺序）。
- **D4（渐进披露）**：提示词只注入 `name [scope]: description` 清单，`SKILL.md` 正文只在 `skill_load` 返回。
- **D5（预算）**：宪法与技能清单各受 `InstructionBudget` 约束（默认各 64 KiB）；`provider.context_window_tokens() < 8192` 时不注入技能清单。
- **D6（同目录 AGENTS.md/CLAUDE.md 共存语义）**：本波**不改变**多候选加载语义（保持首命中），只要求「根层」与「触达层」共用同一候选常量，消漂移；打开共存的语义变更留给 wave-59 + ADR。

## 1. Stage 1：上下文根与技能清单注入（owner: executor-alpha）

写域：`crates/pony-agent-core/src/agent/context.rs`、`crates/pony-agent-core/src/agent/subagent_runtime.rs`、`crates/pony-agent-core/src/agent/runtime/`

- **F1-1（根来源）**：`provider_semistable_context_note` 必须以**会话工作区根**为指令/技能发现根，不得再用 `std::env::current_dir()`。根来源优先级：`retrieved.session_context.env_info.cwd`（非空且为绝对路径）→ 否则进程 cwd，且不得 panic。
- **F1-2（技能清单）**：semistable 段在存在技能时必须包含标题 `=== AVAILABLE REUSABLE SKILLS ===`，且逐行形如 ``- `name` [global|project]: description``；无技能时不得出现该标题。
- **F1-3（前缀稳定）**：同一 `conversation_id` 第 2 轮起，宪法 + 技能清单段字节级不变；中途修改 `AGENTS.md` 只允许以追加 notice 形式出现在 volatile 段，不得改写 semistable 段。
- **F1-4（渐进披露）**：`SKILL.md` 正文不得出现在 provider 请求文本中；只有 `skill_load` 工具结果可携带正文。
- **F1-5（子代理继承）**：`HostTurnEngine` 必须以父会话工作区根构建子代理 runtime，不得固定 `compute_default_workspace_root()`；无父根时才回落默认。
- **F1-6（预算门）**：provider 上下文窗口 < 8192 tokens 时，semistable 段不注入技能清单（宪法照旧）。

## 2. Stage 2：技能解析与扫描/加载一致性（owner: executor-beta）

写域：`crates/pony-agent-core/src/agent/workspace_hotloader.rs`、`crates/pony-agent-core/src/agent/skill_body.rs`、`crates/pony-agent-core/src/agent/path_permission.rs`

- **F2-1（块标量）**：frontmatter 中 `description: |` / `description: >` / `|-` / `>-` 及后续缩进续行必须解析为真实描述文本，`description` 不得等于 `"|"`、`">"`、`"|-"`、`">-"`。折叠标量（`>`）按 YAML 语义以空格连接续行，字面标量（`|`）以换行连接。
- **F2-2（描述上限）**：描述长度超过 1024 字符时按 UTF-8 边界截断。
- **F2-3（信任一致）**：`scan_skills*` 与 `skill_load` 使用同一信任谓词（D1）；对全局根符号链接技能：列出 ⇒ 必须可加载；对项目根越界符号链接：不列出，且产生 `omitted skill:<name> (outside_root)`。
- **F2-4（去重键）**：同名（D3）只保留一个，被淘汰者产生 `omitted skill:<name> (duplicate_name)`。
- **F2-5（超限）**：`SKILL.md` 超过 `MAX_SKILL_BODY_BYTES` 的目录被跳过并产生 `omitted skill:<name> (too_large)`。
- **F2-6（报告通道）**：新增 `SkillScanReport { skills: Vec<HotloadedSkill>, omitted: Vec<SkillOmission> }` 与 `scan_skills_report*` 方法；`SkillOmission { name: String, reason: SkillOmissionReason, path: PathBuf }`。既有 `scan_skills*` 签名保持不变（返回 `(Vec<HotloadedSkill>, bool)`），内部委托新方法。

## 3. Stage 3：触达注入去重（owner: executor-alpha or beta，blocked_by Stage 2）

写域：`crates/pony-agent-core/src/agent/tools.rs`、`crates/pony-agent-core/src/agent/instruction_touch.rs`

- **F3-1（根层不重复）**：会话首轮已将根层 `AGENTS.md` 注入 semistable 段后，首次触达工作区内任意文件**不得**再次注入该文件。
- **F3-2（子目录仍注入）**：`sub/AGENTS.md` 在首次触达 `sub/x.rs` 时仍注入，且只注入一次（会话内去重）。
- **F3-3（候选一致）**：根层与触达层使用同一候选常量集合（`AGENTS.md`、`CLAUDE.md`、`.claude/CLAUDE.md`、`AGENTS.local.md`、`CLAUDE.local.md`），由 `workspace_hotloader` 单一导出，`instruction_touch` 不得再自带第二份字面量。
- **F3-4（新 API）**：`workspace_hotloader::constitution_source_paths(workspace_root, global_homes) -> Vec<PathBuf>` 返回当前会参与注入的指令文件 canonical 路径（供触达层种子化）。

## 4. 机器门禁（Join Barrier）

1. `CARGO_TARGET_DIR=target-test cargo check -p pony-agent-core` Exit 0
2. `CARGO_TARGET_DIR=target-test cargo test -p pony-agent-core` 全绿，且相对 `.dev-team/regression-baseline-wave58.json` 无劣化
3. 机械写域审计：`git diff --name-only` 必须落在各任务 `write_scopes` 内
4. blast-radius：`context.rs` / `tools.rs` / `workspace_hotloader.rs` 命中关键字黑名单（`fs`、`lock`、`Command::new`），强制 ≥2 路互补 L3 审查

## 5. 自洽性检查（防止契约内部冲突）

- F1-2 与 F2-3 的交互：清单必须只列「可加载」的技能，故 F1-2 的断言只对已通过信任谓词的技能生效。
- F1-3 与 F3-1 的交互：根层来源集合由 `constitution_source_paths` 决定，与 semistable 段一致，不得各自计算。
