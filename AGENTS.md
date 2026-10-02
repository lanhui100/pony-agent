# AGENTS.md —— Pony Agent 工程治理总则与常载规则

> 本文是面向所有 Agent、开发团队与协作工具的统一导航索引与常载行动准则。

## 1. 体系入口地图（Tier 索引）

为杜绝事实重复与知识腐化，本项目所有事实严格执行分层存放：

- **文档标准与写作契约**：[`docs/AGENTS.md`](docs/AGENTS.md)（tier 分类法、写作规则、防腐化自查表）
- **工程宪章与治理根**：[`.meta/`](.meta/)
  - 成熟度与治理状态：[`.meta/meta.yaml`](.meta/meta.yaml)（当前定级：L2 承诺可验+文档分层）
  - 核心架构宪法：[`.meta/constitution/constitution.md`](.meta/constitution/constitution.md)
  - 门禁索引说明：[`.meta/gates/README.md`](.meta/gates/README.md)
  - 文档分层清单：[`.meta/docs-tier/README.md`](.meta/docs-tier/README.md)
- **技术决策记录 (ADR)**：[`docs/decisions/`](docs/decisions/)（技术方案选型、现在时记录、替代方案考量；废弃方案归档至 `superseded/`）
- **架构与设计指南**：[`docs/architecture/`](docs/architecture/)（系统拓扑、模块交互、运行时设计）
- **任务推进与多智能体评审**：[`management/task-system/`](management/task-system/)
  - 系统总看板：[`00_DASHBOARD.md`](management/task-system/00_DASHBOARD.md)
  - 任务板：[`01_TASK_BOARD.md`](management/task-system/01_TASK_BOARD.md)
  - 独立评审记录：[`02_REVIEWS/`](management/task-system/02_REVIEWS/)
  - 任务卡：[`03_TASKS/`](management/task-system/03_TASKS/)
- **工程复用技能库**：[`.agents/skills/`](.agents/skills/)

---

## 2. 核心架构约束与不变量

1. **Rust 核心与前端渲染职责分离**：
   - 调度、沙箱执行、持久化与大模型交互必须位于 `crates/pony-agent-core` 及 `src-tauri`。
   - 前端（Vue 3 / TypeScript）仅作交互界面与状态展示，模型网络流量严禁直接走前端 `fetch`。
2. **外部安全边界（Fail-Closed）**：
   - 桌面打开外部链接统一调用 `src/lib/open-external-url.ts`，Rust 后端强制执行 HTTPS 协议与严格 host 白名单（`github.com`, `api.github.com`, `exa.ai`），Windows 平台使用 `ShellExecuteW` 唤起默认浏览器，严禁使用命令拼接。
3. **版本四处同步契约**：
   - `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`、`.version.json` 必须保持严格同步。
   - 任何涉及版本的提交由 `scripts/check-version-sync.ps1` 校验。

---

## 3. 本地与 CI 门禁命令速查

- **全量合规自检**：`npm run verify`（依次校验版本同步、前端单测、构建、Rust 检查）
- **版本同步校验**：`npm run version:check`
- **前端测试**：`npm test`
- **Rust 编译检查**：`npm run cargo:check:shared`
