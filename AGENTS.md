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
- **技术决策记录 (ADR)**：[`.agents/notes/`](.agents/notes/)（最高优先级决策真源，技术方案选型、现在时记录、替代方案考量；受 `adr-verify` 强门禁保护）
- **架构与设计指南**：[`docs/architecture/`](docs/architecture/)（系统拓扑、模块交互、运行时设计）
- **待办任务与执行账本 (Backlog)**：[`backlog/`](backlog/)（唯一活账本 [`tasks.jsonl`](backlog/tasks.jsonl)，支持任务级并发租约锁与行级强校验；契约见 [`backlog/README.md`](backlog/README.md)，由 `scripts/backlog.mjs` 驱动、`backlog-gate`（lint + doctor）门禁约束）
- **工程复用技能库**：[`.agents/skills/`](.agents/skills/)
- **历史归档档案（已冻结退役，只读）**：
  - OpenSpec 规范归档：[`openspec/`](openspec/)（历史规格与设计草案，见 [`RETIRED.md`](openspec/RETIRED.md)）
  - 历史决策归档：[`.agents/notes/archived/`](.agents/notes/archived/)（早期 0001~0123 篇 ADR 均已全量重构迁移并封印，核心架构见 `.agents/notes/implemented/architecture/`）

---

## 2. 核心架构约束与不变量

1. **Rust 核心与前端渲染职责分离**：
   - 调度、沙箱执行、持久化与大模型交互必须位于 `crates/pony-agent-core` 及 `src-tauri`。
   - 前端（Vue 3 / TypeScript）仅作交互界面与状态展示，模型网络流量严禁直接走前端 `fetch`。
2. **外部安全边界（Fail-Closed）**：
   - 桌面打开外部链接统一调用 `src/lib/open-external-url.ts`，Rust 后端强制执行 HTTPS 协议与严格 host 白名单（真值源：`src-tauri/src/platform.rs` 的 `ALLOWED_OPEN_HOSTS`，当前四项：`github.com`, `api.github.com`, `exa.ai`, `s3.local.ponyjob.top`），Windows 平台使用 `ShellExecuteW` 唤起默认浏览器，严禁使用命令拼接。白名单增删必须与本条文档同提交更新。
3. **版本四处同步契约**：
   - `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`、`.version.json` 必须保持严格同步。
   - 任何涉及版本的提交由 `scripts/check-version-sync.ps1` 校验。

---

## 3. 本地与 CI 门禁命令速查

- **全量合规自检**：`npm run verify`（依次校验版本同步、决策记录、账本门、前端单测、构建、Rust 检查）
- **决策记录校验**：`npm run adr:check`（判定真源 `.agents/skills/write-adr/verify-note.sh`，pre-commit 调 `adr-verify.sh`）
- **版本同步校验**：`npm run version:check`
- **前端测试**：`npm test`
- **Rust 编译检查**：`npm run cargo:check:shared`
- **可观测性归属先决**：新增/修改对外接口、网络 IO、后台任务、LLM 调用或 UI 交互前，先更新 `docs/observability.yaml` 的四维归属（适用 / N-A + 结构性理由）与收口点；验收跑
  `~/.agents/skills/observability-onboarding/scripts/audit-diff.sh` 与 `check-declaration.sh`（判定口径见该 skill 的 `references/declaration-rubric.md`）。N-A 必须写结构性理由，不得为让门禁变绿而伪造 `/metrics` 或改 `applicable`。
<!-- BEGIN constitution -->

## 常载命约（Standing Orders）

1. 任何非平凡变更都要落为 `.agents/notes/` 下一条带 `## Alternatives considered` 的决策记录（程序与校准样例见 `.agents/skills/write-adr/SKILL.md`）。
2. 凡机械可查的承诺，配一条非零退出的命令；机器到不了的，显式标注「靠 review」。
3. 删除代码前先搜消费者；恢复条件立法，拒绝「以后可能用得上」式怀旧。
4. 每个事实一个家：文档治理规则的家是 `docs/AGENTS.md`（agent 自动加载），人读契约是 README；改行为/契约的那次提交同步更新对应文档。
5. 非平凡交付必须在 backlog 账本留痕（record / start / resolve）；账本一致性与租约对账由 `backlog-gate`（lint + doctor）机械校验，证据成立性靠 review。
<!-- END constitution -->

