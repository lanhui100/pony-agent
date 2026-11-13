# Agent Note: 上下文装配单程内容单源 + 双视图投影（B133）

Status: implemented

## Problem

`crates/pony-agent-core/src/agent/context.rs::build_layered_turn_context` 曾把同一批语义层**渲染多次、组装多遍**：

- 易变上下文每 turn 渲染 **4 次**：normalized 预算（传 `truncation_note = None`）/ normalized 最终 / native 预算 / native 最终。易变段内部会读全局目标存储（`goal::active_goal_for_session`）与待办存储（`todo::todo_list`）——重复渲染等于重复读取同一份全局事实，且预算路径那一次的产物**从不进入**任何消息。
- 长期记忆渲染 2 次（`build_memory_messages` / `build_native_memory_messages`，文案逐字重复，只有角色包装不同）。
- 原生视图的层序（base → domain → semistable → memory → carry → volatile → input）在 normalized 之外**再组装一遍**，其中 domain 段靠 `reserved_native_messages.len() - 1` 推导插入下标（L574）；Wave-61 C9 已为「门控二次求值 → splice 越界」打过补丁。
- 函数长度 **259 行**，越过 `size:check` 的 200 行硬阈值并被基线祖父化：任何新增层都会让门禁变红。
- 原生视图不是边缘分支：`provider/mod.rs` 的 `requires_provider_native_tool_flow`（OpenAI 家族 + reasoning）与 `supports_native_tool_transcript_replay`（OpenAI 家族 + tools）让 native 成为 DeepSeek 等主模型的主路径。

## Decision

1. **内容单源**：新增 `crates/pony-agent-core/src/agent/turn_prompt_content.rs::TurnPromptContent`，一次性构造 BASE/团队策略、domain profile、semistable、长期记忆、易变段五组文本；`normalized_prefix()` / `native_prefix()` / `native_volatile()` / `system_messages_of()` 只做**角色投影**（normalized：稳定层 `System`、其余 `Developer`；native：稳定层全 `system`）。
2. **易变段分段渲染**：`context.rs` 内的 `VolatileNotes { body, delta }` 把「截断提示之前的正文」与「semistable 变更通知」分离；正文只渲染一次，各视图用自己的截断文案（历史条数 vs 原生 transcript 条数）按原顺序拼装。`goal`/`todo` 全局读取由每 turn 4 次降为 1 次。
3. **删除重复构造与隐式下标**：`len()-1` splice 消失（层序由 `TurnPromptContent` 字段顺序显式表达）；四个重复 helper（`build_volatile_context_messages`、`build_native_volatile_context_messages`、`volatile_context_note`、`build_native_memory_messages`）删除。重复面从「4 渲染 + 2 记忆 + 2 组装」降到「1 渲染 + 1 记忆 + 2 角色投影」。
4. **等价性门禁**：单测 `normalized_and_native_views_project_the_same_content` 对同一输入逐条比对两个视图的层数与内容，任一条分叉即红——这是「单源」承诺的可执行形态。
5. **规模债清零**：`build_layered_turn_context` 259 → **184 行**（< 200），`context.rs` 3003 → 2936 行；`scripts/file-size-baseline.json` 中两条 `context.rs` 条目（file-lines 3003 / fn-lines 259）删除（棘轮只允许删条目或下调）。
6. **兼容面不变**：`LayeredTurnContext` 结构、`flatten_layered_*`、观测装配、`build_request` 签名全部未改；两个视图的消息角色/顺序/文案与重构前一致。

工具自注册部分已先由 `implemented/architecture/2026-11-13-tool-owned-prompt-guidelines-and-templates.md` 落地（模板化 + 工具条款表），本条只覆盖装配去重与规模债。

## Alternatives considered

- **只做 Step 1（单源渲染 + 显式顺序 + 等价测试），不碰结构投影**：风险最低，但函数仍 >200 行、baseline 条目仍在，B133 只能部分闭环；本次按用户裁决直接做到 Step 2。
- **按 DSH 的注册制重写整个提示层（section registry + scope 遮蔽 + waterfall）**：可组合性最好，但会把 `ProviderRole` 双轨、native transcript 重放、Anthropic 单 system join 全部卷进同一变更，超出一次可验证半径。
- **只把 native 分支抽成独立函数以压行数**：能过规模门禁，但重复渲染仍在，等于只治症状；本条约九成收益来自「只渲染一次」而非「拆函数」。
- **易变段整串渲染一次后按视图做字符串替换插入截断提示**：省渲染，但依赖字符串匹配（截断提示文案本身可能含相同子串），易错位；分段结构显式表达顺序更安全。
- **`native_user_input` 直接返回 `content` 值而非完整消息对象**：实现期实测会破坏 `duplicate_tool_recovery_request_keeps_full_context`（native 用户消息变裸字符串，恢复前缀不一致）；已改回显式 `{role, content}` 包装——该断言正是本重构的价值证明。

## Consequences

- 每 turn 易变段全局读取（goal/todo）4 → 1 次；预算与最终消息共享同一份内容，**同一请求内不可能再出现「预算按 A 算、正文按 B 写」**。
- 新增提示层只需改 `TurnPromptContent` 一处字段 + 投影，不再需要在两个视图各加一段代码；`len()-1` 式隐式下标消失。
- 规模债：`context.rs` 两条基线条目删除（棘轮合规），函数 184 行，`size:check` 0 new violation。
- 删除面收据（删除前检索）：`rg -n 'build_volatile_context_messages|build_native_volatile_context_messages|volatile_context_note\(|build_native_memory_messages' crates` 仅命中定义与 `build_layered_turn_context` 内部调用；无外部消费者。
- 删除面的恢复条件：若将来需要在非 turn 场景单独构造易变段或净 memory 消息，从 `VolatileNotes` / `TurnPromptContent` 派生，不得重建「带角色包装的第二套渲染」。
- 残余：两个视图的截断决策仍各自独立（文本历史 vs 原生 transcript 口径不同，ADR 0123 的既有事实），极端超窗输入下截断数量可能不同；等价性测试只要求「同一层内容相同」，不要求「截断计数相同」。
- 观察到的既有 flake（非本次引入）：`build_request_truncates_older_history_to_context_window` 依赖共享 fixture `conversation_id = "session-1"`，并行跑全量 lib 单测时可能被同 session 的 goal/todo 全局态串扰；单独运行与串行运行均通过。已另行记录，不在本次改动范围内修改测试断言。
