# ADR 0040：TUI `/models` 远程模型发现与回环地址代理旁路

Status: implemented

## 背景

TUI 的 `/models` 之前只读 `providers.json` 里已配置的 `models` 数组；而 `ProviderRegistryStore::load_view()` 在数组为空时会注入一个 `{provider.id}-model-default` 占位模型（`config.rs:873-889`），因此用户在新配置 provider 后永远只能看到一个占位模型，无法获知该 provider 真实可用的模型清单。

排查过程中还发现一个环境级缺陷：进程继承了全局代理（`HTTP(S)_PROXY` / `ALL_PROXY`），对 `127.0.0.1` 的模型目录请求被代理劫持并返回 `401 unauthorized`，本地假服务收不到请求；这同样会导致本地 provider（Ollama、vLLM 等）在真实用户环境中不可用。

## 候选方案

### 方案 A：保持只读本地配置

要求用户手工把模型 id 写进 `providers.json`。

否决：违背「开箱可用」，且占位默认模型会让用户误以为 provider 只有 1 个模型。

### 方案 B（采纳）：cache-first + 无条件后台远程发现

- `/models` 一律先用缓存条目立即打开面板（保持零等待），同时**无条件**在后台线程调用既有 `provider::fetch_model_ids`（15s 超时，鉴权头按协议族推导）。
- 成功：用返回的 id 覆盖该 provider 的 `models`（清掉占位默认模型）、`save_view` 持久化、面板若仍打开则刷新并保留命中选中项；`status_text` 置为「已加载 N 个模型」。
- 失败：会话流插入 System 错误消息（含 provider id 与摘要），面板保留缓存列表，不崩溃不退出。
- 面板内 `r`/`R`：对当前 provider 无条件重拉；Normal 模式 `Ctrl+R` 打开面板并刷新（**不占用裸 `r`**，避免吞掉以 r 开头的输入）。
- provider 解析统一走 `resolve_active_provider_id`（注册表 `selected_provider_id` → 首个 provider → 状态值命中校验），启动时把 `current_provider_id`/`model` 与注册表同步。
- **解析顺序改为注册表优先**：`provider_id` 命中注册表即用 `registry.resolve_selection`（含 id 为 `ponyllm` 的情形，取注册表里的 base_url 与 `api_key_value`）；仅当注册表完全没有该 provider 时才回落到「gateway_url + 环境变量 key」兜底——修复「选完模型发消息 401 unauthorized」。
- 鼠标：**默认捕获**（滚轮可用）；`F2` 静默切换释放捕获，以便用终端原生拖选/复制文本（不写状态栏、不推消息），`--help` 保留键位说明。

### 方案 C（采纳，伴随）：回环地址强制直连

新增纯函数 `should_bypass_proxy(base_url)`（`127.0.0.1`/`::1`/`localhost`/`[::1]`/`::ffff:127.0.0.1`，大小写不敏感）与 `client_for_endpoint`；命中回环的 client 使用 `no_proxy()` 强制直连，外部域名保持环境代理行为不变。已接线到模型目录与对话/流式共 5 个出网点。

否决「全局禁用代理」：会破坏需要代理访问外部模型的用户环境。

## 权衡

- `/models` 每次都会触发一次网络请求（最长 15s，但 UI 不阻塞）；换来的是「永远看到 provider 真实模型清单」。
- 覆盖缓存会清掉占位默认模型；若用户手工定制过 `models`（含自定义别名/能力字段），实现按同 id 克隆保留原有条目语义。
- 回环直连会让「本机代理转发本地服务」这一非常规用法失效；判定为可接受（本地服务本就不应经代理）。

## 后果

- 新增验收套件 `tests/acceptance/l2at-tui-models.sh`（12 用例，冻结）：假 OpenAI 兼容服务 + 隔离 HOME/XDG、拉取/持久化/失败/`r` 刷新/`Ctrl+R`/裸 `r` 不触发面板/坏代理下回环直连。
- `tui_acceptance.rs` 增加 provider 解析与代理旁路纯函数单测。
- `status_text` 新增渲染位（composer 色带底行右侧，优先于运行时状态词），使拉取进度与结果可见。
