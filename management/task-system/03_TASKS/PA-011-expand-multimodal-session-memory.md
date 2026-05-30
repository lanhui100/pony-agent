# PA-011 扩展多模态会话记忆与附件生命周期

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## 目标
在已完成“当前轮真实图片输入 MVP”的基础上，补上跨轮可继续引用的附件摘要、生命周期和 session 语义，让用户可以自然地继续问“刚才那张图”。

## 输出
- 附件元数据结构
- session 持久化策略
- recent image recall 规则
- 前端附件展示与恢复策略

## 验收标准
- 用户发送图片后，下一轮可以自然继续追问最近那张图
- session 中不把附件元数据与普通文本消息串乱
- provider context 能区分“真实图片仍可访问”与“只剩文本提及图片”
- 前端恢复历史会话时，用户消息附件可见

## 当前进展
- 前端已支持当前轮附图输入
- Rust runtime 已把当前轮图片 data URL 接入 OpenAI / Anthropic 请求体
- `TurnHistoryMessage` 已新增 `attachments`
- `SessionStore` 已新增 `SessionAttachment` 持久化
- 附件 payload 已按 session 写入本地附件目录，消息历史里保存附件元数据
- runtime 已能在引用图片时从最近一次带附件的用户消息里召回图片
- 前端已用 `displayMessage` 保存“文本 + 附图摘要”的显示消息
- 历史消息区已能显示附件 chip
- 前端回放 history 时会过滤掉没有 `relativePath` 的占位附件，避免把未持久化附件再次发回后端
- recent image recall 现已收紧到“最近一轮 user turn 本身带附件”这一最小安全边界
- 同步 `run_turn` 与流式 `start_turn_stream` 现在对附件持久化失败统一 fail-fast

## 本轮实际结果
- 本轮完成的是多模态 session memory 的最小闭环，不是完整附件中心。
- 当前已落地能力：
- `SessionAttachment` 元数据持久化：`id / name / mime_type / relative_path / size_bytes / created_at_ms`
- 最近图片召回：`load_recent_images()` 仅从最近一轮 user turn 的附件恢复图片 payload，不再跨多轮模糊重发旧图
- provider 上下文提示：`context.rs` 会根据最近附件与图片引用语义提醒模型“可能会重新附带最近图片”
- 前端显示消息：`displayMessage` 用于把“文本 + 已附图片 N 张”作为用户消息落到 history
- 历史附件 chip：`HomeWorkspace.vue` 会展示历史用户消息里的附件名称
- history 回放安全性：前端不会再把 `relativePath=null` 的占位附件混入下一轮 `history`
- 附件失败语义：同步与流式路径都不会再出现“正文成功但附件静默丢失”的分叉
- 当前未落地能力：
- 独立附件中心
- 跨会话附件检索
- 更完整的附件生命周期管理，例如 TTL、批量清理、手动管理台
- 附件资产级摘要、标签、搜索索引

## Review 收口
- 已修复 review 发现的 3 个高优先级问题：
- 前端占位附件可能混入 replay history，导致下一轮把不完整附件重新发给后端
- recent-image recall 过宽，历史任意旧图都可能因泛化指代被静默重发
- 同步 / 流式两条路径对附件持久化失败的处理语义不一致

## 验证结果
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`
- `cargo test --manifest-path src-tauri/Cargo.toml --test session_regression --test provider_registry_regression --test tool_router_regression`
- `npx vitest run tests/runtime-store.spec.ts tests/HomeWorkspace.spec.ts`

## 下一步动作
- 如果继续推进，下一张任务应聚焦“附件中心”，而不是继续扩大本卡范围
- 为跨会话附件检索与 TTL 策略单开任务
- 决定是否需要独立附件摘要、标签或索引层
- 决定是否要把 recent-image recall 从“最近一条有附件的用户消息”扩展为更细的选择逻辑

## 当前卡点
- 当前最小平衡已经成立；剩余卡点是后续若做附件中心，要避免把 `sessions.json + attachments/` 直接演变成无边界二进制仓库

## 断点续跑提示
继续前先看：
- `src/stores/runtime.ts`
- `src/components/HomeWorkspace.vue`
- `src/types/runtime.ts`
- `src-tauri/src/agent/context.rs`
- `src-tauri/src/agent/session.rs`
- `src-tauri/src/agent/runtime.rs`
