# Agent Note: 上下文装配单程投影 + 工具自注册提示段

Status: proposed

## Problem

`crates/pony-agent-core/src/agent/context.rs::build_layered_turn_context`（L444–L702）把同一批语义层**重复构造三遍**：

- normalized 视图的 `reserved_messages`（L483–L495，仅供 token 预算估算，volatile 段传 `truncation_note = None`）；
- normalized 视图的最终消息序列（L525–L536，同一 volatile 段传真实 `truncation_note`，并额外产出 `project_instruction_messages` / `volatile_input_messages`）；
- native 视图（L548–L671，`base_system_messages` / `domain_profile_messages` / `semistable` / memory / transcript / volatile 全部再拼一遍，且 semistable 在 native 侧是 `system` 角色、normalized 侧是 `developer` 角色）。

同一语义因此在三处各有一份「角色 + 内容 + 顺序」的隐含约定，靠注释和 `debug_assert!(prompt_layers_are_ordered)` 兜底。已经付过两次代价：domain profile 在 native 侧要靠 `reserved_native_messages.len() - 1` 推导插入下标（L574），以及 Wave-61 C9 为消除门控二次求值导致的「false→true 窗口内 splice 越界」而做的补丁（L463–L469、L572–L583）。任何新增一层都必须同步改三处，漏一处就是 native/normalized 行为分叉，而现有测试只覆盖其中一侧的字符串断言。

同时，稳定层提示词仍把工具用法规约写在单体常量里（`TodoWrite` / `create_goal` / 团队工具），工具改名或降曝光只能靠人工记忆同步（已由 `implemented/process/2026-11-13-prompt-tool-consistency-gate.md` 的 PT1–PT5 兜底检测，但**检测不等于解耦**）。

## Proposal

1. **单程装配 + 双投影**：把一次 turn 的提示词建模为 `Vec<PromptLayer>`（每个 layer 携带 `content: String`、`normalized_role`、`native_role`、`stability: Stable | Semistable | Volatile`、`owner: LayerOwner`），由 `build_layered_turn_context` **只构造一次**；`flatten_normalized()` / `flatten_native()` 变成纯投影函数，不再各自拼装内容。
2. **预算估算复用同一份 layer 列表**：`reserved_messages` 的临时拼装删除，改为对 layer 列表求和；截断后的 `truncation_note` 以「追加一个 Volatile layer 再重估一次」表达，不再复制整段 volatile 构造。
3. ~~**工具提示段自注册（第二期，backlog B138）**~~ **已落地**（见 `implemented/architecture/2026-11-13-tool-owned-prompt-guidelines-and-templates.md`）：模板化 + 工具条款自注册，本提案只剩第 1/2 条（装配去重）。：新增 `tool guideline` 注册点，由工具模块声明自己的模型可见名与用法条款，`prompt_tool_references()` 由注册表派生；提示词不再硬编码工具名。前缀稳定性由「工具面恒定」的既有契约（`tools/names.rs` W42/Wave-61 C1 注释：曝光面不随运行时接线变化）保证，不改缓存语义。
   - 与 `implemented/architecture/2026-11-11-system-prompt-layered-architecture-and-receipts-guard.md` 记录的否决项「完全从提示词中剥离规划类工具约束（只留 schema）」**不同**：条款仍留在提示词里，只把**归属**从单体常量搬到工具模块，触发约束不会消失。
4. **验收方式**：以 `context_cache_red_contract.rs`（前缀逐字节相等）、`wave61_team_activation.rs`（层序 + 策略段位置）、`system_prompt_contract.rs`（SP1–SP10）为不变式护栏，先建立「三视图内容等价」的黑盒测试，再动手改实现，最后删掉重复构造。

## Alternatives considered

- **保持现状，只加更多断言**：成本最低，但每加一层仍要改三处；已有两次补丁证明这不是理论风险。断言只能发现「已经错」，不能降低「改错」的概率。
- **一次性按 DSH 注册制重写整个提示层**（`systemPrompt.section/context/tools/variable` + scope 遮蔽 + waterfall）：架构上最贴近参考实现，但会把 ponyd 的 `ProviderRole` 双轨、native transcript 重放、Anthropic 单 system join 全部卷进重构面，超出一次变更的可验证半径；且当前无行为缺陷驱动。
- **只做 native/normalized 去重、不碰工具段**：可作为第一期落地，收益集中且风险可控——本提案的第 1/2 条即此范围，第 3 条留作第二期。
- **把 native 视图整体废弃、只保留 normalized**：会丢掉 OpenAI 家族的 provider-native transcript 重放（ADR 0123 的既有决策），前缀缓存与 tool_call 结构都会变差，否决。

## Acceptance criteria

- 新增测试：同一组输入下，`flatten_normalized()` 与 `flatten_native()` 的语义层序列（去掉角色映射后）逐层相等；任一层缺失/错序/角色映射错配即为红。
- `context_cache_red_contract.rs` 的「跨轮前缀逐字节相等」持续为绿（改动不得引入新的前缀抖动）。
- `build_layered_turn_context` 内不再出现同一语义层的第二次字符串构造（可机械检查：`build_volatile_context_messages` 每 turn 调用次数从 2 降到 1）。
- `system_prompt_contract.rs` SP1–SP10 与 `prompt_tool_consistency.rs` PT1–PT5 不变红。

## Risks

- 前缀缓存回归是本提案最大的风险面：投影顺序一旦与现状不同，缓存命中率会静默下降；必须先落「等价性测试」再改实现。
- native 侧的 `system`/`user` 角色差异（semistable、volatile）是既有事实，单程建模必须显式表达 `native_role` 映射，否则会把两套 provider 语义压平。
- 工具自注册会引入「注册顺序即提示词顺序」的隐式耦合，需要像 DSH 的 `SECTION_ORDERS` 那样给出中央分配的 order 常量，否则提示词顺序会随模块加载顺序漂移，直接打穿前缀缓存。
