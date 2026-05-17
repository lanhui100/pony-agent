# TASK-001 主页显示 provider 真实来源与 mock fallback

- 状态：`In Progress`
- 优先级：`P1`
- 负责人：`Codex / 用户协作`

## 目标

让主页和右侧状态区更清楚地告诉用户：当前这一轮到底走了真实 provider，还是因为缺少凭证而回退到了 mock。

## 预期输出

- 主页出现清晰的 provider 来源提示
- 能区分：
  - 当前会话输入的 key
  - 环境变量
  - mock fallback

## 验收标准

- 用户无需看源码，也能判断当前回合是否命中了真实模型
- UI 提示不打断现有对话主区布局

## 当前进展

- provider 配置页已经具备模型选择、模型编辑、env 写入
- 目前主页只有 provider / protocol / model 文本，还没有明确来源解释

## 下一步动作

在 Rust `run_turn()` 响应或 health/status 数据里增加 provider source 信息，再同步到主页状态区。

## 断点续作提示

优先检查：

- [runtime.rs](C:/Users/HUAWEI/Documents/New%20project/src-tauri/src/agent/runtime.rs)
- [provider.rs](C:/Users/HUAWEI/Documents/New%20project/src-tauri/src/agent/provider.rs)
- [main.ts](C:/Users/HUAWEI/Documents/New%20project/src/main.ts)
