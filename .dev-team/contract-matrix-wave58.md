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

## 6. Lead 后续裁决（实现期追加，优先于上文冲突处）

- **D5'（替代 D5）**：技能清单预算按 provider 窗口自适应。`window = provider.context_window_tokens()`；`None` → 不裁剪；`< 32768` → 整段跳过技能清单（宪法照旧）；`>= 32768` → `max_bytes = min(16KiB, window*4/20)`、`max_entries = 64`，超预算条目从**低优先级尾部**丢弃并输出 `omitted skill:<name> (budget)`（≤8 条 + `omitted skills: N more (catalog budget)` 折叠行）。裁剪前必须按 `(rank 升序, scope 项目优先, name 字典序)` 排序确定优先级，渲染顺序保持原返回顺序。
  - 证据：本机全局根 116 技能 / 约 12KB 清单注入 8192 窗口后，`agent::context::tests::build_request_truncates_native_transcript_for_reasoning_provider` 与 `..._keeps_native_tool_roundtrip_on_turn_boundary` 变红；`HOME` 指向空目录复跑转绿 ⇒ 体积导致 recent history 被截断。
- **D6'（Stage 3 生效条件）**：触达层跳过「根层指令来源」的充要条件是本次工具调用 `context.session_id` 非空（该会话前缀已由 semistable 承载根层指令）。`session_id` 为空时保持既有行为，故 wave42 C4 系列契约零改动。根层来源集合由 `constitution_source_paths(workspace_root, &resolve_global_agent_homes())` 给出。
  - 后果：F3-1 用例由 Lead 修订为 `execute_with_context` + 非空 `session_id`（Wave-58 内冻结测试的唯一授权修改，Lead 执行）。
- **D7（红相缺口处置）**：F1-* 与 F2-* 的红相因实现先行（DEV-58-1）不可得，改由 L3 审查以「实现 diff ↔ 契约矩阵 F 编号」逐条对照替代；F3-1/F3-3 保留真实断言失败红相。

- **F2-7（会话基线有界化，Lead 补登授权）**：`WorkspaceHotloader.session_baselines` 必须有界（`MAX_SESSION_BASELINES = 1024`），超限按 LRU 淘汰；淘汰**只**降低「中途变更感知」精度（被淘汰会话下一轮视同首轮重建基线），**不得 panic**、不得影响未超限会话。淘汰必须输出一行可观测诊断（`[pony-agent][hotloader]` 前缀），禁止静默。
  - 与 F1-3 的关系：F1-3「第 2 轮起字节不变」只在**未被淘汰**的会话内成立；活跃会话每轮都会刷新 `last_used`，因此实际只会淘汰长期不活跃会话。此权衡由本条目显式授权。
  - 补登说明：该条目最初由 Lead 以 send_message 下发（未经契约矩阵登记），L3 审查正确指出程序瑕疵；此处补登并锁定新的 1024 上限与诊断要求。
- **B1（L3-join 追加，跨工作区缓存隔离）**：`WorkspaceHotloader` 已成为进程级单例，因此 `skill_files` 与合并后的 `skills` 缓存**必须**按 canonical workspace root 分桶；缓存命中除 `mtime + file_size` 外必须比对 `fingerprint.path`。禁止出现「A 工作区的技能描述进入 B 工作区提示词」。
