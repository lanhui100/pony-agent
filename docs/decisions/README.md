# Pony Agent 决策记录

本目录记录 Pony Agent 的重要技术决策（ADR）。

## 状态枚举

每篇 ADR 标题下方第一行是 `Status:`，取值只有四种：

- `Status: proposed` —— 提案，尚未实施；
- `Status: implemented` —— 决定已采纳并落地。注意：本状态描述的是**决定的采纳**，不是工程的完工度——分阶段工程的阶段进度属于任务系统 / OpenSpec，不改变此状态；
- `Status: superseded by NNNN` —— 已被后续 ADR 接替，正文不再代表现行决定（文件移入 `superseded/`）；
- `Status: rejected — <一行理由>` —— 提案被否决（文件移入 `rejected/`）。

## 目录布局

- 主目录存放活跃的 implemented ADR；
- `superseded/`、`rejected/`、`proposed/` 子目录在对应状态的首个文件出现时创建，此后同状态的文件移入同名目录——**目录即状态**；
- 编号一经分配永不复用、永不修改；同一编号只允许对应一个文件；
- **撞号消解**：若同一编号被发现对应两个文件（并发写入产物），保留被外部引用更多的一篇的原号，另一篇顺延到下一个空闲号——只改文件名与 H1，slug 不变，正文一字不改，旧号永不回用；历史封存件（`management/task-system/99_LOGS/`、`.dev-team/` 内的 wave 契约与 state-archive）里的旧引用**不回改**，对照关系写进被改号那篇顶部的「编号修订」块（裁定见 [ADR 0109](0109-decisions-gate-single-source-and-collision-rule.md)）；
- 状态取值只有上面四种（`proposed` / `implemented` / `superseded` / `rejected`），不引入 `accepted` 之类同义词——「决定已采纳但工程未完工」仍写 `implemented`，进度归任务系统；
- 机械校验：`npm run decisions:check`（判定真源 `scripts/check-decisions.mjs`，跨平台，pre-commit 直接调它）。索引缺行可用 `npm run decisions:fix` 机械回填，但该脚本只搬 H1 标题、不代写摘要。

## 何时必须写 ADR

同一提交内新增或更新一篇 ADR，当且仅当变更命中以下任一条：

1. 改变运行时行为或用户可见行为；
2. 引入或修改跨模块契约（接口、事件、持久化格式、配置结构）；
3. 推翻或修订既有 ADR 的决定；
4. 改变开发流程、工具链或测试策略；
5. 新增一类依赖或外部服务；
6. 一次删除跨越多个模块（简化类决策）。

纯机械改动（重命名、格式化、不触及上列各项的局部修复）豁免。拿不准时写一篇三行 ADR——多记的成本是一段文字，漏记的成本是永久丢失。

## 记录规范

- **时态**：implemented 用现在时描述既成事实；proposed 用将来时；rejected 冻结提案原文，只在 Status 行追加否决理由。
- **候选方案**：每篇 ADR 必须包含"候选方案"节，记录真实比较过的选项及落选原因。备选只能记录、不能编造：2026-08-22 之前产生的既有 ADR 若备选已不可考，以 `<!-- alternatives-not-recorded -->` 如实标注；此日之后的 ADR 不再接受该标记。
- **取代**：推翻旧决定 = 新增 ADR + 在旧篇 Status 行标注 `superseded by NNNN` 并互链。禁止通过改写旧篇正文来"更新"决定。
- **拒绝的取舍**：被否决的想法仅在其仍可能诱惑后续贡献者重新提出时才写入 `rejected/`；明显不会被重提的否决不留痕。
- **事实同步**：implemented ADR 中引用的路径、名称等事实随代码变更在同一提交内更新；决定本身不变。

## 索引

分节与状态一一对应：`### 提案中`(proposed) / `### 活跃`(implemented) / `### 已接替`(superseded) / `### 已拒绝`(rejected)。
每篇 ADR 必须在**自己状态对应的分节**里有且仅有一行，且该行的**第一个链接**指向自己（行尾的修订互链不参与判定）；
索引区不得出现指向不存在文件的链接。以上四条由 `npm run decisions:check` 把守。

### 提案中

- [0021 Agent 运行时统一错误码注册表：上游与协议不匹配可扩展治理](proposed/0021-unified-error-code-registry.md)
- [0074 Ask 控制请求：会话持久化 + 生命周期收口（墙钟 TTL 退出产品语义）](proposed/0074-ask-persistence-lifecycle.md)

### 活跃

- [0107 会话级写者租约与日志优先持久化（多窗口/多智能体并发，预留远程 agent）](0107-session-scoped-writer-lease-and-log-first-durability.md)
- [0001 双轨重构：保留 Hermes 作为参考实现](0001-dual-track-rebuild-with-hermes-reference.md)
- [0002 优先使用 Tauri UI 作为运行时测试界面](0002-tauri-ui-over-tui.md)
- [0003 前端使用 Vue 3 + Pinia，暂不上 vue-router](0003-frontend-stack-vue-pinia-no-router.md)
- [0005 第一阶段视觉方向：暖色极简的研究工作台风格](0005-visual-direction-warm-minimalism.md)
- [0006 API Key 存储从 env-first 演进到统一 SecretStore](0006-api-key-evolution-env-first.md)
- [0007 将缓存命中提升为一等产品指标，按阶段落地](0007-cache-hit-as-first-class-product-metric.md)
- [0008 会话数据架构向事件溯源演进（分阶段落地）](0008-event-sourcing-evolution.md)
- [0010 CI rust-cache 对齐 target 槽位与最小令牌权限](0010-ci-rust-cache-slot-alignment-and-minimal-token.md)
- [0011 版本 bump 自动同步 Cargo.lock](0011-bump-version-syncs-cargo-lock.md)
- [0013 工作台导航精简：观测右栏浮动入口、配置页收敛与提供商页层次折叠](0013-workbench-nav-observation-entry-and-provider-hierarchy.md)
- [0014 提供商协议三值化与模型目录：Responses 适配、模型级覆盖与收起取消编辑](0014-provider-protocol-tri-value-and-model-catalog.md)
- [0015 侧边栏三级树的持久化字段、删除归属重写与 dialog 插件依赖](0015-workspace-sidebar-persistence-and-dialog-plugin.md)
- [0017 发版流水线约定、四处版本一致性同步与安全加固](0017-release-tag-convention-and-tauri-version-sync.md)
- [0018 Windows Job Object 进程生命周期约束](0018-windows-job-object-containment.md)
- [0019 本地契约优先的签名桌面更新](0019-signed-desktop-updater-local-contract.md)
- [0020 侧边栏工作区会话折叠与选中高亮浅色化](0020-workspace-sidebar-collapsible-and-selection-tone.md)
- [0022 桌面端默认工作区基座保持与兜底收敛注册表 default](0022-desktop-default-workspace-registry-convergence.md)
- [0023 Windows 控制台子进程隐藏窗口与输出代码页解码回退](0023-windows-console-hidden-and-codepage-decode-fallback.md)
- [0025 基于 Conventional Commits 的发布说明生成器](0025-release-notes-from-conventional-commits.md)
- [0026 基础工具体系加固（CAS 乐观并发写守卫与富语义错误自愈协议）](0026-tool-cas-and-recovery-hint.md)
- [0027 端到端全链路 EventBus 架构设计](0027-end-to-end-event-bus-architecture.md)
- [0028 将 Sentry 问题收集下沉至 Harness 宿主层与 Agent Trace 解耦](0028-decouple-sentry-issues-to-harness.md)
- [0029 工作区外部访问提权机制（基于 DSH 的升权引导与受控授权协议）](0029-workspace-permission-escalation.md)
- [0030 对话框面板内聚 Ask 提问交互优化](0030-composer-integrated-ask-interaction.md)
- [0031 多模态附件统一受控接入与模型能力按需投影](0031-multimodal-attachment-ingress-and-model-projection.md)
- [0032 graph_runs 非重入锁作用域纪律与桌面端会话页卡死修复](0032-graph-runs-lock-scope-discipline.md)
- [0033 运行状态条移出滚动内容流：composer 上沿常驻覆盖层与 streaming 期隐藏](0033-runtime-status-bar-out-of-scroll-flow.md)
- [0034 删除/归档当前对话后一律进入全新空白会话](0034-fresh-session-after-delete-or-archive.md)
- [0035 Token 指标口径：输入上下文取单次调用、生成速度按调用级即时产出](0035-token-metrics-per-call-semantics.md)
- [0036 IME 组合态回车守卫与动态数字位宽稳定](0036-ime-composition-enter-guard-and-numeric-width-stability.md)
- [0037 废除 PONYSENTRY_ENABLED：开关恒开，门禁由凭证承担](0037-remove-ponysentry-enabled-switch.md)
- [0039 wave-37 工具面扩展、write_file 读前写观察门与子代理真实执行引擎](0039-wave37-tool-surface-write-gate-subagent-engine.md)
- [0040 TUI /models 远程模型发现与回环地址代理旁路](0040-tui-model-discovery-and-loopback-proxy-bypass.md)
- [0038 TUI 设计系统：单色为骨、单一骨色强调、发丝线与 2 列沟槽](0038-tui-design-system-monochrome-restraint.md)
- [0041 Agent Team 端到端协作体系与全双工任务板架构](0041-agent-team-end-to-end-architecture.md)
- [0042 TUI Composer 收边与底部指标行（弱化 cwd / token / 缓存 / 上下文）](0042-tui-composer-metrics-line.md)
- [0043 TUI 消息区层级弱化、代码块材质与三态状态词](0043-tui-message-hierarchy-and-tri-state-status.md)
- [0044 系统提示词 v2——收据化、注入防线、阈值收敛与层序守卫](0044-system-prompt-v2-receipts-injection-and-layer-order.md)
- [0045 用户级（全局）指令与 Skills 分层加载](0045-global-instructions-and-skills-layering.md)
- [0046 全局配置加载四项能力：技能正文按需加载 / 指令预算 / 层级发现 / 触达即载](0046-global-config-loading-wave42.md)
- [0047 Agent Trace 迁移至 OpenTelemetry → Langfuse（PonySentry Trace 下线 + 全局上报开关）](0047-agent-trace-otel-langfuse-migration.md)
- [0048 用量治理硬限流的判定落点与 fail-open 策略](0048-usage-governance-enforcement.md)
- [0049 OTel 准入 token 改为构建期烧录（用户零配置）](0049-otel-build-time-token-distribution.md)
- [0050 配置页「全局轨迹上报」选项收敛为单行标签 + 右端开关](0050-trace-reporting-switch-ui-simplification.md)
- [0051 可观测性上报三项修正（版本单一来源、治理 span 会话归属、思考/缓存 token 出口）](0051-observability-version-session-and-thinking-token-fixes.md)
- [0052 可观测性语义轨补全（成本明细、未计价标记、枚举收敛、降级标记、可读 trace 名、工具调用 id）](0052-observability-semantic-completeness-fixes.md)
- [0055 真实 trace 审计驱动的四项语义缺陷与三项可观测性优化](0055-trace-audit-fixes-usage-ttft-sequence-error-rate.md)
- [0056 流式正文渲染收敛（stale-while-render）与运行状态 hop 级判定](0056-streaming-render-stale-while-render-and-hop-status.md)
- [0053 token / 速度口径真值契约（wave-54）](0053-token-speed-truth-contract.md) —— 其中「速度 = Σout/Σgen」的公式定义已被 [0073](0073-token-speed-decode-basis.md) 修订；上下文归一、usage 来源与显示纪律部分仍现行
- [0054 渲染层外泄通道收口：启用 CSP、markdown 外部资源 fail-closed 与密钥不回显](0054-render-layer-exfil-csp-and-secret-masking.md)
- [0057 流式动效抽象为框架无关规范（五层契约 + 纯 reducer + 机器门禁）](0057-streaming-motion-framework-agnostic-spec.md)
- [0058 流式释放与视口阈值接入 PDS-SM reducer（等价重构 + 常量单源化）](0058-pds-budget-reducer-adoption.md)
- [0059 流式视觉层接入：允许单个 inline 尾巴节点（修订 D-59-1 与 F3-1 部分断言）](0059-streaming-visual-layer-tail-node.md)
- [0060 恢复请求保留完整上下文 + 轮次止损与请求形状可观测](0060-turn-context-integrity-and-loop-guards.md)
- [0061 工具 span 采集脱敏截断的入参/结果预览](0061-tool-span-payload-previews.md)
- [0062 TUI 退出还原与 `pony resume <session-id>` 续接入口](0062-tui-exit-restore-and-resume-entry.md)
- [0063 TUI Ask 覆盖层与取消反馈的用户可见边界](0063-tui-ask-user-overlay-and-cancel-feedback.md)
- [0070 TUI 会话韧性：DSML 折叠、错误摘要、中断检查点与退出回收](0070-tui-session-resilience.md)
- [0071 TUI 流式期可输入、面板可开、用户消息排队](0071-tui-streaming-input-queue-and-panels.md)
- [0072 会话 id 生成规范统一为 `pa_<uuidv7>`](0072-session-id-unification.md)
- [0073 token 速度口径 v2：解码窗口分母 + 物理下限 + basis 自描述](0073-token-speed-decode-basis.md)
- [0075 TUI 键盘协议还原加固：显式置零、复用器最小化 push、失败可告知](0075-tui-kitty-keyboard-protocol-restore-hardening.md)
- [0076 TUI 思考（reasoning）消息可见与消息区底部呼吸间距](0076-tui-thinking-message-and-chat-bottom-gap.md)
- [0078 TUI header 承载工作区标签与数值指标，底面行回归模型身份](0078-tui-header-workspace-and-metrics-layering.md)
- [0079 TUI 右侧滚动条、思考折叠、`/settings` 面板与 Agent 过程默认隐藏](0079-tui-process-fold-scroll-settings.md)
- [0080 TUI 品牌标图形协议改为零 I/O 环境判定：禁止启动期 stdio 能力查询](0080-tui-splash-protocol-env-only-detection.md)
- [0081 Ask 内联面板高度随问题换行增长，硬上限按终端宽度分档（40 / 24）](0081-tui-inline-ask-height-adaptive.md)
- [0082 会话投影剔除宿主续轮提示：标题、轮次与 `/resume` 列表只认真实用户轮](0082-session-projection-excludes-host-continuation.md)
- [0083 TUI 子会话导航：Ctrl+↑ 返回主会话、Ctrl+←/→ 切换队友、输入区身份徽标](0083-tui-sub-session-navigation-and-badge.md)
- [0084 TUI header 指标双口径：`↑in` 当前上下文、`↓out` 与缓存命中率会话累计](0084-tui-metrics-dual-caliber-session-cumulative.md)
- [0086 provider 上游失败分类补齐：边缘 5xx 与传输层标志纳入可重试](0086-provider-transport-and-edge-5xx-retry-classification.md)
- [0087 CLI 自更新：`pony update` 走 GitHub Releases + 强制 sha256 + POSIX 原子替换](0087-cli-self-update.md)
- [0088 TUI 用户消息上下各留 1 行页面底色间距](0088-tui-user-message-vertical-gap.md)
- [0089 tui.rs 原地模块化拆分，保留 bin 入口与 `tui::` 可见性](0089-tui-module-decomposition.md)
- [0090 代码规模门禁采用「基线棘轮」，只判新增超限](0090-code-size-gate-ratchet.md)
- [0091 删除单轮累计墙钟预算，止损下沉到能力层](0091-turn-wall-clock-removal.md)
- [0092 Ask 自锁循环的三处收口（续轮让位 / 续轮文案识别 / Draft 撤销与宿主批准）](0092-plan-draft-self-lock-recovery.md)
- [0094 herdr 复用器变量名对齐 与 会话索引行丢失自愈](0094-tui-multiplexer-env-and-session-row-heal.md)
- [0095 `pony update` 控制面改用空闲读超时 + 30s 总预算，body 读取纳入重试](0095-cli-update-control-plane-timeout.md)
- [0096 TUI 状态行 min-dwell + 轮内工具活动聚合（折叠态「生成中」不再吞掉过程信息）](0096-tui-status-line-min-dwell-and-tool-activity.md)
- [0097 `pony update` 同号不同内容不得谎报「已是最新」，发布侧清单版本必须严格递增](0097-cli-update-same-version-content-mismatch.md)
- [0098 TUI 启动页品牌标改用母版真色派生表 + 位图优先（含 halfblocks）](0098-tui-splash-master-truecolor-bitmap-first.md) —— 档位尺寸经 [0105](0105-tui-splash-logo-scale-third.md) 缩小后，由 [0112](0112-tui-splash-logo-legibility-floor.md) 修订为 Rich 14×16 / Compact 11×12、位图 448×512；真色表与位图优先部分仍现行
- [0099 会话存储改为「事件溯真源 + 单写者 + 墓碑删除」四阶段重构](0099-session-storage-event-sourced-refactor.md)
- [0100 TUI 模型面板：优选行参与统一导航与分区间 1 行间距](0100-tui-model-panel-preferred-selectable-and-section-gap.md)
- [0101 Trace 审核 Loop：用户唤起 + 持久状态机 + 确定性骨架 + 单一审批门](0101-trace-audit-loop-automation.md)
- [0102 为日志格式补齐事件载体（provider transcript / 长期记忆 / history_state_evidence）](0102-session-log-event-carriers-for-transcript-and-memory.md)
- [0103 goal 续轮失败止损、失败轮不烧预算与上游限流冷却](0103-goal-round-failure-and-rate-limit-cooldown.md)
- [0104 上游调用时限/重试配置化与跨 provider 有界故障转移](0104-provider-failover-and-upstream-policy.md)
- [0105 TUI 启动页品牌标缩为原尺寸 1/3（Rich 9×10 / Compact 7×8）](0105-tui-splash-logo-scale-third.md) —— 尺寸已由 [0112](0112-tui-splash-logo-legibility-floor.md) 修订
- [0108 CLI 发布平台矩阵、清单单点装配与词表对齐门禁](0108-cli-release-platform-matrix.md)
- [0109 决策记录门禁单一判据与撞号消解规则](0109-decisions-gate-single-source-and-collision-rule.md)
- [0110 wave-35 测试套件修复：全量挂起真死锁、plan 草稿门禁自锁死与读前写契约漂移](0110-wave35-suite-remediation.md)
- [0111 密钥门 64-hex 规则：snake_case 摘要键与结构化摘要槽的误报收口](0111-secret-gate-snake-case-digest-keys.md)
- [0112 TUI 启动页品牌标按面积约 1/3 定栅格，并以实心子像素占比设可读性下限](0112-tui-splash-logo-legibility-floor.md)
- [0113 TUI 双击 Esc 撤回：轮次级会话截断 + 写前 pre-image 文件还原](0113-tui-esc-double-press-rewind.md)

- [0064 Harness 可扩展性三层模型与不可替换内核](0064-extensibility-three-layer-model.md)
- [0065 工具面单一真值闭合与外部工具通道（T1）](0065-tool-surface-single-truth.md)
- [0066 MCP 通道闭合（T2）](0066-mcp-channel-closure.md)
- [0067 进程外插件宿主协议 v1 与 hooks 沙箱加固（T3）](0067-plugin-host-protocol-and-hooks-sandbox.md)
- [0068 五份契约版本化、conformance kit 与扩展点归属表（T4）](0068-contract-versioning-and-conformance-kit.md)
- [0077 审批类控制请求可恢复：宿主面板 + 批准后授权并续轮](0077-approval-resume-host-panel-and-continuation.md)

### 已接替

- [0106 会话库单写者冲突 fail-closed、退出提示真实性与启动装载预算](superseded/0106-session-store-single-writer-fail-closed-and-startup-budget.md) —— 被 [0107](0107-session-scoped-writer-lease-and-log-first-durability.md) 取代锁语义（启动预算/退出提示能力保留）
- [0024 PonySentry Agent Trace 上报契约与生命周期状态机](superseded/0024-ponysentry-agent-trace-ingest.md) —— 被 [0047](0047-agent-trace-otel-langfuse-migration.md) 接替（服务端 Trace 能力下线，改走 OTel → Langfuse）

- [0004 前端工作台先采用原生 TypeScript + Vite 壳层](superseded/0004-frontend-shell-workbench-direction.md) —— 被 [0003](0003-frontend-stack-vue-pinia-no-router.md) 接替
- [0009 工作台信息架构：trace/metrics 二级遥测页与配置页 tab 化](superseded/0009-workbench-ia-telemetry-page-and-config-tabs.md) —— 被 [0013](0013-workbench-nav-observation-entry-and-provider-hierarchy.md) 部分接替（入口决定修订，其余承接重述）
- [0012 应用内更新检测：前端直连 GitHub Releases 与构造式发布页跳转](superseded/0012-app-update-check-via-github-releases.md) —— 被 [0017](0017-release-tag-convention-and-tauri-version-sync.md) 接替
- [0069 初始化页品牌标走终端图形协议（sixel/kitty/iTerm2），字符画降级为回退](superseded/0069-tui-splash-image-protocol.md) —— 被 [0080](0080-tui-splash-protocol-env-only-detection.md) 接替（启动期 stdio 能力探测改为零 I/O 环境判定；图形协议 + 位图资产 + 字符画回退其余决定承接重述）
- [0085 TUI 启动页品牌标改为官方轮廓覆盖率重采样 + 双色 alpha 合成](superseded/0085-tui-splash-logo-fidelity-and-antialiasing.md) —— 被 [0098](0098-tui-splash-master-truecolor-bitmap-first.md) 接替（配色真值源从自造 OKLab C2 渐变改为母版真色派生表；位图路径不再排除 halfblocks；Rich 档 27 列收为 26 列）

### 已拒绝

- [0016 否决改写 Git 历史以清除已提交调试截图](rejected/0016-reject-git-history-rewrite-for-debug-png.md)
