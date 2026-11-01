# Wave-58 交付说明：上下文构建与 Skills / AGENTS.md 加载修复

> 契约矩阵：`.dev-team/contract-matrix-wave58.md`（含 D1..D7 / F2-7 / B1）
> 上游审查报告：`management/task-system/02_REVIEWS/2026-11-01-context-skills-loading-audit.md`
> 回归基线：`.dev-team/regression-baseline-wave58.json`

## 1. 交付内容（按缺陷编号）

| 缺陷 | 修复 | 落点 |
|---|---|---|
| P0-1 技能清单未注入提示词 | semistable 段接入进程级单例 hotloader，注入宪法 + `=== AVAILABLE REUSABLE SKILLS ===` 清单（渐进披露：只列 name/scope/description） | `context.rs` |
| P0-2 指令发现用进程 cwd | 改用会话工作区根 `env_info.cwd`（非空且绝对路径），否则回落进程 cwd | `context.rs::resolve_session_instruction_root` |
| P0-3 子代理不继承工作区 | `HostTurnEngine.workspace_root` + `with_workspace_root` + `resolved_workspace_root()` 父根优先 | `subagent_runtime.rs` |
| P0-4 缓存永不命中 | 改用 `get_global_workspace_hotloader()` 单例 + 会话基线（首轮固化、后续 append-only notice） | `context.rs` / `workspace_hotloader.rs` |
| P1-3 触达层重复注入根层 | D6'：会话内（`session_id` 非空）按 `constitution_source_paths` 种子化 `seen`；无 session 保持既有行为 | `tools.rs` / `instruction_touch.rs` |
| P1-4 触达/根层候选不一致 | 候选集合收敛为 `workspace_hotloader::INSTRUCTION_CANDIDATES` 单一来源 | `instruction_touch.rs` |
| P1-5 去重键用目录名 | D3：`frontmatter.name` → `rank` → canonical 路径字典序 | `workspace_hotloader.rs` |
| P1-6 列出但加载 404 | D1/D2：扫描与加载共用 `trust_skill_file*` 谓词；全局根放行符号链接逃逸（仍要求名为 SKILL.md 的普通文件、无 `..`、受字节上限），项目根保持严格 canonical 包含 | `skill_body.rs` / `path_permission.rs` |
| P1-7 块标量解析失败 | 手写 frontmatter 状态机支持 `| > |- >- |+ >+`、公共缩进剥离、折叠/字面语义、`allowed-tools` 三形态、描述 1024 字符截断 | `workspace_hotloader.rs` |
| P2-1 丢弃无标记 | `SkillScanReport.omitted` + `omission_markers()` 接入 semistable 清单段（≤8 具名 + 折叠行） | `workspace_hotloader.rs` |
| P2-3 技能根重复扫描 | 技能根先 canonical 去重 | `workspace_hotloader.rs` |
| P2-6 描述无上限 | 1024 字符截断 | `workspace_hotloader.rs` |
| 新增 F2-7 | 会话基线有界化（1024 LRU）+ 淘汰/未命中可观测诊断 + append-only notice | `workspace_hotloader.rs` |
| 新增 D5' | 技能清单按 provider 窗口自适应预算（`<32768` 跳过；`>=32768` 时 `min(16KiB, window*4/20)` / 64 条，超预算按 `(rank, 项目优先, name)` 从尾部丢弃并打标） | `context.rs` / `workspace_hotloader.rs` |

## 2. 门禁收据

- `CARGO_TARGET_DIR=target-test cargo test -p pony-agent-core`：见 `.dev-team/state.json` 的 `join_barrier.full_regression`（基线 1777 passed；预期 1798 = 基线 + 21 条新增验收用例）。
- `cargo check -p pony-agent-core` Exit 0。
- 机械写域审计：`git diff --name-only` 全部落在各任务 `write_scopes` 内（含 Lead 授权的 D6' 测试修订 1 个文件）。
- 可观测性门禁：`audit-diff.sh --base 9af49dd` → `[OK] 无越界实现、收口点完整`，Exit 0。
- L3 对抗审查：`review-join-a/b.json`（第 1 轮 FAIL：2 BLOCKING）→ 修复 → `review-join-a2/b2.json`（第 2 轮复审）。

## 3. 已知缺口（不隐藏）

1. **F1-5 生产未接线（既有缺陷，非本波引入）**：`HostTurnEngine` 在生产代码中**没有任何构造点**，`set_subagent_engine` 生产零调用，`orchestration.rs:252` 恒返回 `capability_unavailable: subagent engine not wired`。本波只把「继承父工作区根」的引擎能力做好，接线点（orchestration / src-tauri 装配）不在本波写域。
2. **A2 残余缺口（D 类边缘）**：`env_info` 缺失**且**进程 cwd ≠ 工具 `workspace_root` 时，semistable 用进程 cwd、触达层种子用 `workspace_root`，两侧根层来源不对等 ⇒ 该窗口内根层规则可能零注入。桌面主链路 `store.rs::snapshot_env_info` 恒提供会话根，正常会话不触发。修法（传递「本次实际注入根」通道）列入 wave-59。
3. **同目录 `AGENTS.md` + `CLAUDE.md` 共存语义未改（D6 本轮不做）**：仍是「首个命中即停」，内容不同的 `CLAUDE.md` 会被静默忽略。语义变更（共载 + canonical/内容双重去重）列入 wave-59 + ADR。
4. **`.agent/`（单数）与仓库根 `AGENT.md` 不被识别**：当前只支持 `.agents`。若需兼容 Antigravity 风格布局，列入 wave-59。
5. **`~/.dsh` 全局根缺失**：rank 表预留 500 但 `resolve_global_agent_homes_from` 不产生该路径（死分支），文档与实现不一致。列入 wave-59。
6. **真实符号链接技能**：全局根放行逃逸后，本机 9 个外部符号链接技能由「列出不可载」变为「可载」（P1-6 目标）；项目根越界符号链接仍在扫描期拒绝并打 `omitted skill:<name> (outside_root)`。

## 4. 流程偏离记录

- **DEV-58-1**：Stage1/Stage2 实现在 Test Agent 红相交付前并行启动，导致 F1-*/F2-* 的「空桩态断言失败」物理收据不可得。补偿：契约矩阵在实现前冻结、测试由独立 Test Agent 编写、F3-1/F3-3 保留真红相、由 L3 审查以「实现 diff ↔ F 编号」逐条对照替代。
- **D6'**：F3-1 由 Lead 修订（冻结测试唯一授权修改），起因是新契约与既有 wave42 C4-5 逻辑互斥；裁决为「按 `session_id` 门控」，既有 C4 契约零改动。
- **F2-7 补登**：该条目最初仅经 send_message 下发、未登记契约矩阵，被 L3 正确指出程序瑕疵，现已补登。

## 6. 计划阶段对齐审查（补做）结论

`review-plan-wave58.json`（schema v2，Subagent 独立审查，verdict=PASS，3 ADVISORY）：

- **覆盖性**：契约矩阵 22 条（F1-1..F1-6 / F2-1..F2-7 / F3-1..F3-4 / B1）**全部有实现落点**，无缺失。
- **未授权变更 1 处（已追认）**：semistable 段标题由 ADR-0045 C6 冻结的 `=== Workspace & User Instructions (AGENTS.md / CLAUDE.md) ===` 变为 hotloader 生成的 `=== WORKSPACE CONSTITUTION & RULES ===`（该段现已同时承载宪法 + 技能清单 + `omitted` 诊断，旧标题会让模型误判段边界）。已写入 **ADR 0046 Wave-58 修订**追认。
- **`skills` 为空但 `omitted` 非空时仍渲染标题**：Lead 裁定**保留**——向模型表达「有技能但不可用（越界/超限/同名）」比静默无提示更有用；已有用例覆盖两个空集分支。
- **技能缓存按 workspace 分桶无淘汰（真实泄漏）**：已修（`MAX_SKILL_CACHE_WORKSPACES = 16` + LRU + 淘汰时清理逐文件缓存键 + 诊断行）。
