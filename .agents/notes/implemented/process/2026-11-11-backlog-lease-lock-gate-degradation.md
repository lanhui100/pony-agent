# Agent Note: 任务租约锁不参与 governance 门禁（B069：干净克隆恒红的判据收窄）

Status: implemented

## Problem

`node scripts/backlog.mjs lint` 在干净克隆 / CI 上对**任何** `in_progress` 任务恒判违规：

```
[backlog-lint] tasks.jsonl (B060) 记录为 in_progress 但缺少租约锁文件（疑似手动改状态）
```

根因是判据建在一个不可入仓的载体上。租约文件 `backlog/.locks/<ID>.lock` 的内容是
`{task_id, owner, session_id, pid, acquired_at, heartbeat_at, ttl_ms}`——pid 与会话标识
按定义就是**本机运行时**事实：入仓会让每次 `start`/`heartbeat` 都产生一次 diff，并在
多会话下产生必然的合并冲突。它既未跟踪、也未被 ignore（实测 `git check-ignore` 无命中），
于是干净克隆里必然不存在，而 `lint` 只检查「锁文件是否存在」⇒ 只要账本里有
`in_progress` 任务（多会话并行的常态），该门禁在任何干净环境都不可能绿。

现场复现（`PONY_BACKLOG_ROOT` 指向台账副本，`start` 后删除 `.locks`）：

```
$ PONY_BACKLOG_ROOT=... node scripts/backlog.mjs lint
[backlog-lint] tasks.jsonl (B069) 记录为 in_progress 但缺少租约锁文件（疑似手动改状态）
[backlog-lint] FAIL（1 项）   # exit 1
```

附带缺陷：`doctor()` 内复制粘贴了同一段校验，却把结果 `push` 进**未定义**的 `problems`
（应为 `report.issues`）——一旦账本有 `in_progress` 任务，`doctor` 直接抛 `ReferenceError`。

## Decision

1. **租约是本地运行时状态，不入仓**；`backlog/.locks/` 进 `.gitignore`。
2. **判据按「本工作区是否真的在跟踪租约」分流**（`isLeaseTrackingActive`：目录是否存在）：
   - `.locks/` **存在** → 缺锁文件仍判 `problem`（本地确有租约设施在用，缺锁即手改状态的强信号）；
   - `.locks/` **不存在** → 降级为 `note`（只提示、不阻断）。干净克隆/CI 走这一支，门禁可绿。
3. **owner 冲突仍是硬错**：两侧数据都在手（记录 + 锁），可直接比对，不受降级影响。
4. **单一实现**：抽出 `leaseConsistency(paths, rows) -> { problems, notes }`，`lintAll` 与
   `doctor` 共用；`doctor` 删除重复块（顺带修掉 `ReferenceError`），并把 `notes` 透出到报告。
5. CLI `lint` 把 `notes` 打到 stderr（`[backlog-lint] NOTE ...`），stdout 的 PASS/FAIL 语义不变。

## Alternatives considered

- **把 `.locks/` 入仓**：否决——内容含 pid/会话/心跳时间戳，每次 `start`/`heartbeat` 都产生 diff，
  多会话并行时必冲突；且「他人工作区的 pid」对本机毫无意义，入仓只是把本地事实伪装成共享事实。
- **`lint` 完全不管租约（只由 `doctor` 负责）**：否决——本地误用（忘 `start` 直接改 JSONL）会失去
  唯一自动发现点；保留「有租约目录则严格」这一支，成本为零且保住本地防护。
- **用 `git ls-files backlog/.locks` 判「是否干净克隆」**：否决——干净克隆与「本地从未 start 过」在
  文件系统上同形，git 视角也一样；真正可观测的差异只有「本工作区是否建立过租约目录」。
- **在缺锁时要求显式 `--allow-missing-lease` 开关**：否决——把「干净环境常态」变成需要人工放行的
  异常，等于给 CI 增加一个每克隆都要踩的开关；判据应内建在可观测事实里。
- **基线式白名单（把当前 in_progress 任务写进豁免清单）**：否决——白名单随任务流动而腐化，
  且解释不了「下一个克隆里新任务为何又红」，是治标。

## Consequences

- **门禁语义变更**：`lint` 在无租约目录的工作区不再校验锁文件存在性（该信号在此环境下不可判别）；
  代价是「本地从未 start 过任何任务时手改状态」漏检——由 `doctor` 的 `notes` 提示，且属于低频路径。
- **测试改动**：原用例「in_progress 但缺少租约锁文件判违规」拆为两例——「有 `.locks/` 时判违规」
  与「干净克隆只提示不阻断」；新增「`doctor` 不崩溃且复用同一判据」。`tests/backlog-cli.spec.ts` 26 例全绿。
- **机器收据**（本机实测）：
  - `npx vitest run tests/backlog-cli.spec.ts` → 26 passed
  - 干净克隆场景 `lint` → `PASS`，stderr 一条 `NOTE ... 无本地租约目录`，exit 0
  - 建空 `.locks/` 后同一账本 `lint` → `FAIL（1 项）`，exit 1（严格支仍在）
  - `doctor` 在含 `in_progress` 账本上不再抛异常
- **与 `docs/observability.yaml` 无冲突**：不改对外接口、网络 IO、后台任务、LLM 调用或 UI，
  属结构性 N-A（纯本地 CLI 门禁判据）。
- 后续若有第二个「不可入仓载体参与门禁」的诉求，按本条模式先问「该事实在干净环境可观测吗」，
  再决定硬错/提示，而不是直接 fail-closed。
