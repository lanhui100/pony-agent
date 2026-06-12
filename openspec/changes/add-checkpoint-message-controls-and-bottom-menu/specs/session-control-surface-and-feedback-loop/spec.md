## ADDED Requirements

### Requirement: Checkpoint rollback affordances SHALL be embedded in historical agent messages
Pony Agent 前端 SHALL 在主对话区中把 checkpoint 回退入口嵌入到非最新 agent 消息底部，而不是把该能力只放在侧边栏 history graph 中。

#### Scenario: A historical agent message is checkpoint-backed
- **WHEN** 某条非最新 agent 消息具备稳定的 checkpoint 映射
- **THEN** 该消息底部 SHALL 展示两个纯图标回退入口
- **AND** 两个入口 SHALL 分别表示 `仅回退对话历史` 与 `回退对话历史并尝试恢复文件改动`

#### Scenario: The latest agent message is rendered
- **WHEN** 当前消息是最新一条 agent 消息
- **THEN** 前端 SHALL NOT 为该消息展示 checkpoint 回退图标

#### Scenario: A message lacks stable checkpoint identity
- **WHEN** 前端无法为某条 agent 消息获得稳定的 checkpoint 标识
- **THEN** 前端 SHALL 隐藏该消息的 checkpoint affordance
- **AND** SHALL NOT 通过消息顺序、数组下标或视觉位置猜测 checkpoint 归属

### Requirement: Stable turn-to-checkpoint mapping SHALL be provided before message-level controls render
前端 SHALL 通过稳定的显式映射决定某条 agent 消息是否拥有 checkpoint affordance，而不是依赖隐式顺序关系。

#### Scenario: Runtime state provides explicit mapping
- **WHEN** runtime state、snapshot、runtime view 或等价 read-plane 为某个 `turnId` 提供稳定的 checkpoint 标识
- **THEN** 前端 SHALL 基于该显式映射决定是否展示消息级 checkpoint 控制

#### Scenario: Mapping is absent for a turn
- **WHEN** 某个 `turnId` 没有对应的稳定 checkpoint 映射
- **THEN** 前端 SHALL 隐藏该消息的 checkpoint affordance
- **AND** SHALL NOT 通过 assistant 消息序号、history node 顺序或视觉位置推测映射关系

### Requirement: Checkpoint read model SHALL be derived from existing runtime state
本 change 的 checkpoint read model SHALL 基于既有 runtime state 扩展和 store 归一化生成，而不是新增独立的 checkpoint command family。

#### Scenario: Existing runtime state is sufficient
- **WHEN** runtime state 已提供 `HistoryNode`、`HistoryBranch` 与稳定的 turn-to-checkpoint mapping
- **THEN** 前端 store SHALL 在本地归一化出消息级 checkpoint read model
- **AND** SHALL NOT 要求新增独立 checkpoint endpoint 或私有命令路径

#### Scenario: Existing runtime state lacks required fields
- **WHEN** 当前 runtime state 缺少 `turnId`、workspace rollback 能力位或其他消息级 affordance 必需字段
- **THEN** 本 change SHALL 先补齐 runtime state schema 或等价 read-plane 合同
- **AND** UI 实现 SHALL NOT 在 schema 未补齐时通过私有推理绕过该缺口

### Requirement: Message-level checkpoint actions SHALL reuse existing history-control boundaries
消息级 checkpoint 图标 SHALL 继续派发到既有 history-control 边界，而不是新增新的回退命令族。

#### Scenario: The user requests transcript-only rollback from a message
- **WHEN** 用户点击某条消息下方的“仅对话”图标
- **THEN** 前端 SHALL 派发到既有 `checkoutHistoryNode(nodeId, "transcript_only")`

#### Scenario: The user requests transcript-and-workspace rollback from a message
- **WHEN** 用户点击某条消息下方的“对话 + 文件”图标
- **THEN** 前端 SHALL 派发到既有 `checkoutHistoryNode(nodeId, "transcript_and_workspace")`
- **AND** SHALL NOT 新增独立 rollback backend command

### Requirement: Checkpoint affordances SHALL explain workspace rollback capability
消息级 checkpoint affordance SHALL 通过 tooltip 或等价提示明确解释恢复模式与降级语义。

#### Scenario: Workspace rollback is supported
- **WHEN** 某个 checkpoint 支持 `transcript_and_workspace`
- **THEN** 相关图标 hover 时 SHALL 显示“回到此 checkpoint（对话 + 文件）”或等价提示

#### Scenario: Workspace rollback is unsupported
- **WHEN** 某个 checkpoint 不支持文件回退
- **THEN** 相关图标 hover 时 SHALL 明确提示“将仅恢复对话”或等价降级语义
- **AND** 前端 SHALL NOT 把该入口伪装成完整 workspace rollback 成功

### Requirement: Forked checkpoint conversations SHALL be discoverable from the message surface
当某个 checkpoint 已经产生新的对话轨迹时，Pony Agent 前端 SHALL 让用户在对应消息处直接发现这些 fork。

#### Scenario: A checkpoint has one or more forks
- **WHEN** 某个 checkpoint 对应的历史节点已存在 fork 分支
- **THEN** 对应消息底部 SHALL 额外展示一个 fork 摘要纯图标
- **AND** 点击后 SHALL 打开一个展示各 fork 摘要的菜单、弹层或对话框

#### Scenario: The user inspects fork summaries
- **WHEN** fork 摘要菜单或弹层被打开
- **THEN** 每个 fork 项 SHALL 至少展示分支标签或等价名称，以及对应 checkpoint 的摘要截取
- **AND** 每个 fork 项 SHALL 提供“转到该 checkpoint”的菜单动作

#### Scenario: The user jumps via a fork item
- **WHEN** 用户点击某个 fork 项的“转到该 checkpoint”
- **THEN** 前端 SHALL 关闭 fork 摘要菜单、弹层或对话框
- **AND** SHALL 派发到既有 `switchHistoryBranch`、`checkoutHistoryNode` 或等价既有 history-control action

#### Scenario: A checkpoint has no forks
- **WHEN** 某个 checkpoint 尚未产生 fork 对话轨迹
- **THEN** 前端 SHALL NOT 展示 fork 摘要图标

### Requirement: Composer footer SHALL provide the primary checkpoint picker
Pony Agent 前端 SHALL 把 checkpoint 主选择入口收口到主对话区底部 bar，而不是继续依赖当前笨重的 session control 主设计。

#### Scenario: The user wants to choose a checkpoint from the composer area
- **WHEN** 用户在主对话区底部 bar 打开 checkpoint picker
- **THEN** 菜单 SHALL 展示各 checkpoint 的摘要截取
- **AND** 点击某项后 SHALL 回退到对应 checkpoint

#### Scenario: Existing session-control surfaces remain available
- **WHEN** sidebar 仍保留 explainability 或次级历史管理信息
- **THEN** 这些 surface SHALL 不再承担 checkpoint 主选择入口职责
- **AND** SHALL NOT 与底部 checkpoint picker 并列提供双主入口 CTA

#### Scenario: Sidebar keeps checkpoint-related information
- **WHEN** sidebar 继续展示 history graph、audit summary 或 branch 管理动作
- **THEN** 这些内容 SHALL 被视为 explainability 或次级管理 surface
- **AND** SHALL NOT 冒充为 workspace 中的首要 checkpoint 导航层

### Requirement: A keyboard shortcut SHALL open the checkpoint picker
Pony Agent 前端 SHALL 提供一个可测试的快捷键以打开底部 checkpoint picker。

#### Scenario: The shortcut is pressed
- **WHEN** 用户触发已定义的 checkpoint picker 快捷键
- **THEN** 前端 SHALL 打开 checkpoint picker
- **AND** 该行为 SHALL 可通过前端测试稳定验证

#### Scenario: The shortcut would conflict with core text input editing
- **WHEN** 候选快捷键与基础文本输入或现有全局快捷键冲突
- **THEN** 实现 SHALL 选择一个不冲突的替代组合
- **AND** 最终组合 SHALL 在实现与测试中被显式记录
