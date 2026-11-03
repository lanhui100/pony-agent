# TUI Provider & Initial Splash Refactor Contract

## 1. 目标与背景
用户使用 tmux 测试 TUI 时发现三大核心体验问题：
1. **提示 provider 的 key 错误**：当前切换 provider 或新增 provider 后，由于 key 缺失或未持久化至 secret_store，导致发送请求时报 `Error: provider xxx missing API key` 或鉴权错误，提示不友好，且新增流程未能写入 key 到 secret_store。
2. **初始化页面的 P 元素块画面不美观**：当前初始页面的 P 元素网格为 12x16 粗颗粒半块字符（`SHAPE` 12x16），字形边缘锯齿严重，配色生硬，缺乏质感与高级现代终端排版。
3. **Provider 操作缺失与二级联动**：当前 `/provider` 列表只有选择和 `a` 新增，无法删除无用或测试 provider；且回车选中 provider 后直接退出弹窗回到 Normal 模式，用户需要再次输入 `/models` 才能选模型，体验割裂。要求回车选中 provider 后无缝衔接打开该 provider 的二级 model 选择菜单。

---

## 2. 详细设计与交互契约

### 2.1 Provider Key 错误诊断与持久化契约
- **新增 Provider 时的 Secret 存储**：
  在 `finalize_add_provider` / 提交新增 provider 时，如果用户填写了 `api_key`，除了在 `view.providers` 中保留之外，必须同步写入 `secret_store`（key 为 `format!("provider/{}/api-key", provider_id)`）。
- **Key 缺失诊断与错误提示**：
  - 在选择 provider 时，若该 provider 既无 `api_key`（secret_store / api_key_value）且对应环境变量亦为空，在列表右侧或切换提示时清晰标注 `(未配置密钥)`。
  - 在用户发送消息时，若当前 provider 缺少 API key，拦截并给出明确友好的错误提示，指导用户：“当前提供商 [xxx] 未配置 API Key，请在 /provider 中重新配置或设置环境变量 [ENV_VAR]”，避免底层生硬报错。

### 2.2 P 元素块全新美观设计规范
- **设计语言**：科技感、极简流光、微渐变、结构对称的 ASCII/Unicode 现代几何块设计。
- **字形与网格**：
  采用更优雅精细的字形比例（如 16x16 或精雕细刻的流线型双线/实心平滑圆角块）：
  采用更高辨识度、几何倒角平滑的现代科技感“P”，避免粗糙的方块拼凑。
- **色彩停靠与着色**：
  采用符合深色终端美学的柔和流光渐变（如科技青/冰蓝/琥珀/紫罗兰），避免刺眼的荧光混杂。
  右侧信息栏（版本、当前模型、工作区）与品牌 Logo 采用卡片式垂直对齐排版，呼吸感充足。

### 2.3 Provider 增删与二级 Model 联动契约
- **删除操作 (`d` / `Delete`)**：
  - 在 `InteractionMode::SelectingProvider` 下，按 `d` 或 `Delete`：
  - 若选中的是内置/系统核心 provider（如 `ponyllm`），拦截并提示：“内置提供商不可删除”；
  - 若为自定义添加的 provider，将其从注册表 `view.providers` 中移除，同时清理 secret_store 中的 key；
  - 若当前选中的正是被删除的 provider，回退当前 provider 为 `ponyllm` 或列表中第一个有效 provider；
  - 更新注册表持久化并刷新 `provider_list`。
- **二级 Model 联动 (`Enter`)**：
  - 在 `InteractionMode::SelectingProvider` 下，用户按 `Enter` 选中某个 provider：
  - 立即将该 provider 设为当前激活提供商，并持久化保存；
  - **状态机流转**：不直接回到 `Normal`，而是**无缝打开该 provider 的模型选择菜单**（`InteractionMode::SelectingModel`），将列表填充为该 provider 的 `models`；
  - 同时在后台异步触发 `spawn_model_discovery` 拉取该 provider 最新的远程可用模型列表；
  - 在 `InteractionMode::SelectingModel` 中，按 `Esc` 则回到 `Normal`（或保存选中的模型后回 Normal）。
