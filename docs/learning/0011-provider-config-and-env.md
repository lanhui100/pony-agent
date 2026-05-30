# 0011 Provider 配置、SecretStore 策略与模型编辑

## 这次学习回答了什么

这次连续推进主要解决了四组问题：

1. `provider`、`protocol`、`model` 三者分别是什么意思
2. `API Key` 为什么不能继续当普通配置长期保存
3. 为什么当前阶段要从 `env` 主存储演进到统一 `SecretStore`
4. Provider 配置页为什么需要“已有模型可编辑”和“保存时立即写入应用密钥存储”

## 一组最重要的概念区分

### 1. Provider Type 不是 API Key

`Provider Type` 表示“提供商适配器类型”，例如：

- `openai`
- `openrouter`
- `deepseek`
- `ollama`

它的作用是告诉后端：

- 该走哪一类 provider 适配逻辑
- 默认协议是什么
- 默认 base URL 是什么
- 默认应该读取哪个环境变量

它本质上更像“适配器代号”，不是密钥。

### 2. Protocol 不是 Provider

`protocol` 表示“和模型接口说话的协议格式”。

当前项目里主要是：

- `responses`
- `chat_completions`

同一个 provider 可能支持不同协议，所以：

- `provider` 解决“连谁”
- `protocol` 解决“怎么连”

### 3. Model 才是最终跑的具体模型

例如：

- `gpt-4.1-mini`
- `deepseek-chat`
- `qwen2.5:7b`

也就是说：

- `provider type`：提供商类型
- `protocol`：接口格式
- `model`：具体模型名

## 为什么当前不再以 env 作为主存储

这次做出的关键决策是：

当前阶段把 `SecretStore` 升级为长期默认凭证来源，`env` 降级为兼容 fallback。

原因不是因为我们突然需要“过度设计”，而是因为之前 `env` 主存储已经开始暴露出产品与工程上的真实问题：

- 保存新 key 后不能可靠热更新
- 旧进程环境变量可能遮挡用户刚保存的新值
- 前端体验上像“保存成功”，但后端实际仍可能读旧值
- provider 配置与敏感凭证边界不够稳定

这很重要，因为 Pony Agent 现在的主线不是做一个完整的凭证系统，而是继续打通：

- `run_turn()`
- 真实 provider 对话
- tool call 链路
- runtime 与工作台协作

所以这次的做法不是停下来重造一套大系统，而是用一个很薄的 Rust 抽象把平台差异收进去：

- `SecretStore`
- 系统 keyring backend
- 文件 fallback backend
- `env` 兼容读取

## 当前采用的边界

当前阶段我们明确把普通配置和敏感配置分开了。

### 会长期保存的内容

- provider type
- model
- protocol
- base URL
- 当前选中的 provider / model

### 不再长期保存的内容

- `API Key`

也就是说：

- `providers.json` 不再保存 `API Key`
- 前端 `localStorage` 也不会再把 `API Key` 当作长期配置保存
- 新保存的 `API Key` 进入应用密钥存储，而不是再写环境变量

## 现在运行时的凭证优先级

当前代码采用的是：

```text
runtime input > SecretStore > env fallback
```

它的含义是：

1. 如果当前保存请求里带了新的 `API Key`，这一轮运行优先用它
2. 如果当前 provider 在 `SecretStore` 中已有密钥，运行时直接读它
3. 如果系统密钥后端为空或旧配置尚未迁移，最后才回退到环境变量

这是一种非常适合当前阶段的结构，因为它同时兼顾了：

- 易用性
- 可测试性
- 未来可演进性

## 为什么现在改成“保存时写入应用密钥存储”

如果继续沿用“保存时自动写入环境变量”，就会出现几个现实问题：

- 热更新不可靠
- 不同 provider 的 key 可能被旧 env 遮挡
- Windows 上用户环境变量和当前进程变量并不总是同步
- Linux server 上也不应该要求用户总是自己维护 shell env

所以这次补的是：

- 用户可以在 Provider 配置页输入 `API Key`
- 保存 provider 时优先写入应用密钥存储
- 输入框默认使用保密显示，避免明文长期暴露在界面上
- 前端成功提示与后端解析路径终于一致

## SecretStore 的设计边界

这次不是把“保存配置”和“保存凭证”混成一件事，而是故意分开。

### 普通保存

普通的“保存 Provider / 保存模型”只负责：

- 名称
- model
- protocol
- base URL

### 写入 SecretStore

保存 provider 时只负责：

- 按 provider id 生成稳定 `secret_ref`
- 把新值写入 `SecretStore`
- 删除已移除 provider 的旧 secret
- `providers.json` 只保存 `secret_ref`，不保存真实 key

也就是说，凭证不再伪装成普通配置。

这是一个非常重要的边界设计。

## 为什么已有模型要支持编辑

如果 Provider 页面只能“新增模型”，会很快出现三个问题：

1. 一旦 model 名写错，只能删掉重建
2. base URL 或 protocol 有调整时，不方便迭代
3. 学习阶段经常需要反复改同一条模型配置

所以这次补了“已有模型可编辑”。

当前设计是：

- 已有模型列表直接展示当前 provider 下的模型
- 列表项支持“编辑”和“删除”
- 点击后，下方表单切换成“编辑模型”
- 保存后更新原模型，而不是新增重复条目

这个交互对学习阶段很合适，因为它：

- 轻量
- 直观
- 不需要额外弹窗

## 这次实现背后的设计原则

这轮工作其实很体现 Pony Agent 的整体方法论：

### 1. 先让主链路可用

先确保：

- provider 可以配置
- 模型可以切换
- `run_turn()` 可以调用真实模型

### 2. 再逐步补安全和体验

在此基础上再补：

- `API Key` 不入长期配置
- `SecretStore` 方案
- UI 写入应用密钥存储

### 3. 不让一个子系统阻塞整个 agent core

密钥系统很重要，但它不能反过来卡住：

- provider 抽象
- runtime 演进
- tool calling

## 当前学习进度

截至这次记录，学习和实现已经推进到：

1. 已理解 `run_turn()` 是 agent core 的最小回合入口
2. 已理解它和 Claude / queryLoop 在方向上是一致的，只是复杂度不同
3. 已有最小 provider 抽象，并支持多 provider / 多协议 / 多模型
4. 已有 Provider 配置页、模型切换、模型编辑
5. 已把 `API Key` 从长期普通配置里剥离
6. 已支持在保存 provider 时把 `API Key` 写入应用密钥存储
7. 已把工作台主链路从单次阻塞返回推进到事件驱动的最小流式回包
8. OpenAI 兼容协议与 Anthropic 协议都已接入真实 stream 骨架
9. 已具备 `turn:started / turn:delta / turn:trace / turn:tool / turn:completed / turn:failed` 最小事件模型

## 平台后端怎么落地

当前 `SecretStore` 的后端优先级是：

- Windows：Credential Manager
- macOS：Keychain
- Linux 桌面：Secret Service / libsecret
- Linux server / headless：若系统密钥服务不可用，则降级到本地 `secrets.json`

这意味着：

- Linux 服务器版本也被覆盖到了
- 文件 fallback 是兼容方案，不是新的主安全建议
- `env` 只保留读取兼容，不再承担保存入口

## 下一步最自然的学习方向

下一步不应该继续深挖密钥系统，而应该回到 agent core 主线、事件契约和运行时指标层。

最自然的后续方向是：

1. 让主页更清楚地展示“当前这轮到底是走了真实模型，还是回退到 mock”
2. 补齐 `providerMode / fallbackReason / token 统计 / 首 token 延迟`
3. 继续增强 `run_turn()` / `start_turn_stream()` 的真实回合信息，比如工具链路和更细的状态展示
4. 逐步推进真实工具协议、更清晰的 runtime 状态，以及未来可脱离 Tauri 的接入层

## 一句总结

这次最重要的收获不是“学会了怎么写 env”，而是理解了一个工程化判断：

在学习式重构里，安全方案要有方向，但不能成为主线阻塞项；先把边界划对，再逐步加固，往往比一开始就追求终局方案更正确。
