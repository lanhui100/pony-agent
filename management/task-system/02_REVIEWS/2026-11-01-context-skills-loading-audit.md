# 审查报告：上下文构建与 Skills / AGENTS.md 加载机制

- 日期：2026-11-01
- 范围：`crates/pony-agent-core/src/agent/{context.rs, workspace_hotloader.rs, skill_body.rs, instruction_touch.rs, path_permission.rs, tools.rs, subagent_runtime.rs}`，ADR 0045 / 0046
- 方法：静态走读 + 临时探针测试（`cargo test --test zz_audit_probe`，exit 0，探针已删除，未改动任何源码）

## 0. 结论

用户感知的「`.agents/` / `.claude/` 下的 skills 没有自动加载」由两个独立缺陷叠加造成：

1. **技能清单从未注入主回合提示词**（P0-1）。扫描器、清单渲染、会话基线代码都已实现且有测试，但生产路径没有任何调用方。模型能看到 `skill_load` 工具，却不知道有哪些技能名可加载。
2. **指令发现用的是进程 cwd，不是会话工作区根**（P0-2）。桌面端进程 cwd 通常不是工作区，`AGENTS.md` / `CLAUDE.md` 读取位置因此出错；技能根同理。

另有若干去重缺陷、frontmatter 解析缺陷和「列表里有、加载被拒」的不一致，见下文。

## 1. 物理凭证（探针输出，workspace=/home/dm/pony-agent）

```
PROBE skills_total=88
PROBE block_scalar_desc=16 ["design-style-skill","dev-backlog","dev-docs","doc-writer",...,"resolve-issues","website-to-hyperframes"]
PROBE skill_load gsap listed=yes scope=Global load=skill_out_of_allowed_root
PROBE skill_load website-to-hyperframes listed=yes scope=Global load=skill_out_of_allowed_root
PROBE skill_load write-adr listed=yes scope=Project load=ok
PROBE touch_injects=["/home/dm/pony-agent/AGENTS.md"]
test result: ok. 1 passed; 0 failed
```

调用方检索（`grep resolve_session_prompt_context|build_cached_prompt_prefix|scan_skills`）：生产代码中 `scan_skills*` 只有 `skill_body::resolve_skill_by_name`（skill_load 内部）一个调用方；`resolve_session_prompt_context*`、`build_cached_prompt_prefix` 只在测试中被调用。

## 2. 缺陷清单

### P0：主链路缺陷

| ID | 问题 | 位置 | 影响 |
|---|---|---|---|
| P0-1 | 文件型技能清单（`=== AVAILABLE REUSABLE SKILLS ===`）只存在于 `build_cached_prompt_prefix`，而该函数和 `resolve_session_prompt_context*` 没有生产调用方。`context.rs` 的 `provider_semistable_context_note` 只注入宪法文本。volatile 段的 `planner_skills` 来自 `CapabilityRegistry`（host/MCP 快照），与 `SKILL.md` 无关。 | `context.rs:817-844`，`workspace_hotloader.rs:584-745` | 88 个已发现技能对模型不可见；`skill_load` 只能靠模型猜名字。这就是「没有自动加载」的根因。 |
| P0-2 | 宪法发现以 `std::env::current_dir()` 为根，没有使用会话工作区。同一请求里 env note 的 `cwd` 已经按会话工作区计算（`store.rs:2811 snapshot_env_info`），工具层也用 `context.workspace_root`，三者不一致。 | `context.rs:829` | Tauri 进程 cwd 不是工作区时，项目 `AGENTS.md` / `CLAUDE.md` 根本读不到，或者读到别的目录的文件；多工作区并发时所有会话读同一份。 |
| P0-3 | 子代理工作区固定为 `compute_default_workspace_root()`（`~/pony_agent`），不继承父会话工作区。 | `subagent_runtime.rs:121-124` | 子代理拿不到项目指令和项目技能。 |
| P0-4 | 每回合 `WorkspaceHotloader::new()` 新建实例；`resolve_skill_by_name` 也是每次新建。进程级单例 `get_global_workspace_hotloader()` 和会话基线（KV-Cache 保护）在生产中未使用。 | `context.rs:828`，`skill_body.rs:217` | 缓存命中率为 0，每回合全量 stat + 读盘（88 个技能 + 全部指令层）。会话中途修改 `AGENTS.md` 会直接改写前缀，与 ADR 0007/0046「前缀只追加」的意图相反。 |

### P1：去重与一致性

| ID | 问题 | 位置 | 影响 |
|---|---|---|---|
| P1-1 | **同目录 AGENTS.md 与 CLAUDE.md 共存时只取首个命中**（`AGENTS.md → CLAUDE.md → .claude/CLAUDE.md`），后两者被静默丢弃，没有任何提示。ADR 0045 有意这么设计，但内容不同的情况没有覆盖：用户在 `CLAUDE.md` 写了独有规则，Pony 不加载，也不告知。 | `workspace_hotloader.rs:10, 784-792` | 本仓库两者是符号链接所以没问题；其他仓库会出现规则丢失。 |
| P1-2 | 跨层去重只比较「规范化后的正文」，**不比较 canonical 路径**。两份文件内容相同但空白不同可以去重；同一个文件经两个根到达也能去重（内容相同）。但同一文件的两份副本，只要有一份被预算截断（`oversized`），就不参与判等，会重复注入。 | `workspace_hotloader.rs:1023-1036` | 边缘场景重复注入。 |
| P1-3 | **触达即载没有感知根层已注入的内容**。`TouchedInstructionMemory.seen` 初始为空，第一次读写工作区内任意文件，就会把根目录 `AGENTS.md` 作为 "Additional instructions" 再注入一次（探针：touch `src/main.ts` → 注入 `/home/dm/pony-agent/AGENTS.md`）。 | `tools.rs:1165-1246`，`instruction_touch.rs:66` | 每会话重复约 3.4 KB；还会与 P0-2 叠加：根层来自进程 cwd，触达层来自真实工作区，两份内容不同时规则冲突。 |
| P1-4 | 触达层候选只有 `AGENTS.md`/`CLAUDE.md`，缺少 `.claude/CLAUDE.md` 和 `*.local.md` 覆盖层；根层有这些候选。两层的发现规则不一致。 | `instruction_touch.rs:20` vs `workspace_hotloader.rs:10-14` | 同一目录在不同入口加载的文件不同。 |
| P1-5 | 技能去重键是**目录名**，不是 frontmatter `name`，也不是 canonical 路径。两个不同目录声明同一 `name` 时会以同名列出两次；`resolve_skill_by_name` 按 BTreeMap 顺序取先命中的一个，与 rank 无关。 | `workspace_hotloader.rs:541-554`，`skill_body.rs:220-228` | 同名技能二义，加载结果不可预测。 |
| P1-6 | **扫描与加载的信任判定不一致**：扫描接受任何 `<root>/<dir>/SKILL.md`（包括符号链接到根外的目录），`skill_load` 要求 canonical 目标在受信根内。`~/.claude/skills/gsap → ~/.codex/.tmp/...` 这类常见安装形态被列出，加载时却返回 `skill_out_of_allowed_root`。这正是 F4 想消除的「列表有、加载 404」。 | `workspace_hotloader.rs:468-479`，`skill_body.rs:97-104` | 本机 2 个技能可见但不可用（gsap、website-to-hyperframes）。 |
| P1-7 | frontmatter 按行解析，不是 YAML 解析器。`description: |` / `>` 块标量被解析为字面量 `"|"` / `">"`；多行 `allowed-tools` 列表、带冒号的值、转义引号都会出错。 | `workspace_hotloader.rs:1176-1236` | 本机 88 个技能中 16 个描述变成 `|`/`>`，模型无法按描述选技能。 |

### P2：健壮性、可观测性与代码卫生

| ID | 问题 | 位置 |
|---|---|---|
| P2-1 | `skill_metadata_within_scan_budget`、`omitted_skill_marker` 只有定义，没有调用；超限技能和超预算被丢弃的技能都静默消失，模型和用户都不知道。 | `skill_body.rs:191-206`，`workspace_hotloader.rs:1286-1325` |
| P2-2 | `~/.dsh/skills` 在 rank 表中是 500，但 `resolve_global_agent_homes_from` 从不产生 `~/.dsh`，是死分支。文档与实现不一致。 | `workspace_hotloader.rs:888-935` |
| P2-3 | 项目技能根 `.claude/skills` 是指向 `.agents/skills` 的符号链接时，`push_skill_dir` 用非 canonical 路径去重，同一批目录被扫描两次（结果正确，IO 翻倍）。 | `workspace_hotloader.rs:945-954` |
| P2-4 | 单一项目层时返回原始字节，多层时加 `## ... (<path>)` 标题。新增一个全局文件会改变整段格式，前缀整体漂移。 | `workspace_hotloader.rs:987-1009` |
| P2-5 | `instruction_scope_sources` 只记录 `thread://base-system` 和焦点文件，不记录实际加载的 `AGENTS.md`/`CLAUDE.md` 路径；可观测面无法回答「这一轮注入了哪些规则文件」。 | `context.rs:1025-1031` |
| P2-6 | `user_invocable`、`allowed_tools` 已解析但无消费方；描述没有长度上限（Agent Skills 规范为 1024 字符），长描述直接放大前缀。 | `workspace_hotloader.rs:96-107` |
| P2-7 | 生产中未使用 `find_project_root`：工作区是仓库子目录时，仓库根的 `AGENTS.md`、`.agents/skills` 不加载（ADR 0046 记为已知语义，建议改为以 `.git` 为界上探）。 | `workspace_hotloader.rs:823-824` |
| P2-8 | 悬空符号链接技能（本机 `camofox-browser`、`find-nearby` 等 9 个）和缺少 `SKILL.md` 的目录被静默跳过，没有诊断信息。 | `workspace_hotloader.rs:476-479` |
| P2-9 | 仓库根的 `AGENT.md`（单数，5.9 KB）不是候选文件名，不会被加载。若它仍是有效规则，需要合并进 `AGENTS.md`；若已过时，建议删除，避免误导。 | 仓库根 |

## 3. 优化方案（分阶段，每阶段可单独合入）

### 阶段 1：修复主链路（P0-1 ~ P0-4、P1-3），最小闭包

1. **统一根解析**：在 `ContextStateQuery` / `SessionContext` 中携带会话工作区根。最小改法是复用已有的 `session_context.env_info.cwd`（已按会话工作区计算），取不到时才回退进程 cwd，并输出告警。`provider_semistable_context_note` 改用该根。
2. **接入技能清单**：semistable 段改为调用进程级单例 `get_global_workspace_hotloader().resolve_session_prompt_context_with_budget(session_id, turn_count, root, homes, budget)`，同时得到宪法和技能清单。清单只列 `name [scope]: description`，正文仍按需通过 `skill_load` 加载（渐进披露）。
3. **前缀稳定**：会话首轮固化基线；后续变更产生的 `delta_notice` 追加到 volatile 段，不改写前缀（这段代码已有，只需接线）。
4. **子代理继承工作区**：`HostTurnEngine` 接收父会话的 `workspace_root`，不再固定 `~/pony_agent`。
5. **触达去重种子**：会话首轮把根层已注入的来源 canonical 路径写入 `TouchedInstructionMemory.seen`，触达层只注入真正的子目录指令。

验收（先写红测，由非实施者冻结）：
- 进程 cwd ≠ 工作区时，提示词包含工作区 `AGENTS.md` 标记，不包含 cwd 下的标记；
- 工作区 `.agents/skills/x` 与全局 `~/.agents/skills/y` 都出现在 provider 请求文本中；
- 第 2 轮修改 `AGENTS.md` 后，前缀字节不变，volatile 段出现 notice；
- touch 根目录文件不再重复注入根 `AGENTS.md`；
- 子代理请求包含父工作区的项目指令。

### 阶段 2：技能解析与一致性（P1-5 ~ P1-7、P2-1、P2-3、P2-6、P2-8）

1. frontmatter 改用 YAML 解析器（`serde_yaml` 或 `yaml-rust2`，需要新增依赖，按规则补 ADR），仍保持 64 KiB 有界前缀读取；解析失败的技能回退到目录名加首段文本，并记录诊断。
2. **扫描与加载共用同一个信任谓词**。可选两种方案（需决策）：
   - A（推荐）：以「技能目录入口位于受信根内」为信任边界，允许根内的符号链接指向外部（这是 `npx skills` / Claude Code 的标准安装形态），仍要求目标是普通文件、受字节上限约束、不跟随 `..`；
   - B：保持 canonical 包含判定，扫描期同样拒绝，并在清单尾部写 `omitted skill:<name> (outside root)`。
3. 去重键改为 frontmatter `name`（缺省用目录名），先比 canonical 路径再比 rank；`resolve_skill_by_name` 改为在已去重结果上精确匹配。
4. 技能根先 canonicalize 再去重；描述截断到 1024 字符；超限、悬空、预算丢弃统一输出 `omitted skill:<name> (<reason>)`。

### 阶段 3：AGENTS.md / CLAUDE.md 共存去重语义（P1-1、P1-2、P1-4、P2-4、P2-7）

建议的去重规则（需要新 ADR 取代 ADR 0045 第 32 行的决定）：

1. 每级目录收集全部候选：`AGENTS.md`、`CLAUDE.md`、`.claude/CLAUDE.md`、`AGENTS.local.md`、`CLAUDE.local.md`；
2. 先按 **canonical 路径**去重（处理本仓库这类符号链接），再按**规范化内容哈希**去重；
3. 剩余内容不同的文件：
   - 方案 A（推荐，兼容 Claude Code/Codex 双生态）：都加载，同目录内 `AGENTS.md` 在前，`CLAUDE.md` 在后；
   - 方案 B（保守）：保持首命中，但对被遮蔽的文件追加 `shadowed <path>` 标记，让用户知道；
4. 根层与触达层共用同一个候选表和发现函数，消除 P1-4；
5. 渲染格式在单层和多层情况下保持一致（始终带来源标题），消除 P2-4 的格式漂移（一次性前缀变更，需要更新基线测试）；
6. 以 `.git` 为界上探到仓库根（`find_project_root`），工作区是子目录时也能加载仓库根规则。

### 阶段 4：可观测性（P2-5）

- `instruction_scope_sources` 写入实际注入的每个来源（`file://<canonical>#scope=global|project`），以及 omitted、truncated、shadowed 标记；
- 按仓库规则先在 `docs/observability.yaml` 中登记四维归属，再改代码。

## 4. 风险与依赖

- 阶段 1 会改变每轮提示词内容（新增约 24 KB 技能清单，按本机 88 个技能估算）；64 KiB 技能预算内可容纳，但小上下文模型需要按 `context_window_tokens` 下调预算。
- 阶段 2、3 涉及跨模块契约和新依赖，按 `write-adr` 规则需要先写 ADR。
- 既有基线测试（`wave42_global_config.rs` C2-1 字节等值、`context_cache_red_contract.rs`）会受阶段 1/3 影响，必须由测试方更新冻结基线，实施方不得自改。
