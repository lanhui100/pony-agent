# Agent Note: 提示词-工具一致性门禁与 v2.1.0 契约回填

Status: implemented

## Problem

稳定层提示词与工具注册表是两份**互不校验**的事实源。

- 提示词按产品名点名工具：`TodoWrite`（`context.rs` BASE 段第 29 行）、`create_goal` / `update_goal`（goal 策略段）、团队工具与 `Run`（`AGENT_TEAM_POLICY_PROMPT`）。
- 工具名、别名与曝光面独立维护在 `crates/pony-agent-core/src/agent/tools/names.rs`：`canonical_tool_name` 做产品名→primitive 映射，`tool_exposure_for_name` 决定 `ModelVisible` / `Deferred` / `Internal`。

两者之间没有任何机械约束。后果是**静默撒谎**：把 `TodoWrite` 改名、把某个工具降级成 `Deferred`（只能经 `ToolSearch` 提升曝光）、或删掉某工具，提示词仍会继续要求模型调用它，而现有全部测试都不会变红。这正是本仓「凡机械可查的承诺，配一条非零退出的命令」尚未覆盖的一块。

同一时期还暴露了第二处同源腐化：`.dev-team/contracts/system-prompt-v2.md`（标称 FROZEN v2.0.0）与代码已失配 4 项 —— §2 冻结全文仍是 v2 原文（缺 receipts / goal 策略 / 不可信数据扩展后的条款，且仍写 `` `todo_write` ``）、SP2 正例片段是旧措辞、§6 登记字节数 3099（实测 3790）、SP9 阈值 3400（测试实际 3900）。文档承诺与实现分家，等于「冻结」二字失效。

本条接续并补强 `implemented/architecture/2026-11-11-system-prompt-layered-architecture-and-receipts-guard.md`（提示层序不变量与 receipts 注入）：那条建立了「层序可测」，本条建立「文本与工具表可测」。

## Decision

1. **新增一致性门禁** `crates/pony-agent-core/tests/prompt_tool_consistency.rs`（PT1–PT5），把两对事实源钉死：

   | ID | 断言 | 失灵时暴露 |
   | --- | --- | --- |
   | PT1 | `prompt_tool_references()` 每项都能被 `canonical_tool_name` 解析 | 提示词点名了改名/删除的工具 |
   | PT2 | 每项曝光面为 `ToolExposure::ModelVisible` | 提示词要求使用模型看不见的工具 |
   | PT3 | 每项确实出现在 BASE 或团队策略段文本里 | 登记表僵化成过期清单 |
   | PT4 | 提示词里"像工具名"的 token 必须已登记，**或**在白名单里（fail-closed 两支） | 改名漂移点名了不存在的工具、新增点名漏登记 |
   | PT5 | 契约 §2 全文与 §6 字节数必须与 `base_system_prompt()` 逐字/逐字节一致 | 改文案不改契约 |

2. **登记真源下沉到代码**：`agent::tools::prompt_contract::{prompt_tool_references, prompt_non_tool_tokens}` 是提示词点名工具的唯一清单（10 项）与「像工具名却不是工具」的显式白名单；PT1/PT2 校验清单，PT3/PT4 双向校验清单与文本。清单落在 `tools/prompt_contract.rs` 而非 `context.rs`，因为后者已顶到 `size:check` 存量基线（3003 行），净增任何行都是新违规；按 `implemented/process/2026-11-12-file-size-gate-module-split-not-baseline-bump.md` 的先例拆模块，不抬基线。

3. **契约回填为 v2.1.0**：§2 全文按代码现状逐字回填（未删任何承诺条款）、SP2/SP7 片段改按代码措辞与模型可见名（`TodoWrite`，内部 primitive 名 `todo_write` 不得进提示词）、SP9 改为「逐字冻结 `== 3790` + 护栏 `<= 3900`」、§6 实测记录重测、C2 偏差标记关闭，并新增 §7 门禁说明与维护约定。

4. **PT4 是 fail-closed 三分支的**：形状启发式（长度 ≥ 5 且含下划线或内部大写）命中后，token 按 `(是否登记, 是否可解析)` 落入三支之一——①可解析但未登记；②**已登记却解析不到**（改名后三方自洽地指向不存在的工具）；③解析不到且不在 `prompt_non_tool_tokens()` 白名单。第一版只做了第 ① 支：独立验证者的 A′ 变异（`TodoWrite` → `TodoWriteRenamed`，并同步改提示词/登记表/契约使三方"自洽"）实证其 5/5 全绿。第二版补 ③ 支后复验：A′ 被拦下，但拦下它的是 PT1/PT2（PT4 因 `registered.contains` 先跳过仍放行）——因此补了 ② 支，让 PT4 自身独立成立；三条防线并集覆盖，且每条都如实描述自己的判据。
   - 启发式边界同样写进注释：只把长 token 当候选，避免把散文里的 `Read`（`Read-before-write`）误判成工具点名。

## Alternatives considered

- **把工具条款搬进工具自身的条件段**（DSH 的 `systemPrompt.section` + `text: ({scope}) => tools.get('read') ? … : ''` 模式）：结构上最彻底，但要新建注册制提示层、改动 native/normalized 双视图与 `reserved_system_message_extra_count()` 的 trace 判定，属于跨文件重构；且本仓内建工具（todo/goal/团队）曝光面恒定，当前**无行为缺陷可修**。按「最小闭包」拒绝夹带，登记为后续提案与待办（`proposed/architecture/2026-11-13-context-assembly-single-pass-projection.md` 第 3 条 / backlog B138）。**已由 `implemented/architecture/2026-11-13-tool-owned-prompt-guidelines-and-templates.md` 落地**：工具条款自注册 + 模板化，渲染字节不变。
- **只补契约文档、不加门禁**：能修当下漂移，但下次改文案仍会静默分家——文档没有执行力，门禁才有。
- **让测试直接 `grep` 提示词里的工具名并与 `names.rs` 对拍**（不引入 `prompt_tool_references()`）：省一个 API，但反向扫描需要在测试里复制一份「什么算工具名」的判定，且无法给 PT2 的曝光面校验提供稳定输入；真源放代码里，测试只做比对更简单。
- **把字节数改为 `<=` 单一阈值**（放弃逐字冻结）：会丢掉「文案只能在契约内改」的强约束，与既有 `assert_eq!(len(), 3790)` 冲突；改为「逐字冻结 + 宽护栏」两段式，既保留强约束又给护栏留余量。

## Consequences

- 改提示词文案、改工具名、改工具曝光面，现在各有对应门禁会红；三处必须同提交更新，漂移从「靠人记得」变为「冲突即报错」。
- PT5 让契约文档成为**可执行承诺**：文档不再是一次性交付物，而是测试输入。
- 已知残余：清单与白名单都是人工维护（PT4 会逼出新条目，但归因仍需人判断）；`Run` / `Read` 这类短名（<5 字符）不参与反向扫描（启发式边界）。
- 门禁的正向证据（独立验证者变异测试，7 组 + A′ 复验 2 轮）：PT2 抓「工具降曝光」；PT5 抓「契约文本漂移」与「契约字节漂移」；PT3 抓「登记项不在文本中」；A′（三方自洽地指向不存在的工具）由 PT1/PT2 与 PT4 第 ② 支并集拦截。若只保留第一版 PT4，改名漂移会以 5/5 全绿提交。
- 契约文档 §5 仍保留 v2.0.0 阶段的 T3/executor 叙述（历史交付语境），未改；如需清理按 `docs/AGENTS.md` 分层规则另案处理。
