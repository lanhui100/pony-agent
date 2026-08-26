# 技术方案：协议三值化 + Responses 适配 + 模型目录

## D1 协议命名与兼容（跨模块契约）

规范名（wire 值，前后端与 providers.json 一致）：

- `openai-responses` —— OpenAI Responses API（`POST {base_url}/responses`）
- `openai-completions` —— OpenAI Chat Completions（现 `openai` 的更名）
- `anthropic-messages` —— Anthropic Messages（现 `anthropic` 的更名）

Rust：

```rust
pub enum ProviderProtocol {
    #[serde(rename = "openai-completions", alias = "openai")]
    OpenAiCompletions,
    #[serde(rename = "openai-responses")]
    OpenAiResponses,
    #[serde(rename = "anthropic-messages", alias = "anthropic")]
    AnthropicMessages,
}
```

- 反序列化接受旧值 `openai`/`anthropic`（旧 providers.json 与旧 trace 回放可读）；序列化只写新名。**降级兼容为已接受的残余风险**（旧版应用读新文件会失败），在 ADR 中明示。
- 新增家族助手，替代散落的字符串/枚举比较：
  - `ProviderProtocol::is_anthropic() -> bool`
  - `ProviderProtocol::is_openai_family() -> bool`（completions + responses）
  - `protocol_family() -> ProviderProtocolFamily`（枚举 `{OpenAi, Anthropic}`）：**`provider_native_tool_result_message_for_protocol`、`provider_native_assistant_tool_call_message_for_protocol` 的签名改为接收该家族枚举（或 `&ProviderProtocol`）**，消除裸 `&str == "anthropic"` 分叉的静默错路由风险（provider/mod.rs:2726/2771、tool_recovery.rs:528、turn_prep.rs:692-696 全部改传枚举）。
  - `protocol_label()` 返回规范名，仅用于遥测/trace 写入（turn_flow.rs:471、turn_sync.rs:926、turn_stream.rs 各 patch 点保持规范名字符串；历史 trace 旧串不迁移，指标页新旧并存属预期）。

全量穷尽匹配位点（Rust 编译器兜底）：provider/mod.rs 决策/跟进四路分发、流式门控（responses 与 completions 同为真流式 + native tool flow）、config.rs 默认 auth（anthropic→x-api-key，openai 族→auto/bearer 按既有两处映射各自保持）、默认 base_url（responses 复用 `https://api.openai.com/v1`）、默认模型 ID（gpt-4.1-mini）、`resolve_thinking_param_pattern`（responses 走 completions 同分支）、CAPABILITY_CATALOG 的 protocol 字符串匹配改为按家族匹配。

前端 TS：

```ts
export type ProviderProtocol = "openai-responses" | "openai-completions" | "anthropic-messages";
```

- 加载规范化：`normalizeLegacyProtocol(value)`：openai→openai-completions、anthropic→anthropic-messages，未知值回落 openai-completions；应用于 registry 载入、endpoints、supportedProtocols、model.protocol 全部入口（store `normalizeProvider` 单点收敛）。
- **前端 CAPABILITY_CATALOG（providers.ts:27-53）同构改为按家族匹配**（openai 族两值均可命中 "openai" 条目），否则三值化后能力推断整体回落 auto（R5）。
- `runtime.ts:3977` 浏览器预览兜底值 `"openai"` → `"openai-completions"`，避免改名后新 trace 仍产出旧名（R5/P2-4）。
- 左列迷你徽标映射（ProviderConfigPage.vue:855-860）与二元 `protocolLabel()`（:785-787）删除/重写为三值规范名直显，防未知值全渲染成 Anthropic。
- 保存即写规范名。`ProviderAuthType` 类型保留（存储兼容），UI 不再暴露选择；**新建 endpoint 写 `auto`；保存时保留既有非 auto 的 authType 不静默改写**（显式 Bearer 覆盖语义仍有效），请求期由后端按家族解析。
- 徽章文案即规范名本身（等宽小字徽标），不再用「OpenAI 协议」中文标签。

## D2 后端 Responses 适配

**存储与传输中枢统一为 chat 形态**（架构裁决 R1）：`assistant_message` 在 sync/stream 两路都产出 `{role:"assistant", content, reasoning_content?, tool_calls[]}` 形态（与 completions 路径一致）；转换只发生在 `send_responses_*` 请求边界。tool_calls[].id 一律取 Responses wire 的 **call_id**（禁用 output item id），sync 与 stream 同一口径——否则工具跟进 `function_call_output.call_id` 引用错位必挂 400。

新文件 `crates/pony-agent-core/src/agent/provider/responses_api.rs`（纯函数为主，便于单测）：

请求体（sync）：

```json
{
  "model": "...", "input": [ ...items ], "stream": false, "store": false,
  "max_output_tokens": N, "temperature": f,
  "tools": [{ "type": "function", "name": ..., "description": ..., "parameters": {...} }],
  "tool_choice": "auto"
}
```

- input items 由 `ProviderRequest` 构造：
  - system/developer → `{role, content:[{type:"input_text", text}]}`；
  - user → `input_text`；最后一条 user 携带图片时追加 `{type:"input_image", image_url:<data-url>}`（对齐 completions 路径的图片口径）；
  - `native_messages`（chat 形态累积转录）→ 转换器 `chat_messages_to_responses_input`：assistant 文本 → `{role:"assistant", content:[{type:"output_text",text}]}`；`tool_calls[]` → `{type:"function_call", call_id, name, arguments}`；role=tool → `{type:"function_call_output", call_id, output}`。
  - **外来形态守卫**：输入含 anthropic blocks 形态（content 为数组且首块 type ∈ text/tool_use/tool_result）时，采用与 anthropic 侧外来 transcript 相同的 skip-and-rebuild 语义（丢弃外来形态项，从 request.input 重建），禁止产出非法 payload；跨协议切换会话允许丢上下文（现状语义），补测试钉死。
- reasoning 模型（capabilities.supports_reasoning 且 effort 有值）附加 `"reasoning": {"effort": ...}`；temperature 在 reasoning 模型时省略（o 系/GPT-5 拒收），与 with_openai_request_options 的现状差异在 ADR 中记录。

响应解析（sync）：优先顶层 `output_text`；否则遍历 `output[]`：`message.content[].output_text` 拼接、`function_call` → ToolCall{call_id:**call_id 字段**, name, arguments 解析}、reasoning 汇总文本。**多 function_call 取首个，provider_log 记录丢弃数**（决策单数契约不变）。usage 映射 `{input_tokens, output_tokens, total_tokens, reasoning_tokens?}`；**若映射 `input_tokens_details.cached_tokens` 进 cache_hit_input_tokens 必须同时设 cache_hit_source="responses"**，否则触发 build_provider_call_cache_record 的 panic 契约（stream_support.rs:185-190）——B3 单测含此断言。

流式：**先抽公共帧阅读器 `sse_data_line_reader`（字节缓冲 / 1MB 上限 / `data:` 提取 / 尾行冲刷 / elapsed 错误包装）供 completions 与 Responses 两套 accumulator 复用，拒绝第二份帧循环**（架构裁决 R5）。Responses 累积器只实现 push_payload/finish：

- `response.output_text.delta` → Text(delta)
- `response.reasoning_summary_text.delta` / `response.reasoning_text.delta` → Reasoning(delta)
- `response.function_call_arguments.delta` → 按 item_id/output_index 累积参数
- `response.output_item.done`（type=function_call）→ 兜底补全 name/**call_id**
- `response.completed` / `response.response.completed` → 取 usage 后结束；`[DONE]` 容错结束
- 终态校验同 completions：无文本且无 tool_call 报错；多槽 function_call 同样取首个并记录丢弃数。

ProviderManager 接线：`ProviderProtocol::OpenAiResponses => send_responses_*` 四路（decision sync/stream、followup sync/stream）；stream 失败回退 sync、sync 失败本地 fallback 的既有语义保持一致；endpoint 为 `{base_url}/responses`，鉴权按 D4 解析序（默认 Bearer）。

## D3 /models 目录拉取

新文件 `crates/pony-agent-core/src/agent/provider/model_catalog.rs`：

```rust
pub fn fetch_model_ids(protocol, base_url, auth_hint, api_key: &str) -> Result<Vec<String>, String>
```

- `GET {base_url}/models`，15s 超时；头按家族：openai 族 `Authorization: Bearer k`；anthropic `x-api-key` + `anthropic-version: 2023-06-01`。
- **error-for-status + 内容嗅探**：非 2xx 报错带状态码与正文预览；200 但 body 非 JSON（HTML 错误页/纯文本）→ 显式报错而非返回空列表，杜绝"失败被呈现成空目录"。
- 解析容错：`data[].id`（OpenAI/Anthropic/OpenRouter 通例）、根数组 `[{"id"}]`、`models[].id`；去重排序，上限 1000 条截断并在错误信息外附提示字段（返回结构改为 `ModelCatalogResult { models, truncated }`？——否，保持 `Vec<String>`，截断静默但日志 provider_log 记录，避免前端新增类型）。
- API Key 不入日志；错误信息不含 key。
- Tauri 命令（src-tauri/lib.rs）：

```rust
#[tauri::command]
fn fetch_provider_models(provider_id: Option<String>, protocol: ProviderProtocol,
                         base_url: String, api_key: Option<String>) -> Result<Vec<String>, String>
```

key 解析顺序：显式入参 → provider_id 对应已存密钥 → 报错"缺少 API Key"。注册进 generate_handler!。

## D4 模型级协议 / base_url 覆盖

- Rust `ProviderModelConfig`（View+Storage 两份）增加：

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub base_url: Option<String>,
```

- **模型协议 ⊆ 提供商已启用徽章集合**（架构裁决 R3，关闭 P0-1 矛盾态）：前端高级设置协议下拉只列 provider 已启用协议（含当前值兜底）；后端 `resolve_model_protocol` 的回落规则保持不变；保存后以命令返回的 normalized view 重渲染，杜绝内存态/落盘态分叉。不做"自动启用端点"魔法。
- 关闭徽章时若存在引用该协议的模型，UI 就地警示「N 个模型正在使用该协议，保存后将回落主协议」（不阻断、不静默）。
- `resolve_selection` 解析序（架构裁决 R4，保住 SenseNova 式显式 auth 用例）：
  1. effective_protocol = model.protocol ∈ supported_protocols ? 它 : provider 主协议；
  2. base_url = model.base_url 非空 trim → 用之；否则该协议 enabled endpoint.base_url；否则 provider.base_url（协议一致时）/ default_base_url 兜底；
  3. **auth_type = 定位到的 endpoint.auth_type 非 Auto 则尊重之，Auto 才按家族推导**；
  thinking_param_pattern 与能力目录按 effective protocol 家族解析。
- TS `ProviderModelIdentity` 增加 `baseUrl?: string | null`；store normalizeModel 单点清洗（trim 或置 null）；保存/加载往返无损。

## D5 提供商编辑行 + 高级设置

编辑态（提供商 section 内）单卡三段同行（窄屏纵向堆叠）：

```
[提供商名称 input]  [协议: (openai-responses)(openai-completions)(anthropic-messages)]  [API Key password input]
```

- 徽章为 toggle 按钮：选中=启用该协议（aria-pressed），至少保留一个启用方可保存；默认新建仅 openai-completions 启用。
- 「高级设置」折叠（默认收起）：对每个已启用协议给 Base URL 输入（预填当前值或默认值）。去除认证方式下拉与每协议卡片开关（由徽章取代）。
- 视图态同步改排：名称 / 协议徽章 / API Key 状态一行化，「密钥与环境」「协议入口」区块合并精简，文案「协议入口」→「协议」。

## D6 模型表单（新增/编辑共用标记，双份受控重复维持 ADR 0013 决策）

- 模型 ID 区：
  - 「获取列表」按钮 → store.fetchModelCatalog({providerId, protocol: 高级设置当前协议, baseUrl: 高级设置覆盖或解析值, apiKey: 表单未保存 key})；loading 态禁用；错误就地显示；**浏览器预览模式由 isTauriAvailable 守卫直接提示"预览模式不可用"，不触网**（F1 认领并配测试）。
  - 下拉面板：搜索框 + checkbox 多选列表（选项来自 catalog），选中项以 chips 呈现可移除；无 catalog 时面板隐藏。
  - 手动输入 Input 始终可用（单值路径）。
  - 批量添加：**走专用 store action `addModelsFromCatalog`，预去重后批量 upsert 并返回 `{added, skipped}`**，不复用 upsertModel 的 this.error 错误通道（避免全局红色横幅误报）；notice 文案「已添加 N 个模型，跳过 M 个已存在（同 ID 按别名折叠）」；新增模型 name=id、默认能力、高级设置协议/baseUrl。
  - 编辑已有模型：ID 字段退化为单选手动输入（保留建议下拉单击填充，不出多选）。
- 高级设置折叠（默认收起）：
  - 协议 select：**仅列提供商已启用协议（含当前值兜底）**，默认 openai-completions；
  - Base URL 覆盖 input：空 = 使用提供商该协议 endpoint URL（placeholder 展示实际解析结果）。
- 能力/参数区维持 ADR 0013 迭代四形态不动。

## D7 输入面背景

- 页面 scoped 样式统一：`.config-form :deep(input)` 已有白底；补充 `.config-form :deep(select)`、`.config-form :deep(textarea)` 同底色（rgb(255 255 255 / 0.82)，focus 0.98），新增的自绘下拉 trigger/面板显式 `bg-white` + ring，保证与 white/72 卡片底色可见区分；Input.vue 默认 bg-white 保持。

## D8 收起自动取消编辑

- **任何使承载活动编辑/创建的 section 或展开行收起的翻转（含手风琴从另一区触发的连带收起）都先执行该区取消**，再完成折叠——不是"被点击的区"判断（边界审核 R3/P1-1：现状 `toggleProviderSection` 只查自身锁，模型编辑中点提供商区头已存在"表单被收起、保存不可见"死状态，本变更一并修复）。
- create 态收起 = **放弃并复位**（清空表单），title 已警示；重新展开为空白新建表单。
- 孤儿半配置防护（P1-2 放大路径）：`saveProviderForm` 在 create 模式首次 addProvider 后将 editorState.mode 切为 "edit"，失败重试复用同一 providerId，不再二次 addProvider 产生孤儿提供商。
- 模型行：展开行处于编辑态时再次点击收起 → 取消编辑并收起；点击其他行同样先取消当前编辑再切换目标。
- 行 title 提示改为「收起将放弃未保存的修改」；手风琴互斥保留；左列提供商切换维持既有覆盖语义并补测试钉住现状。
- 必须重写的既有用例：spec.ts:291-310「锁定期间点击模型列表头」整条改写为「收起即取消创建」；:395-418 中段锁定断言改为取消断言；:209-212 取消按钮语义与 :187-191 双击收起保持不变。

## D9 指针与行尾动作悬停显隐（同日用户补充）

- 一级折叠区头与模型行整行 hover 显示 pointer 光标：外层容器已有 `cursor-pointer`，但行内占位的原生 `<button>` 会以浏览器默认光标覆盖——给两级行内的 trigger `<button>` 显式补 `cursor-pointer`。
- 空闲态行尾图标动作（提供商头的 编辑/删除、模型列表头的新增模型、模型行尾的 编辑/删除）默认隐藏，仅在该行 hover（或键盘 focus-within）时显现：
  - 行容器加 `group`；动作簇容器加 `opacity-0 invisible group-hover:opacity-100 group-hover:visible focus-within:opacity-100 focus-within:visible transition`；
  - 编辑态的 取消/保存 文字钮不参与悬停显隐（常显，避免"保存不可见"死状态）；ConfirmPopover 弹层挂在 body 侧不受影响；
  - 触屏/键盘可达性由 focus-within 分支兜底。

## D10 编辑形态迭代五（同日用户二次补充）

- **提供商编辑改单栏排列**：名称 / 协议徽章行 / API Key 各占一栏（label 在上、输入框整行）；高级设置内每协议 Base URL 输入框改为单列全宽，消除三列挤压。
- **模型 ID 目录交互重做**：
  - 刷新按钮收编为输入框右端内的纯图标按钮（RefreshCw），旁为目录展开/收起 chevron；
  - 获取成功自动展开下拉面板（搜索 + 多选 checkbox + 批量添加按钮维持 D6 契约；编辑态为单击填充的单选列表）；
  - 选择器外侧新增 info 图标：常态 tooltip 说明刷新用法与批量选择方式；拉取失败时图标切换为失败语义（AlertTriangle、rose 色），tooltip 内容替换为具体错误信息，独立错误段落移除；
  - 测试：info/fail 语义切换用例 + 占位符同步更新。

## 风险与回滚

- 协议改名是持久化格式演进：读路径别名兜底；**降级矩阵如实描述**——旧版本应用读新文件会在 `load_storage`（config.rs:400-408）反序列化失败并**无声回落默认注册表**，再次保存即以出厂模板覆写用户配置、keychain secret_ref 成孤儿。缓解：`load_storage` 解析失败时先把原文件另存 `providers.json.invalid-<ts>` 再回落默认（架构裁决 R6），ADR 0014 登记完整矩阵。
- Responses 适配不影响存量 completions/anthropic 请求通路（分发隔离在新模块）；但 protocol_label 改值自合并当刻起改变新 trace 的落盘字符串，指标页新旧分组并存属预期，后续可按 family 二次聚合（P3 不阻塞）。
- /models 命令是新增出网读取面：仅 GET、带超时与 error-for-status、key 不落日志。

## 测试计划

- Rust：
  - responses 请求构建 / chat→responses 转换（含外来形态 skip-and-rebuild）/ SSE 累积 / sync-stream call_id 同口径解析单测；
  - usage cache_hit_input_tokens ⇒ cache_hit_source 断言（panic 契约）；
  - model_catalog 解析容错（data[]/根数组/models[]/非 JSON 报错）与鉴权头单测；
  - config 别名**四位置往返**（provider.protocol、supportedProtocols[]、endpoints[].protocol、models[].protocol 各含旧值 → 读 → 存 → 全部规范名）；resolve_selection 覆盖生效 + endpoint 显式 auth 尊重单测；load_storage 失败备份行为单测。
- Vitest：store 规范化/覆盖字段/fetchModelCatalog 四路径（浏览器守卫/无 key/超时文案/非 JSON）/addModelsFromCatalog 返回 {added,skipped}；页面测试徽章开关、徽章关闭时模型引用警示、高级设置绑定、多选批量添加、收起取消编辑（含手风琴交叉折叠）、create 收起复位、输入面背景 smoke、悬停显隐 class 断言。
- 受影响既有前端测试面（F6 清单）：runtime-store.spec.ts（约 35 处 fixture）、HomeWorkspace.spec.ts:256/466、HomeSidebar.spec.ts:65、HomeWorkspaceMarkdown.spec.ts:64、workspace-composer-attach.spec.ts:63-125、TraceInspector.spec.ts:65/794、ModelMonitorPage.spec.ts:34/70、ProviderConfigPage.spec.ts（多处）、providers.store.spec.ts（20+ 实参）。vue-tsc 不覆盖 tests，以上靠运行期断言兜底。
- 门禁：npm run test:unit、vue-tsc --noEmit、cargo check:shared、**cargo:test:shared**（G4 从 regression 目标集升级为全 crate 测试，覆盖 config.rs 内嵌 #[cfg(test)] 与新模块单测）。

## 审核记录

两轮独立对抗审核（边界回归向 f66793f5、架构契约向 0b354df0）均「有条件通过」，无相互冲突意见；架构报告对 P0-1 与持久化契约已给出唯一裁决案，编排者裁定采纳全部 R1-R8/R1'-R7' 并落入本文 D1-D9，不另起 consultant 轮（无冲突仲裁需求、决策可逆、数据面有备份缓解）。关键采纳：

| 意见 | 处置 |
|---|---|
| P0-1 模型协议 × 徽章启用耦合 | 采纳 R3'：下拉限已启用集合 + 以返回 view 重渲染 + 关徽章警示 |
| P0-2 降级覆写链 | 采纳 R2+R6：验收改写 + load 失败先备份 invalid 文件 |
| P1 字符串分叉 | 采纳 R4/R2'：家族枚举签名，漏改编译错误 |
| P1 编译器盲区四类 | 采纳 R7'：B1 验收附 grep 门禁清单（matches! 单臂门控、endpoints vec 构造、catalog 通配符、TS 比较） |
| P1 call_id 两路口径 | 采纳：sync/stream 统一 wire call_id |
| P1 usage panic 契约 | 采纳：cache_hit_source 强制同设 + 测试断言 |
| P1 跨族 transcript 形态 | 采纳 R1'：chat 形态中枢 + 外来形态 skip-and-rebuild |
| P2 SSE 双骨架 | 采纳 R5'：抽公共 sse_data_line_reader |
| P2 批量添加 error 误报 | 采纳：专用 action 返回 {added,skipped} |
| P2 auth 忽略 endpoint 显式值 | 采纳 R4'：非 Auto 尊重 |
| 其余 P2/P3（截断标记、指标聚合、探针波及） | 记录为已知项/后续优化，不阻塞 |

### 代码阶段双审采纳（code-reviewer-a 正确性 / code-reviewer-b 边界安全，均「有条件通过」）

| 意见 | 处置 |
|---|---|
| A-P1-1 两一级区头缺 `group` 致悬停显隐对鼠标失效 | 已修：容器补 group + 区头动作簇断言 |
| B-P1-1 组件漏传未保存 apiKey（D6 偏离） | 已修：providerFormContextId 守卫下回传（防 A 草稿密钥发往 B 的 URL）+ invoke args 断言 |
| B-P1-2 provider_api_key 静默回退 selected/first | 已修：精确 id 匹配，未命中 None；解析链单测钉死 |
| A-P2-1 启用 responses 徽章静默翻转主协议 | 已修：既有主协议仍启用则保持首位；create 态按徽章序 |
| A-P2-2 SSE 不处理 response.failed/incomplete/error | 已修：失败终态显式 Err（含部分文本场景），杜绝二次扣费回退 |
| A-P2-4 sse_data_line_reader 零覆盖 | 已修：抽 read_sse_data_lines 通用核心 + CRLF/跨包/尾冲刷/1MB/提前结束 五组直测 |
| A-P3-1 外来守卫补 "thinking" 首块 | 已修 + 注释说明构造顺序依据 |
| A-P3-2 空 item_id 分片碎片化 | 已修：空 id 归并首个 partial；done 收养唯一未完成宿主（附测试） |
| B-P3-4/A-P3-7 /models 不尊重 endpoint 显式 auth | 已修：provider_endpoint_auth 解析链接入命令 |
| B-P3-3 空 baseUrl 直调产生误导错误 | 已修：命令层校验并给可操作提示 |
| A-P3-5 回退日志硬编码旧名 | 已修：openai-completions 规范名 |
| B-P3-1 invalid 备份跨秒累积 | 已修：同前缀已存在则跳过 |
| A-P2-3/B-P2-1 store 级前端测试承诺未兑现 | 已补：normalizeLegacyProtocol 表、四位置收敛（saveRegistry 浏览器分支）、fetchModelCatalog 四路径+args、addModelsFromCatalog {added,skipped,lastAddedModelId}+别名折叠；runtime-store fixture 迁移规范名 |
| A-P3-4/6、A-P3-11/12、B-P3-2/5/6、A-P3-8 eprintln 噪音等 | 记录为已知项：与 completions 同基线或 UI 当前不可达，不阻塞合入 |
| B-安全清单九项 | 全部通过或接受现状（SSRF 与 chat 同基线属有意设计，已在 ADR 0014 影响节登记） |
