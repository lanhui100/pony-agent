# backlog/ —— 本仓待办与执行账本（唯一活账本）

> **判定真源 = `scripts/backlog.mjs`**（`validateRecord` / `lint` / `doctor`）。本文是它的可读契约：
> 冲突时以代码与命令输出为准，本文只描述模型，不复刻判据细节。

## 布局

| 路径 | 角色 | 入仓 |
|---|---|---|
| `backlog/tasks.jsonl` | 唯一活账本，一行一任务 | ✅ |
| `backlog/archive/tasks-<YYYY>.jsonl` | 闭环滚动归档（`resolve` 幂等追加） | ✅ |
| `backlog/.next-id` | ID 高水位（下一候选编号，单调不减） | ✅ |
| `backlog/.locks/<ID>.lock` | 任务级租约（含 pid/会话/心跳，**本地运行时**） | ❌ 已 gitignore |
| `backlog/legacy/` | 冻结历史（旧 markdown 看板），只读不参与校验 | ✅ |

## 记录模型

- 必填：`id`（`B\d{3,}`）/ `category`（`Defect|Task|Debt`）/ `priority`（`P0..P3`）/
  `status`（`todo|in_progress|done`）/ `title` / `created_at` / `metadata.fingerprint`。
- 条件必填：`in_progress` → `owner` + `touched_paths`（≥1）；`done` → `evidence` + `closed_at`。
- 未知字段一律拒绝；`description` ≤2000 字符；`metadata.triage_context` ≤8KiB。

## 状态机

`todo → in_progress → done`，允许 `in_progress → todo`（`release`）。**`done` 只能由 `resolve` 写入**
（写入证据并归档；从 `todo` 直接闭环也合法）；活跃池残留 `done` 由 `lint` 判违规。
手改 JSONL 的状态跳变不被 CLI 拦截，但「记录非 `in_progress` 却仍持锁」会被 `lint`/`doctor`
判违规（手改降级 / `start` 中断都走这一条）。会话中断后用 `owner` / `touched_paths` /
`description` 与 `git diff --stat` 交叉确认真实进度——不维护任何 plan 文件。

## 租约（并发控制）

- `start` 以 `O_EXCL` 原子获取 `<ID>.lock`：同一任务同时只允许一个会话持有；`heartbeat` 刷新；
  `release` 显式释放；`reap --id <ID> --minutes N` 回收指定任务的超龄租约（不带 `--id` 会清全部租约，
  并发 lane 下慎用）。
- `.locks/` 是**本地运行时状态**，不入仓：干净克隆/CI 上缺锁只提示（`NOTE`），不阻断；只有在
  「本工作区确实存在 `.locks/`」时才把缺锁判为手改状态的强信号（见 B069 决策）。
- 缺锁判定的前提是记录带**新鲜的** `heartbeat_at`（< TTL）；缺 `heartbeat_at` 时静默跳过
  （已知 fail-open，记于 `backlog:B107`）。反向组合「锁在、记录不是 `in_progress`」一律判违规。
- 租约只保护「同一任务」，不保护账本追加：并发 `record` 仍可能丢行，靠「record 后立即提交 +
  冲突人工合并」兜底。

## 命令

```bash
node scripts/backlog.mjs list                 # 看板（状态/优先级排序）
node scripts/backlog.mjs record --category Defect --title "..." --fingerprint "core::x::y" --priority P1
node scripts/backlog.mjs start B106 --owner lane-1 --paths "a/b.rs,c/d.rs"
node scripts/backlog.mjs resolve B106 --evidence "npm test 全绿"
node scripts/backlog.mjs lint                 # 记录不变量（非零 = 违规）
node scripts/backlog.mjs doctor               # 跨归档/高水位/孤儿租约对账
```

## 门禁与验收

- 门禁：`bash .meta/gates/backlog-gate.sh`（= `lint` + `doctor`），已接 pre-commit、CI 与
  `npm run verify`；负样本自检：`bash .meta/gates/backlog-gate.spec.sh`；登记见
  `.meta/gates/INSTANCE-GATES.md`。
- 本地钩子前提：`git config core.hooksPath .githooks`。git **不会克隆**该配置，新 clone 需跑一次
  `npm run hooks:install`（ponygo init 已装配的仓库无需重复）；未装配时本地门禁静默失效，仅 CI 兜底。
- 本契约的变更属非平凡改动：改判据要同步改 CLI + 本文 + spec，并按 `.agents/notes/` 写 ADR。
- 编码约定：提交信息/文档引用账本任务写 `backlog:B106`；裸 `B106` 视为 legacy 历史编号。
