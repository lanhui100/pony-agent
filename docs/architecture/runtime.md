# Rust 运行时设计

## 第一阶段目标

先实现最小运行时闭环：

1. 接收输入
2. 调用模型
3. 返回结果
4. 在 UI 中展示状态变化

## 推荐模块

### `runtime`

负责：

- 启动一次 agent turn
- 协调 graph、provider、tool、session

### `graph`

负责：

- 描述执行状态和状态转移

第一版可以很简单：

- `Idle`
- `CallingModel`
- `CallingTool`
- `Done`
- `Failed`

### `provider`

负责：

- 对接模型
- 提供统一的 `generate` 能力

### `tools`

负责：

- 注册工具
- 执行工具
- 返回统一结果

### `session`

负责：

- 保存当前消息
- 管理会话元数据
- 维护 `sessionId -> session state` 的映射
- 为 runtime 提供 `snapshot / append_turn` 这种稳定边界
- 后续可从“内存 + JSON 文件”平滑升级到 SQLite / PostgreSQL

当前最小实现已经不是单个固定 `SessionState`，而是一个 `SessionStore`：

- 内存态负责当前进程内的快速读写
- 本地 `.pony-agent/sessions.json` 负责最小持久化
- `SessionBackend` 已预留成可替换后端接口，后续可接 SQLite / PostgreSQL
- `run_turn()` 与 `start_turn_stream()` 都只通过 session snapshot 读取上下文
- turn 完成后再统一 append 回 session store，而不是让前端长期承担真实会话态

## 最佳实践

- 先抽象 trait，再落地 provider/tool 实现
- 返回结构化结果，不要到处传裸字符串
- 明确同步和异步边界
- 为调试留出事件或状态输出接口
