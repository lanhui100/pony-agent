# 0014 提供商协议三值化与模型目录：Responses 适配、模型级覆盖与收起取消编辑

Status: implemented

## 背景

配置页"模型" tab 的提供商/模型管理在 ADR 0013 的双折叠区布局上继续迭代：协议需要区分 OpenAI Chat Completions 与 Responses API 两族并以三个规范名徽章呈现；新增模型要求经 `/models` 目录拉取批量选择；同一模型可跨协议复用但 Base URL 不同；同时用户裁决推翻 0013 中"编辑中锁定折叠"的交互（改为收起即取消编辑）。后端 `ProviderProtocol` 原为 `openai`/`anthropic` 二值，`resolve_selection` 忽略模型级 protocol 字段（惰性装饰），providers.json 无版本演进机制。

## 候选方案

**协议命名与持久化演进**

- 保留 wire 值 `openai`/`anthropic` 不变、仅 UI 换标签：落选——存储与界面两套名字需终身映射，`model.protocol` 落盘值与徽章展示不一致，排查成本永久化。
- **规范名迁移 + serde alias 只兜读取（采纳）**：wire 值改为 `openai-completions`/`openai-responses`/`anthropic-messages`，反序列化 alias 接受旧值，序列化只写新名。已知降级矩阵如实登记：旧版应用读新文件会在 `load_storage` 反序列化失败并**无声回落默认注册表**，再次保存即覆写用户配置。
- version 字段门禁 + 升级提示：落选——单机桌面应用无多版本并存的运行场景，字段成本高于收益；缓解改由"解析失败先把原文件另存 `providers.json.invalid-<ts>` 再回落默认"承担（可恢复、不静默丢数据原件）。

**Responses 适配分层**

- 在 mod.rs 内联第三组 send_* 函数并复制 SSE 帧循环：落选——帧循环（字节缓冲/1MB 上限/尾行冲刷/elapsed 包装）是最难测的边界代码，第二份必然漂移。
- **抽公共 `sse_data_line_reader`，completions/responses 共用；请求边界转换器 + chat 形态中枢（采纳）**：`assistant_message` 在 responses 全链路统一产 chat 形态（tool_calls[].id 取 wire `call_id`，sync/stream 同口径），native transcript 与 completions 可互换；转换只发生在 `send_responses_*` 边界。外来 anthropic blocks 形态输入采用 skip-and-rebuild 守卫，禁止非法 payload 出网。
- responses 原生 items 形态作为持久化 transcript：落选——会话跨协议切换时 transcripts 三形态互不可读，且现有 anthropic 侧守卫语义要复制三份。

**模型级协议 × 提供商徽章的耦合**

- 下拉无约束三值 + 后端自动启用对应 endpoint：落选——激活既有惰性 `model.protocol` 后会产生"UI 显示 A 运行时走 B"或反向静默批量改写的矛盾态（spec 审核 P0）。
- **模型可选协议 ⊆ 提供商已启用徽章集合；保存以命令返回的 normalized view 重渲染；关闭徽章时有模型引用则就地警示（采纳）**：与后端 `resolve_model_protocol` 回落规则天然一致，无魔法联动。
- base_url/auth 解析序定为 model.base_url > enabled endpoint > provider.base_url > default；endpoint 非 Auto auth_type 尊重（保住 SenseNova 式显式 Bearer 覆盖），Auto 才按家族推导。

**折叠与编辑语义（修订 0013 子决定）**

- 维持 0013"编辑中锁定所在区折叠"：落选——实测仍存在手风琴交叉折叠死状态（模型编辑中点提供商区头），且锁语义阻断用户预期的"收起=离开"。
- **任何使承载活动编辑的区/行收起的翻转先取消该区编辑再折叠（含连带收起）；create 态收起=放弃复位；create 首次落地即切 edit 态防孤儿提供商（采纳）**。行尾空闲动作簇改为 hover/focus-within 显隐，编辑态按钮常显。

## 决策

协议三值化以规范名落地并兼容旧读；Responses 经公共 SSE 骨架接入、chat 形态为存储中枢；模型目录经 `fetch_provider_models` 命令拉取（error-for-status、非 JSON 报错、key 不入日志）；模型级覆盖按 D4 解析序生效且受徽章启用集合约束；折叠交互改为收起即取消编辑。行为契约见 openspec `workspace-shell-navigation` 与 `provider-registry` 两份 delta。

## 影响

- providers.json 自本次保存起写出规范协议名与新 `models[].baseUrl` 字段；降级到旧版应用存在"回落默认注册表"风险（备份文件 `providers.json.invalid-<ts>` 可人工恢复）。
- 新增出网读取面 `{base_url}/models`（GET、15s 超时、按家族鉴权）；遥测/指标页将并存新旧协议字符串分组，后续可按 family 二次聚合。
- ADR 0013 中"编辑中锁定所在区折叠""行尾动作常显"两条迭代子决定自本篇起不再代表现行交互，其余承接有效。
