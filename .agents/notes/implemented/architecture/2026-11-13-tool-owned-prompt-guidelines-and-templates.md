# Agent Note: 工具自注册提示条款 + 稳定层模板化（B138 解耦）

Status: implemented

## Problem

稳定层提示词原先是一段单体常量，工具专名直接写在句子里（`TodoWrite` / `create_goal` / `update_goal` / 团队六件套 / `Run`）。工具名、别名与曝光面的真源在 `crates/pony-agent-core/src/agent/tools/names.rs`，提示词里那份是**第二份手抄**：改工具名、降曝光面、删工具时，提示词不会自动跟随，只能靠人记得同步。

`implemented/process/2026-11-13-prompt-tool-consistency-gate.md` 用 PT1–PT5 把这份耦合变成**可检测**：漂移会红。但检测不等于解耦——每次工具改名仍然要改两处（注册表 + 提示词常量），门禁只是让人不敢漏改。

## Decision

1. **稳定层改为「模板 + 占位符」**：新建 `crates/pony-agent-core/src/agent/system_prompt_template.rs`，存放 `BASE_PROMPT_TEMPLATE` 与 `TEAM_POLICY_TEMPLATE`。模板本体**不含任何工具专名**，只允许两种占位符：
   - `{{tool:<primitive>}}` —— 渲染时查 `tools::names::model_visible_tool_name_opt` 取模型可见名；
   - `{{guideline:<key>}}` —— 渲染时取工具自注册的条款正文；条款所属工具不在 `ModelVisible` 面时**整行删除**（不留空项目符号）。
2. **工具自注册用法条款**：`tools::prompt_contract::GUIDELINES` 里每条 `ToolGuideline { key, primitive, text }` 声明「哪个工具 owns 哪段说明」，`text` 内嵌 `{{tool}}` 占位。模板只放槽位，正文归工具侧。
3. **工具名清单改为派生**：`prompt_tool_references()` 不再人工维护，由「模板里的 `{{tool:…}}` + 条款表的 primitive」自动生成；PT1/PT2（存在性、曝光面）继续守门。
4. **入口与缓存不变**：`context::{base_system_prompt, agent_team_policy_prompt}` 是再导出入口（契约面不动），渲染结果用 `OnceLock` 缓存一次、逐轮恒定，稳定前缀与 provider 前缀缓存行为不变。
5. **注入门集中**：`system_prompt_template::team_policy_injected()` 委托 `tools::subagent_runtime_wired()`，`context::agent_team_policy_enabled()` 再委托它——wave-61 C5-A3「锚串所在文件必须复用同一判定」继续成立，没有第二套接线判定。
6. **新增 PT6**：直接检查模板原文（剥掉占位符后）不含任何工具专名，且渲染后不残留 `{{`。这一步不可省：若有人把工具名写回模板，渲染结果不变、PT1–PT5 仍全绿，但解耦已失效——PT6 是唯一能发现它的断言。

**渲染结果逐字节不变**：BASE 3790 字节、团队策略段 2624 字节（改动前后实测相等），故模型可见行为零变化，`SP1–SP10`、`wave61`、`context_cache_red_contract`、`wave_plan_goal_acceptance` 全部原样通过。

## Alternatives considered

- **按 DSH 的 section registry 拆成多条独立 System 消息 + 数字 order 常量**：结构上最可组合，但会改变渲染字节与消息条数（动到契约 §2、`reserved_system_message_extra_count()` 的 trace 判定与缓存前缀），而本次没有任何行为缺陷需要改变；模板槽位把顺序表达在文本行序里，比数字 order 更强——没有 tie-break、不依赖模块加载顺序。
- **保持人工清单，只加强检测**：就是 B138 之前的状态。检测能让人不敢漏改，但每改一次工具名仍要动两处，“解耦”名不副实。
- **`base_system_prompt()` 改签名返回 `String`**：调用方与契约入口全要改，且每轮重新渲染带来前缀抖动风险；`OnceLock` + 保持 `&'static str` 用零签名变更拿到同样的解耦。
- **让渲染依赖接线状态**（未接线时返回空串）：渲染是确定性资产，注入才是门控。把两者混在一起会让缓存、快照与测试都无法独立验证。
- **把条款正文散进各工具源码文件**：本仓内建工具没有独立模块（`tools/schema.rs` 统一声明、handlers 分散），散过去只会得到一堆 `pub` 常量继续被 `prompt_contract` 引用，收益低于跨文件噪声；集中一张表是当前工程形态下的最小可行归属，未来工具拆模块时按条目平移即可。

## Consequences

- 改工具名/曝光面只改 `tools/names.rs`：提示词自动跟随或整条消失；改名后若模板引用了不存在的 primitive，渲染即 `panic`（PT1/PT6 在测试期先炸）。
- `context.rs` 净减 29 行（3003 → 2973），缓解 `size:check` 的存量基线压力（该文件此前已顶到基线，净增任何行都是新违规）。
- 契约升到 `v2.2.0`：新增 §8「模板与占位符」与 PT6；渲染文本、SP1–SP10、字节登记值均未变。
- 已知残余：条款正文仍集中在一张表里（非散落各工具文件）；`{{tool:…}}` 解析失败选择 `panic` 而非降级，属开发期错误快速失败——模板是编译期常量，PT6 会在 CI 先行拦截。
