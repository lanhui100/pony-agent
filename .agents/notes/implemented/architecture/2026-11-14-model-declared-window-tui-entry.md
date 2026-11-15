# Agent Note: 上下文窗口有了 TUI / CLI 入口（逐模型声明，定点写入不改他人）

Status: implemented

## Problem

`providers[].models[].capabilities.contextWindowTokens` 是**逐模型**的个体事实，桌面端之外
却没有任何入口：

- TUI `/provider` 只有「名称 / Base URL / API Key」三步，`/settings` 只有三项显示偏好，
  `/models` 只负责切换；`/models` 面板甚至连窗口都不显示。
- CLI 的 `SubCommand` 只有 `Run` / `Init`，维护命名空间只有 `sessions` / `config`，
  没有任何 provider/model 配置面。

后果实测（本机 6 个 provider、42 个模型）：全部模型都停在预设补缺值 `262144`，而模型真实容量
是 200K / 1M 不等的。用户看到 TUI 右上角的上下文百分比失真（200K 模型显示 62% 而非 81%），
唯一解法是手改 `providers.json`——而这条解法本身不可发现。

上一轮（[2026-11-14-model-declared-context-window-precedence](./2026-11-14-model-declared-context-window-precedence.md)）
已把「声明优先、预设只补缺」修好，并把「TUI 无窗口输入入口」如实登记为未收口项。本卡收口它。

## Decision

1. **真源下沉到 `agent::model_window`**（新模块）。提供 `list_context_windows` /
   `find_context_window` / `set_context_window` / `parse_context_window_input` /
   `format_context_window`，TUI 与 CLI 共用，不各写一套口径。
2. **写入是「定点单键」而不是 `load_view → save_view`**。原因：`load_storage` 归一化会把
   **缺失**的窗口按预设补成显式值，view↔storage 往返会把「未声明」固化成「声明了 262144」，
   此后预设再也补不了缺，用户也分不清哪个数是自己填的。`set_context_window` 因此直接读写
   `providers.json`：只增删目标模型的 `capabilities.contextWindowTokens`，其余键值原样保留，
   同目录临时文件 + `rename` 落盘（半截 JSON 比写失败更难恢复）。
3. **「清除」= 删键**，不是写 `0`、也不是写预设默认值。`0` 会让上下文比例与压缩水位按 0 窗口
   算，比缺失更坏。空输入 / `-` / `clear` / `default` 一律解释为删键。
4. **声明与生效值必须同时可见**：
   - TUI 模型面板徽标：`ctx 200K`（声明）/ `ctx 262.1K·预设`（预设补缺）；
   - 编辑器回显 `200K（声明）` / `262.1K（预设）`；
   - CLI `models list` 输出 `declaredContextWindowTokens` 与 `contextWindowTokens` + `source`。
   只说一个数值，用户永远无法自查「这个数是谁给的」——这是上一轮 62% 事件的可诊断性缺口。
5. **TUI 入口是 `/models` 面板内的 `w`**（不新增斜杠命令）：编辑对象天然是「面板高亮项 +
   当前活动 provider」，不需要再造一个模型选择器。编辑器自带缓冲（不借 Composer），
   避免编辑窗口时吞掉用户草稿；进 `is_modal_mode`，否则排队消息回灌的 Enter 会被当成「保存」。
6. **CLI 入口是 `ponycli models list|set-window`**，与 `sessions` / `config` 同属「常规参数解析
   之前拦截」的命名空间（并同样受 `--help` 让路护栏保护，零副作用）。解析与执行拆成
   `parse_models_command` / `execute_models_command` 两个可测函数。
7. **失败一律留在现场**：解析错误、超上限、未命中模型、落盘失败都在面板/CLI 显式报错并
   **零写入**；写入后读不回来也报错（绝不假装成功）。
8. **十进制口径与桌面端一致**（K = 1000、M = 1_000_000，`200K` → 200000），
   与 `ProviderConfigPage.vue::formatTokenUnit` / `CONTEXT_TOKEN_PRESETS` 对齐；
   上限 10M token 只防手滑（多打一位数），不参与业务判断。

## Alternatives considered

- **A. 复用 `save_view` 整份重写注册表**（最小改动）：会把**所有**模型的未声明窗口固化成
  `262144`，还会顺带重写 `maxOutputTokens` 等归一化字段——「改一个数字却动了 42 个模型」
  的副作用无法接受，也正好把上一轮刚修好的「未声明」语义毁掉。
- **B. 新增斜杠命令 `/model-window`**：`/models` 面板已经选中了目标模型，再让用户用参数
  重复指定 provider/model 是双重输入；且新增命令要配套 `/help`、拼写纠错表、别名，成本高于收益。
- **C. 在 `/provider` 加第 4 步「上下文长度」**：`/provider` 是**新增** provider 的向导
  （新 provider 还没有 models），窗口是既有模型的属性，挂错表单。
- **D. 让 CLI 走 `ponycli config set` 通用键值写入**：`config` 命名空间的语义是「读生效策略」，
  引入通用写入口等于给任意路径写文件开洞；模型窗口是唯一确定的需求，给窄接口更安全。
- **E. 只做 TUI，不做 CLI**：远程/无 TTY 场景（SSH、脚本、CI 里修配置）依然只能手改文件，
  而这正是本次事故的现场形态；两个入口共用同一真源，边际成本很低。
- **F. 纯十进制显示「262144」而不是「262.1K」**：与桌面端不一致会出现「同一个窗口两个数字」，
  这正是本项目历史上窗口口径分叉的老毛病。

## Consequences

- **正向**：窗口改完立即生效（TUI 每轮重读注册表；面板徽标与右上角分母就地刷新），
  改一个模型不影响其它模型，未声明的模型继续跟随预设。
- **代价（显式承担）**：定点写入用 `serde_json` 重新序列化整份文件，**键顺序**可能被重排
  （数据不变，含未知键也被保留）；这是「不整份归一化」换来的代价，已由用例钉住
  「除目标键外整棵树逐字等价」。
- **边界**：不支持跨 provider 批量设置；`0` 不接受（要清除请用 `clear`/空值）——
  避免把「没填」与「填了 0」混同。
- **回归证据**：
  - 核心 `agent::model_window` 10 条：声明/预设并列、定点写入不动兄弟、删键而非写默认值、
    `capabilities` 缺席时只长一个键、未命中零写入、按线上模型名寻址、解析与格式化边界；
  - TUI `model_window_editor_tests` 10 条 + 模块内 2 条：预填/留空语义、提交只改目标模型、
    清除恢复预设、越界输入留在面板且零写入、Esc 不落盘、脏字符过滤、面板徽标来源、
    编辑器渲染文案、`is_modal_mode` 收纳；
  - CLI `cli_acceptance` 新增 5 条：解析（含 `clear` → `None`）、参数缺失/未知子命令、
    `list` 的 declared/effective/source、`set-window` 只改一个键、未命中模型零写入。
