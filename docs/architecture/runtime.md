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

## 最佳实践

- 先抽象 trait，再落地 provider/tool 实现
- 返回结构化结果，不要到处传裸字符串
- 明确同步和异步边界
- 为调试留出事件或状态输出接口
