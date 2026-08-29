# 提供商协议徽章化 + OpenAI Responses 适配 + 模型目录拉取

## 为什么现在

用户对配置页"模型" tab 提出六点迭代：提供商编辑行收敛为「名称 | 协议 | API Key」同级；协议改为三个徽章选项（openai-responses / openai-completions / anthropic-messages），选择即启用，去除认证方式等杂项；新增模型支持经 `/models` 接口拉取 ID 多选与手动输入，并增加高级设置（模型级协议默认 openai-completions）；同一模型跨协议不同 base_url 的场景在高级设置中适配；全部输入框背景色与卡片底色区分；编辑未保存即收起折叠的自动取消编辑态。当前后端仅有 completions / anthropic messages 两条通路，需要补 Responses 协议适配。

## 变更范围

- 后端（crates/pony-agent-core、src-tauri）：
  - `ProviderProtocol` 三值化并兼容旧值读取；
  - 新增 Responses API 请求构建/SSE 解析/工具环接线；
  - 新增 `fetch_provider_models` Tauri 命令；
  - `resolve_selection` 尊重模型级 protocol/base_url 覆盖。
- 前端（src/types、src/stores/providers.ts、ProviderConfigPage.vue）：
  - 协议类型三值化 + 旧值规范化；
  - 提供商编辑行重构 + 高级设置（每协议 Base URL）；
  - 新增/编辑模型表单：ID 目录多选下拉 + 手动输入 + 高级设置（协议/Base URL 覆盖）;
  - 输入面统一背景；收起即取消编辑。

## 非目标

- 不改变 providers.json 的存储位置与密钥存储机制。
- 不引入除 /models 外的其他目录类接口（如 /fine_tunes），不做分页拉取。
- 不改动遥测/指标页对历史 trace 中旧协议字符串（"openai"/"anthropic"）的展示逻辑。
- 浏览器预览模式不触网，目录拉取仅返回提示性错误。

## 验收标准（摘要，细则见 tasks.md）

1. 旧 providers.json（含 "openai"/"anthropic"）可加载，且保存后全部协议位（provider/supportedProtocols/endpoints/models）写出新规范名；**已知风险（ADR 登记）**：旧版本应用读取新文件会静默回落默认注册表——serde 别名只能"新读旧"，降级后再保存将以默认模板覆写用户配置。
2. 选择 openai-responses 徽章后，决策/工具跟进（同步+流式）走 `{base_url}/responses` 并正确解析文本/工具调用/用量；多 function_call 取首个并记录丢弃数。
3. 模型表单点击"获取列表"按高级设置协议与 Base URL 拉取 `/models`，成功填充多选下拉；手动输入始终可用；批量添加经独立 action 预去重并提示「已添加 N 个，跳过 M 个」。
4. 模型级协议或 base_url 覆盖生效于运行时 resolve_selection。
5. 收起任一承载未保存编辑的折叠区（含手风琴连带收起）自动取消编辑态，无"保存指向不可见表单"死状态。
6. 全部 vitest / vue-tsc / cargo check / cargo:test:shared 通过。
