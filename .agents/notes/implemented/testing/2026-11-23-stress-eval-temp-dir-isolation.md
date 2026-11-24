# Agent Note: 压力测试的临时目录改为每次唯一（消除「重跑就好」的偶发红）

Status: implemented

## Problem

`tests/acceptance/stress-eval.spec.ts` 在 #67 的首次 CI 里红过，rerun 即绿。它的问题不在断言，而在**夹具**：

```ts
const TEMP_DIR = path.join(process.cwd(), 'temp-eval-env');
if (fs.existsSync(TEMP_DIR)) fs.rmSync(TEMP_DIR, { recursive: true });
fs.mkdirSync(TEMP_DIR, { recursive: true });
```

用的是**固定路径**，且「先删后建」。后果：

1. 同一 checkout 上并发的两个 run（本地并行跑、或 CI 上同 runner 的另一个作业）会互相删掉对方的目录，
   进而删掉对方的 `loop-state.json` —— 表现就是某个 run 在 `await_approval` 分支读到残缺状态、
   或在第 N 步拿不到文件；
2. 上一次异常退出的残留会污染下一次（`rmSync` 先删，但如果那一刻另一进程正在写，删的时机就成了骰子）；
3. 用例串行执行 10 次 `python3 trace-audit-loop.py`，仍用 vitest 默认 5s 超时 —— CI 机器比本地慢一个量级时
   会直接超时（本地实测 353ms，不代表 CI）。

## Decision

1. **每次 run 用 `fs.mkdtempSync(path.join(os.tmpdir(), 'pony-stress-eval-'))` 拿唯一目录**，彻底去掉固定路径；
2. **`finally` 里清理自己的临时目录**（只删本次 mkdtemp 产出的路径，`force: true` 容忍已不存在）；
3. **显式给 120s timeout**（10 次子进程 + CI 机器慢一档的余量），不再依赖默认 5s；
4. 断言与驱动逻辑**逐字不动**（仍驱动 10 轮、仍断言 `phase === 'patrol'`）。

## Verification

- `npx vitest run tests/acceptance/stress-eval.spec.ts` → 1 passed（**353ms**）
- **并发两次同时跑同一文件**（旧写法必互踩）：两个 run 均 `1 passed`，无失败
- 清理生效：跑完后 `os.tmpdir()` 下无残留 `pony-stress-eval-*` 目录

## Alternatives considered

- **只把 timeout 调大**：不解决两个 run 互删目录的问题；超时只是若干失败模式中的一种。
- **靠 CI rerun 过关**：#67 已经这样过了一次——这正是"隐债"的定义：测试自己不可信，红绿取决于运气。
- **把该用例挪到独立 CI job 独占机器**：能降低碰撞概率，但要新增 job、拉长流水线，且仍不解决"固定路径 + 并发"
  这个根因；先改夹具，成本最低。
- **删掉该用例**：它覆盖的是 trace-audit-loop 状态机的多轮守恒性，删掉等于放弃这条唯一的长链路守卫。
