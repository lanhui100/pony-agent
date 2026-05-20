# 为什么要先做 SessionStore，再做新对话和历史对话

## 问题

当前 Pony Agent 对 `session` 到底实现了什么，为什么先做 Rust core 的 `SessionStore`，而不是先做前端“新对话 / 历史对话”界面？

## 简短结论

我们这轮完成的重点，不是“会话 UI”，而是“会话 ownership” 的迁移：

- 真正的会话状态已经从前端临时 history，收敛到 Rust core 的 `SessionStore`
- 当前已经具备最小多会话与最小持久化能力
- 这一步的意义，是先把 agent core 的边界立住，再往上长出“新对话 / 历史对话”交互

所以现在最自然的下一步，确实就是在已有 `SessionStore` 之上补前端的会话管理能力。

## 系统化梳理

### 1. 我们这次对 session 真正做了什么

这轮实现的核心，不是“把聊天记录显示出来”，而是把“谁拥有真实会话状态”这个问题定下来。

当前已经落地的能力包括：

- Rust core 不再只维护单个临时会话，而是开始维护 `sessionId -> session state`
- runtime 在每轮开始前先读取 session snapshot，而不是长期依赖前端传来的临时 history
- 一轮结束后，再把用户消息和 assistant 消息统一回写到 session store
- 最小持久化已经落到本地 `.pony-agent/sessions.json`
- session backend 已预留成可替换接口，后续可以接 SQLite / PostgreSQL

这说明我们已经有了“真正的会话层”，而不只是“前端消息数组”。

### 2. 为什么要先做这一步

因为“新对话 / 历史对话”如果先做在前端，而后端没有真正的 session ownership，最后只会得到一个看起来像会话系统、实际上还是 UI 本地状态的壳。

先做 `SessionStore` 的价值在于：

- 先明确 session 属于 agent core，而不是属于 Tauri 页面
- 先把 runtime、tool、provider、多轮上下文都接到同一条会话主线上
- 先让未来的 HTTP/SSE adapter、TUI、Web UI 都可以复用同一套会话语义

也就是说，这一步是在做地基，而不是做表层交互。

### 3. session 和 history/context window 不是一回事

当前需要明确区分三层：

- `session`：长期会话状态，由 Rust core 持有
- `history/context window`：某一轮真正送给模型的上下文切片
- `session summary`：当前会话的轻量概览信号

这三者相关，但不能混成一团。

如果不先把 session 层独立出来，后面做上下文裁剪、summary compression、prompt caching 和持久化时会越来越乱。

### 4. 当前代码组织说明了什么设计边界

当前的边界已经比较清楚：

- `session.rs` 负责定义会话状态、store 和 backend
- `runtime.rs` 负责消费 session，而不是拥有 session
- `lib.rs` 负责把共享 runtime 暴露给 Tauri command，避免每轮重建

这说明我们已经把“会话是什么”“会话怎么被使用”“会话怎么被交付出去”分成了三层。

### 5. 为什么现在该进入“新对话 / 历史对话”

因为 session 的后端基础已经够用了。

当前缺的已经不是“有没有 session”，而是：

- 用户能不能显式新建一个新的 `sessionId`
- 用户能不能看到已有会话列表
- 用户能不能切换到旧会话继续工作
- 前端能不能把“当前会话”真正映射成一个可管理的对象，而不是隐式固定成 `workbench-main`

所以接下来的新对话 / 历史对话，不是另起炉灶，而是在现有 session 基础上往上补产品层交互。

## 常见误区

- 误区 1：当前已经有持久化文件，所以“历史对话功能”已经算完成了
- 误区 2：前端能把最近几轮消息发给后端，就等于已经有真正的 session 系统
- 误区 3：`session summary` 已经等于长期 memory 或上下文压缩系统

## 后续值得继续学什么

- 新对话 / 历史对话的产品交互，应该如何映射到 `sessionId`
- session store 和未来 SQLite / PostgreSQL backend 的关系
- session、summary、memory、context builder 四层未来如何分工

## 可延展内容选题

- 为什么会话 ownership 必须先回到 agent core，前端会话 UI 才值得做
- SessionStore、context window 和 memory，到底是不是同一个东西
