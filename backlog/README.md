# backlog/ —— 本仓待办与执行账本（v3 批次日志 + 按批归档封存）

> **判定真源 = `scripts/backlog.mjs`**（`validateTask` / `lintV2` / `lintV3` / `fold` / `doctor` /
> `verifyArchive`）。本文是它的可读契约：冲突时以代码与命令输出为准，本文只描述模型，
> 不复刻判据细节。模型版本由 `backlog/MODEL`（内容 `v3`）声明。

## 为什么是 v3（2026-11-22）

v2 把全部任务写进同一个 `backlog/tasks.jsonl`，并用 `backlog/.next-id` 做全局计数器：
**两个并发会话只要都记账，就必然在同一文件的行尾与同一计数器上撞车**（实证：#40 与 #41
的账本冲突，人工解一次还不够，下个 PR 又来）。

v3 把「一次会话按发布面选出的那批任务」做成账本的**一等写入单元**：

- **写**：每个会话/发布面批次一个 `backlog/log/<batchId>.jsonl`，只追加、永不改他人文件；
- **读**：`fold` 把所有批次 + 归档批次按时间折叠出现行视图，不落盘（现算，不算错）。

⇒ 不同会话 = 不同文件 = git 合并天然零冲突；一个 PR 带多个任务也只是一个（或几个）批次文件，
reviewer 看到的就是「这个发布面批次做了什么」。

## 布局

| 路径 | 角色 | 入仓 |
|---|---|---|
| `backlog/MODEL` | 模型标记（内容 `v3`） | ✅ |
| `backlog/log/<batchId>.jsonl` | **活跃批次日志**（只追加；本会话只写自己的文件） | ✅ |
| `backlog/archive/batches/<year>/<batchId>.jsonl` | 已归档批次（只读，按批封存） | ✅ |
| `backlog/archive/batches/<year>/manifest.sha256` | 归档清单（`sha256  <相对路径>`，只追加） | ✅ |
| `backlog/archive/tasks-<YYYY>.jsonl` | v2 遗留归档（只读，fold 时并入历史） | ✅ |
| `backlog/.locks/<ID>.lock` | 任务租约（本地运行时：pid/会话/心跳） | ❌ 已 gitignore |
| `backlog/.state/batch` | 当前工作区活动批次 id（本地运行时） | ❌ 已 gitignore |

## 批次（batch）

- 命名：`<YYYYMMDD>T<HHMMSS>Z-<surface>-<rand4>`（UTC；`surface` 只允许 `[a-z0-9-]`），
  例如 `20261122T051325Z-tui-0b59`。
- 首行必须是 `batch.start`：`{kind, model:"backlog-v3", batch_id, surface, release?, created_at, session_id?}`。
- `surface` = 发布面（`tui` / `frontend-runtime` / `acp` / `docs` …）。`backlog list --surface tui`
  直接列该面批次内的任务。
- 写命令默认复用本工作区已有批次（`backlog/.state/batch`）；首次写自动创建；
  `backlog batch new --surface <s> [--release <v>]` 显式开新批次。

## 事件（一行一个 JSON）

| kind | 负载 | 语义 |
|---|---|---|
| `batch.start` | `batch_id, surface, release?, created_at, session_id?` | 批次头（每文件恰好一行） |
| `record` | `at, task:{完整任务行}` | 新建任务（ID 全局唯一；JSON 字段与 v2 记录一致） |
| `dedup` | `at, id, context?` | 同 `fingerprint` 去重命中（`occurred_count+1`） |
| `start` | `at, id, owner, paths[], session_id?` | 进入 `in_progress`（取租约） |
| `release` | `at, id, owner?` | `in_progress → todo`（释放租约） |
| `annotate` | `at, id, description?, priority?` | 更新描述/优先级 |
| `resolve` | `at, id, evidence` | 闭环（终态；释放租约） |

## 记录模型（任务行，与 v2 字段逐字一致）

- 必填：`id`（`B\d{3,}`）/ `category`（`Defect|Task|Debt`）/ `priority`（`P0..P3`）/
  `status`（`todo|in_progress|done`）/ `title` / `created_at` / `metadata.fingerprint`。
- 条件必填：`in_progress` → `owner` + `touched_paths`（≥1）；`done` → `evidence` + `closed_at`。
- 未知字段一律拒绝；`description` ≤2000 字符；`metadata.triage_context` ≤8KiB。

## ID 与状态机

- **新 ID = `B` + epoch 毫秒**（13 位，仍满足 `B\d{3,}`）：跨会话按时间取号，天然不撞号；
  老 ID（`B265` 等短号）原样保留，`backlog:B265` 引用不变。
- 状态机：`todo → in_progress → done`；允许 `in_progress → todo`；**`done` 是终态**——
  回归请新开任务并引用旧 ID（归档只读，见下）。`done` 只能由 `resolve` 写入。
- 手改批次日志的状态跳变不被 CLI 拦截，但「记录非 `in_progress` 却仍持锁」会被 `lint`/`doctor`
  判违规。

## 租约（并发控制）

- `start` 以 `O_EXCL` 原子获取 `<ID>.lock`：同一任务同时只允许一个会话持有；`heartbeat` 刷新；
  `release` 显式释放；`reap --id <ID> --minutes N` 回收指定任务的超龄租约（`--dry-run` 只打印）。
- `.locks/` 是本地运行时状态，不入仓：干净克隆/CI 上缺锁只提示（`NOTE`），不阻断；
  只有在「本工作区确实存在 `.locks/`」时才把缺锁判为手改状态的强信号。

## 归档（按批封存，照 ADR 机制）

- 时机：**批次内任务全部 `done` 且随某个版本发布之后**，由发版 housekeeping 或每周兜底任务执行；
  **未闭环批次不归档**（否则要开新批次补正）。
- 动作：`backlog archive [--release <v>] [--before <ISO>] [--dry-run]` → 批次文件
  `git mv` 到 `archive/batches/<year>/` + 追加 `manifest.sha256` 行 + 删 `log/` 原件。
- 冻结：归档批次**只读**；`verify-archive` 逐文件校验 sha256，被改动/未入清单即红。
- fold 同时读「活跃批次 + 归档批次 + v2 遗留归档」，所以归档不丢历史。

## 命令

```bash
node scripts/backlog.mjs record --category Defect --title "..." --fingerprint "core::x::y" --priority P1 [--surface tui] [--release 0.1.203]
node scripts/backlog.mjs start B123 --owner lane-1 --paths "a/b.rs,c/d.rs"
node scripts/backlog.mjs heartbeat B123 --owner lane-1
node scripts/backlog.mjs release B123
node scripts/backlog.mjs resolve B123 --evidence "npm test 全绿"
node scripts/backlog.mjs annotate B123 --priority P2 --description "..."
node scripts/backlog.mjs list [--surface tui]
node scripts/backlog.mjs fold                  # 现算视图（JSON：活跃/批次/问题）
node scripts/backlog.mjs lint
node scripts/backlog.mjs doctor
node scripts/backlog.mjs archive [--release <v>] [--before <ISO>] [--dry-run]
node scripts/backlog.mjs verify-archive
node scripts/backlog.mjs batch {new [--surface <s>] [--release <v>]|show}
node scripts/backlog.mjs reap [--id B123] [--minutes N] [--dry-run]
```

## 门禁与验收

- 门禁：`bash .meta/gates/backlog-gate.sh`（v3 = `lint` + `doctor` + `verify-archive`；
  v2 过渡期 = `lint` + `doctor` 只读兼容 + NOTE），已接 pre-commit、CI 与 `npm run verify`；
  负样本自检：`bash .meta/gates/backlog-gate.spec.sh`（P1–P3 放行 + N1–N8 拒绝 + pre-commit 接线）。
- 本地钩子前提：`git config core.hooksPath .githooks`（新 clone 跑一次 `npm run hooks:install`）。

## 迁移（v2 → v3）

```bash
node scripts/backlog-migrate-v2-v3.mjs --dry-run            # 先看计划
node scripts/backlog-migrate-v2-v3.mjs --surface legacy-migration [--release vX]
node scripts/backlog.mjs lint && node scripts/backlog.mjs doctor && node scripts/backlog.mjs verify-archive
git add backlog/MODEL backlog/log backlog/archive && git rm backlog/tasks.jsonl backlog/.next-id
```

脚本做的事：把现有活跃任务写成**一个基线批次**（原 ID/字段不变）→ 临时写 `MODEL` 用 v3 fold
对账（活跃数、done 数必须一致，不一致自动回滚）→ 删除 `tasks.jsonl` 与 `.next-id`；v2 遗留归档
`tasks-<YYYY>.jsonl` 保留只读。迁移是断言式失败关闭的，**不可重复执行**（MODEL 已存在 → exit 2）。

## 禁止

- 手写批次事件冒充 CLI 写入（`lint`/`doctor` 会抓结构、时间倒退、重复 record/resolve）；
- 修改已归档批次（`verify-archive` 抓 sha256）；
- 在子 worktree 重复 init 第二份账本实例（账本根 = 当前 worktree 顶层，批次随各自 PR 走）；
- 复用已归档任务的 ID（`done` 是终态，回归开新任务并引用旧 ID）。
