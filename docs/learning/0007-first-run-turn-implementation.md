# 0007 Pony Agent 第一版 run_turn() 实现了什么

## 问题

Pony Agent 第一版 `run_turn()` 到底实现了什么？为什么第一版不直接接真实模型？

## 简短结论

第一版 `run_turn()` 先实现的是“工程闭环”，不是“能力闭环”。

它已经完成了：

- 前端可以发起一次 turn
- Rust 可以接收输入
- Rust 可以返回结构化结果
- 前端可以展示消息、phase、trace 和 tool 活动

但它还没有接真实模型和真实工具。

## 系统化梳理

### 1. 第一版做了什么

当前第一版 `run_turn()` 已经具备这些职责：

- 接收输入消息
- 生成本轮的 assistant 回包
- 返回本轮 phase
- 返回 trace steps
- 返回 tool activities
- 返回 session summary

它说明 Pony Agent 已经有了“本轮执行入口”。

### 2. 为什么不直接接模型

因为当前阶段最重要的是先验证这一条主链：

- UI 输入
- Tauri command
- Rust runtime
- 结构化结果
- UI 回显

如果一开始就同时引入：

- 真实模型接口
- API key
- provider 抽象
- 错误处理
- 工具调用

那问题会混在一起，不利于学习和调试。

### 3. 这一步的真正价值

这一步的价值不是“变聪明了”，而是：

- 一轮执行的边界确定了
- 输入输出结构确定了
- 前后端交互路径打通了
- 后续真实模型、工具、记忆都能顺着这个入口接进来

### 4. 可以怎么理解这一步

可以把它理解成：

- 先把插座装好
- 以后再接不同的电器

这里的插座就是 `run_turn()`。

## 相关文件

- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/lib.rs`
- `src/stores/runtime.ts`
- `src/components/ChatPanel.vue`

## 常见误区

- 误区 1：没有接真实模型，就不算实现了 `run_turn()`
- 误区 2：应该先做能力最强，再补工程闭环

## 后续值得继续学什么

- `TurnInput` 和 `TurnResult` 如何进一步抽象
- 下一步如何接入 provider trait
- `run_turn()` 里怎么逐步长出 while loop

## 可延展内容选题

- 公众号：`为什么 AI Agent 的第一版 run_turn() 不应该一上来就接大模型`
- 知乎：`做 AI Agent 时，先打通执行闭环还是先接真实模型？`
