---
name: backlog-onboarding
description: 何时用：把 backlog 账本体系接入一个仓库或 worktree（首次 init、从旧 md/taskctl 迁移、或核对既有接入是否完整）时——给出步骤、注意事项与机械校验，避免每仓各写一套。
---

# backlog-onboarding —— 在仓库/worktree 落地 backlog 账本

## 真相源

- `backlog/README.md` —— 账本契约（布局/字段/状态机/租约/归档/命令）；**判定真源 = `scripts/backlog.mjs`**
- `.meta/gates/INSTANCE-GATES.md` —— 实例门登记（含 `backlog-gate.sh` 的承诺→命令）
- `AGENTS.md` —— 入口地图与常载命约（若有）
- 目标仓的 `.gitignore`、`.githooks/pre-commit`、CI workflow

## 步骤

### 第 0 步：判定这是哪一类接入

| 场景 | 判定命令 | 结论 |
|---|---|---|
| 同一仓库的新 worktree | `git rev-parse --git-common-dir` 指向主仓 `.git` | **账本、hooks、契约天然共享**，只需做「校验」一节，不重复初始化 |
| 新仓库、全新接入 | 无 `backlog/`、无 `taskctl.py` | 走「新仓 init」 |
| 新仓库、旧体系（md + taskctl） | `ls backlog/backlog.md scripts/taskctl.py .task-state` 有命中 | 走「旧体系迁移」，迁移前不得删旧文件 |

### 新仓 init（一次性）

```bash
# 1) 数据骨架（新仓从 B001 起；注意 CLI 当前分配下限是 B056，见「注意事项 3」）
mkdir -p backlog/archive
: > backlog/tasks.jsonl
printf '1\n' > backlog/.next-id
printf 'backlog/.locks/\n' >> .gitignore        # 租约是本地运行时，不入仓

# 2) 契约指针（不要复制全文，避免与真源漂移）
cat > backlog/README.md <<'EOF'
# backlog/ —— 唯一活账本
判定真源 = scripts/backlog.mjs；契约与用法见团队基线 backlog/README.md。
EOF

# 3) CLI（单文件，随仓版本化；全局统一前的推荐做法）
cp <基线仓>/scripts/backlog.mjs scripts/backlog.mjs

# 4) 门禁脚本（与 CLI 同仓，CI 才能用）
mkdir -p .meta/gates
cp <基线仓>/.meta/gates/backlog-gate.sh .meta/gates/
cp <基线仓>/.meta/gates/backlog-gate.spec.sh .meta/gates/
chmod +x .meta/gates/backlog-gate.sh .meta/gates/backlog-gate.spec.sh

# 5) 本地钩子接线（git 不会克隆 core.hooksPath，新 clone 必须重跑）
git config core.hooksPath .githooks
# 已有 pre-commit 时：把 backlog-gate 调用插到哨兵块之外，失败即 exit 1

# 6) CI 接线（独立 step，不要改框架不可变门禁）
#   - run: bash .meta/gates/backlog-gate.sh
#   - run: bash .meta/gates/backlog-gate.spec.sh
```

### 旧体系迁移（每个仓一次）

1. **盘点**：`git ls-files backlog scripts/taskctl.py .task-state`；记录条数（活跃/已完成）。
2. **转换**：逐条生成 JSONL——`id` 重编（保留 `source_ref` 指向旧路径）、`done` 行回填 `evidence` + `closed_at`、`in_progress` 行补 `owner` + `touched_paths`。
3. **冻结**：旧 `backlog/backlog.md`、`backlog/current.md`、`.task-state/` 移入 `backlog/legacy/`，只读；删除 `scripts/taskctl.py` 前先 `grep -rn taskctl` 确认无消费者。
4. **验账**：`lint` + `doctor` 双绿、ID 无重复、高水位越过最大编号。
5. **留痕**：迁移本身写一条 ADR（旧载体、映射规则、丢字段取舍），并 `record` + `resolve` 一条执行项。

## 注意事项

1. **同仓 worktree 共享账本**：CLI 用 `git rev-parse --git-common-dir` 的父目录定位 `backlog/`，所有 worktree 读写同一份 `tasks.jsonl`；租约锁也落在主工作树。**不要**在子 worktree 重复 init 第二份账本。
2. **`.locks/` 不入仓**：干净克隆/CI 没有锁目录，缺锁只出 `NOTE` 不阻断；只有本工作区确有 `.locks/` 时，「锁在、记录不是 `in_progress`」才判违规。
3. **编号下限**：`scripts/backlog.mjs` 当前把候选编号钳在 56（pony-agent 迁移遗留）。别的仓复用前先确认：要么接受从 `B056` 起，要么把下限参数化并补负样本（已登记为体系待办）。
4. **账本只经 CLI 改**：手改 JSONL 的字段/状态跳变 CLI 不拦；能在提交前发现的是 lint 的记录不变量和 doctor 的租约-状态对账，别依赖门禁兜住语义错误。
5. **并发追加没有文件锁**：租约只保护「同一任务」，不保护账本追加；`record` 后立即提交，冲突人工合并。
6. **钩子前提**：`core.hooksPath` 不随 clone 传播；没装配时本地门禁静默失效（CI 仍兜底）。门禁脚本被删时 pre-commit 必须 fail-closed，不允许静默跳过。
7. **CI 里必须有 CLI**：gate 调 `node scripts/backlog.mjs`，所以 CLI 要随仓提交；「全局唯一 CLI」方案必须先解决 CI 取用问题。
8. **node 依赖**：无 `node` 的仓门禁 fail-closed；要么装 node，要么明确只靠 CI（CI runner 有 node）。

## 校验（机械收据，逐条贴 exit code）

```bash
node scripts/backlog.mjs lint      # 期望 exit 0，输出 PASS（活跃 N / 归档 M）
node scripts/backlog.mjs doctor    # 期望 exit 0，ok:true、issues:[]
bash .meta/gates/backlog-gate.sh          # 期望 exit 0，PASS（lint + doctor 对账通过）
bash .meta/gates/backlog-gate.spec.sh     # 期望 exit 0，P1 放行 + N1–N7 + H 接线
git config --get core.hooksPath           # 期望 .githooks（新 clone 后必须为真，否则钩子不跑）
```

**端到端**（临时 clone 里做，别弄脏真仓）：

```bash
git clone . /tmp/onboard-check && cd /tmp/onboard-check
npm run hooks:install 2>/dev/null || git config core.hooksPath .githooks
git commit --allow-empty -m control   # 期望 0：合法账本放行
# 造非法账本（缺 metadata.fingerprint），期望 commit 被拒
```

## 校准样例

- **正例**：新仓 init 后 `record` 一条 `todo` → `lint` 0；`start --owner a --paths x/y` → 0；`resolve --evidence "测试全绿"` → 归档 + 清锁，`lint`/`doctor` 0。
- **反例 1**：手改 `in_progress → todo` 但 `.locks/<ID>.lock` 仍在 → `lint` 1，文案含「存在租约锁但记录状态」；按提示 `reap --id <ID> --minutes 0` 后恢复 0。
- **反例 2**：`reap --id=B001`（未知拼写）→ exit 2 且不清锁；破坏性命令不接受文法外参数。
- **反例 3**：无 `.locks/` 的干净克隆里同一记录 → `lint` 0 且只有 `NOTE`（不得判违规）。

## 验证与报告

- 贴出上面「校验」四条命令的真实 exit code 与关键输出行；
- 迁移仓额外贴：迁移条数、`lint` 的「活跃/归档」计数、`doctor` 的 `highWater`；
- 任何一条非零先修再报，不得用「CI 会兜底」替代本地绿。
