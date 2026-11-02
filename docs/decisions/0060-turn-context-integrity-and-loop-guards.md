# 0060 恢复请求保留完整上下文 + 轮次止损与请求形状可观测

Status: implemented

## 背景

对桌面端最近 14 条 `agent.turn` trace（Tempo `service.name=desktop-client`，2026-10-30 ~ 11-02）做逐条审计，暴露四类可复现实的缺陷：

1. **末轮回答丢失全部上下文**：trace `38590d52…`（session `pa_01a19b48` turn-3）第 7 次 `llm.chat`（即最终回答）input 由 18896 掉到 2311、`cache_read=0`、output 1663。根因是「重复工具调用」恢复路径把 `native_messages` 整体替换为 `[当前 user_message] + 本轮 hops`，而 native 流程下 provider 直接把该字段当 messages（`provider::openai_request_messages`）——system prompt、技能、记忆、历史对话全部消失，模型在无系统约束的状态下作答。
2. **工具参数契约自相矛盾**：job 系列工具的 JSON Schema 声明 camelCase（`jobId` / `timeoutMs`），serde 结构体字段却是 snake_case。模型两种拼法都试过，先后被 schema 预检（`unexpected argument job_id`）与 serde 解析（`missing field job_id`）拒绝，同一批 trace 里 9 次调用全部失败且反复振荡。
3. **长轮无早期收敛**：同一批 trace 里单轮 942s / 71 次调用 / 16 次工具失败；`JobOutput` 连续失败 5 次仍未被停——既有指纹闸是 `(工具, 入参指纹, 错误码)` 三元组，换参数或换错误码即可绕过。
4. **可观测性盲区**：上述「恢复请求换掉上下文」在 trace 上与普通 follow-up 完全不可区分，只能靠 token 断崖反推。

## 候选方案

- **方案 A：不修代码，只在文档里记现象**。落选：用户拿到的是丢上下文后的错误答案，属可复现的功能缺陷。
- **方案 B：恢复请求里单独补回 system prompt**。落选：技能/记忆/历史仍在丢失，只补 system 是半个修；且与正常 follow-up 的拼装路径分叉，后续必然再漂移。
- **方案 C（采纳）：恢复请求复用完整 transcript，并在 provider 出口标注请求模式**。恢复路径不再自建 transcript：`native_messages` 原样保留，被阻断的 assistant tool_call 与合成 tool result 通过 `accumulated_messages` 追加在尾部——与普通 follow-up 完全同构；同时新增 `llm.request.mode/messages/system_prefix` 三键，让「上下文被重建」在 trace 上可直接检索。
- **方案 D：给 job schema 写 `anyOf` 同时接受两种拼法**。落选：本仓 schema 预检器只实现 `type/required/properties/additionalProperties/items` 子集，扩语法面的收益远小于成本，且会让「schema 真值」变得难以推理。改为「入参在预检前归一到 schema 口径 + serde alias 兜底」。
- **方案 E：只把 `max_tool_followups_per_turn` 从 500 调小**。落选：既误伤合法长任务，又解决不了「同一工具打转」这一真实模式。

## 决策

1. **恢复请求保留完整 transcript**：`build_tool_followup_recovery_request` 只克隆 `planning_request`（清空 observation），不再重建 `native_messages`；`recover_tool_followup_completion(_stream)` 接收真实 `accumulated_messages` 并透传给 provider，使恢复请求 = 完整上下文 + 被阻断帧 + 合成 tool result。
2. **请求形状进 span**：新增 `LlmRequestMode`（`initial` / `tool_followup` / `tool_recovery`）与 `llm_request_shape()`（消息条数、首条是否 system，native transcript 优先、回退分层 input），落 `llm.request.mode` / `llm.request.messages` / `llm.request.system_prefix` 三键。既有 `provider_decision*` / `provider_followup*` 公开签名保留为兼容包装（转调 `*_with_mode`），验收用例零改动。
3. **job 参数双拼法**：模型来源的 tool call 在 schema 预检前把 snake_case 别名改写成 schema 口径（仅目标键缺席时），serde 侧同时加 `alias`，host/direct 调用同样吃得住；schema 语义不变。
4. **环境事实补口径**：`Environment:` 行新增 `shell=` 与平台命令语法口径（Windows 明确 dir/where/type/findstr、禁用 ls/cat/grep 与 `VAR=value cmd`），对应 trace 里 exit 9009 的重复失败。
5. **止损三道**：既有指纹闸（3 次同指纹）保留；新增「同一工具连续失败」闸（默认 4 次，忽略入参与错误码）；新增单轮墙钟预算（默认 900s），超预算走恢复式收尾给出可读结论而非硬失败。
6. **新增两个环境变量**：`PONY_AGENT_MAX_TURN_WALL_SECONDS`、`PONY_AGENT_MAX_CONSECUTIVE_SAME_TOOL_FAILURES`；非法/越界值一律回退默认，禁止静默取 0 导致闸门失效。

## 影响

- **用户可见行为**：末轮答案恢复完整上下文与系统约束；Windows 会话中工具参数/命令语法失败显著减少；单轮超过墙钟预算或同一工具连续失败时会**提前收尾并给出可读结论**（有意的行为变化，而非静默截断）。
- **遥测**：`llm.chat` 新增三条键（有值才输出，不做空占位）；归属仍在 tracing 唯一落点 `agent/otel/mod.rs`，`docs/observability.yaml` 已登记，`check-declaration.sh --project` PASS。
- **兼容性**：`provider_*` 旧签名保留，`crates/pony-agent-core/tests/` 内既有验收用例未改动即通过。
- **验收**：`cargo test --lib` 1209 passed；新增 6 例真实断言（请求形状纯函数、模式/属性单源、job 别名归一、双拼法解析、同工具止损、墙钟收尾含「预算原因随消息下发 + 答案送达 UI」）。证伪：把恢复逻辑退回旧实现后，新用例以「恢复请求 7 条消息塌成 5 条」失败，与线上 18896→2311 同因。
- **推迟项**：计价缺失（`workbuddy/deepseek-v4.1-flash` 未入价表，`pony.cost.priced=false`、预算治理空转）不在本篇范围。
