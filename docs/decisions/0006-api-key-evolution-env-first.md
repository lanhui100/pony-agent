# 0006 API Key 存储从 env-first 演进到统一 SecretStore

## 背景

Pony Agent 当前处于学习式重构阶段，主线目标是持续打通这些核心能力：

- `run_turn()` 的真实执行链路
- 多 provider / 多模型切换
- tool routing 与 tool execution
- Rust runtime 与桌面工作台之间的协作

与此同时，项目已经开始进入真实 provider 接入阶段，因此 `API Key` 的来源与存储方式必须尽快有一个明确结论。

## 约束

- `API Key` 是敏感信息，不适合继续作为普通前端状态长期保存
- 当前阶段不能让密钥系统设计阻塞 agent core 的主线推进
- Pony Agent 未来需要考虑多端部署，因此最终方案不能只绑定桌面端本地存储
- 当前团队仍处于学习和快速验证阶段，优先级是“先跑通主链路，再逐步加固”

## 决策

截至 2026-05-24，`API Key` 的主存储不再采用环境变量，而是统一走 Rust 侧 `SecretStore` 抽象。

具体策略是：

- 非敏感配置继续保存在 Rust 管理的配置文件中
- `API Key` 不作为长期明文配置写入普通 JSON
- Provider 配置页保存密钥时，优先写入平台原生密钥存储
- 若系统密钥后端不可用，则降级写入本地 `secrets.json`
- Rust provider 解析时，优先使用运行时传入值，其次读取 `SecretStore`，最后兼容读取环境变量
- `env` 从主存储降级为兼容 fallback，而不是继续承担产品主路径

当前推荐优先级为：

`runtime input > SecretStore > env fallback`

## 为什么这么做

这样做的核心原因不是“再做一套凭证系统”，而是把“安全边界”和“平台适配”收敛到 Rust 内部，同时不让前端、provider 和 runtime 到处直接碰密钥。

它的价值在于：

- 让 provider 保存后可以立即生效，不再依赖环境变量热更新
- 把普通配置与敏感凭证彻底拆开，避免再次把密钥写回 `providers.json`
- 把平台差异封装在 `SecretStore` 后端，而不是泄漏到前端或 runtime 主流程
- 为未来 HTTP / SSE / CLI / 桌面等多宿主复用同一套 provider 配置边界

## 权衡

### 好处

- 保存即生效，解决“新 key 已保存但旧 env 仍在生效”的热更新问题
- 平台优先走系统密钥存储，桌面侧安全性和可用性更合理
- Linux server / headless 场景仍有文件后端可用，不会因为缺少桌面密钥服务直接失效
- Rust 抽象边界清晰，便于后续替换或扩展后端

### 代价

- 需要维护多后端：系统 keyring + 文件 fallback
- Linux 的文件 fallback 只是兼容方案，安全性不等同于系统密钥存储
- 如果未来扩展到 Web 或托管部署，仍需要新的服务端 secret backend

## 平台后端矩阵

- Windows：Credential Manager
- macOS：Keychain
- Linux 桌面：Secret Service / libsecret
- Linux server / headless：若无 Secret Service，则降级到本地 `secrets.json`

说明：

- Linux server 包含无桌面环境、无 DBus secret service 的常见服务器部署
- 文件 fallback 仅为可运行兼容，不是新的推荐主路径

## 后续影响

这项决策意味着 Pony Agent 接下来需要坚持以下边界：

- agent core 不直接承担密钥存储职责，而是依赖 `SecretStore` 接口
- 普通配置与敏感凭证继续分层
- 未来多端部署时，以统一凭证接口适配不同平台实现，而不是强求单一底层存储方式
- `providers.json` 继续禁止落盘真实 API Key

## 后续演进方向

后续可按以下顺序演进：

1. 当前阶段：统一 `SecretStore` 已落地，桌面端优先走系统密钥存储
2. 兼容阶段：继续保留 `env fallback`，平滑承接旧配置与手工部署
3. 多端阶段：为 HTTP / SSE / CLI / 桌面宿主复用同一套 `SecretStore` 边界
4. 服务端 / Web 阶段：接入服务端 secret manager 或远端凭证代理
