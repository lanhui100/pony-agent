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
- 编号一经分配永不复用、永不修改。

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

### 活跃

- [0001 双轨重构：保留 Hermes 作为参考实现](0001-dual-track-rebuild-with-hermes-reference.md)
- [0002 优先使用 Tauri UI 作为运行时测试界面](0002-tauri-ui-over-tui.md)
- [0003 前端使用 Vue 3 + Pinia，暂不上 vue-router](0003-frontend-stack-vue-pinia-no-router.md)
- [0005 第一阶段视觉方向：暖色极简的研究工作台风格](0005-visual-direction-warm-minimalism.md)
- [0006 API Key 存储从 env-first 演进到统一 SecretStore](0006-api-key-evolution-env-first.md)
- [0007 将缓存命中提升为一等产品指标，按阶段落地](0007-cache-hit-as-first-class-product-metric.md)
- [0008 会话数据架构向事件溯源演进（分阶段落地）](0008-event-sourcing-evolution.md)
- [0009 工作台信息架构：trace/metrics 二级遥测页与配置页 tab 化](0009-workbench-ia-telemetry-page-and-config-tabs.md)
- [0010 CI rust-cache 对齐 target 槽位与最小令牌权限](0010-ci-rust-cache-slot-alignment-and-minimal-token.md)
- [0011 版本 bump 自动同步 Cargo.lock](0011-bump-version-syncs-cargo-lock.md)

### 已接替

- [0004 前端工作台先采用原生 TypeScript + Vite 壳层](superseded/0004-frontend-shell-workbench-direction.md) —— 被 [0003](0003-frontend-stack-vue-pinia-no-router.md) 接替
