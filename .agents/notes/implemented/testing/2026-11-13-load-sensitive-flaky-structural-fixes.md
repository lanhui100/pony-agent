# Agent Note: lib 单测负载敏感 flaky 的结构性根治（parallel_tools / run_command / web deadline）

Status: implemented

## Problem

`cargo test -p pony-agent-core --lib` 全量并行时偶红（单独 `--exact` 复跑恒绿），集中在三类：

1. **parallel_tools 并发探针**：用例用固定 `sleep(60ms)` 制造重叠窗口，第二路线程在负载下没被
   调度进窗口，实测 `peak` 停在 1，断言「并发确实发生」偶发假失败；serial 负控同样依赖固定窗口。
2. **run_command promote 开关**：用例在存续期用进程级 `std::env::set_var` 改写
   `PONY_AGENT_RUN_PROMOTE_ON_TIMEOUT`，同二进制并行跑的其它 Run 用例会读到被改写/随后删除的值，
   promote 语义随调度漂移；此外后台作业 marker 的等待上界直接取作业生命周期（5s），等于把
   「spawn + 登记 + 首次读输出必须在 5s 内跑完」写成隐含前提。
3. **web deadline 时序断言**：断言把「测试准备阶段必须在固定 slack 内跑完」当前提
   （`budget + 1000ms`、`elapsed < deadline` 的固定比较），负载下被调度抖动击穿。

共同点：断言的不是**行为不变量**，而是「机器在给定时间内完成了多少事」。这类假设在并行高负载下
必然偶发失效，且失败方向与代码正确性无关。

## Decision

按「把时间假设换成结构性不变量或条件等待」逐类根治，不抬超时、不加重试、不允许 ignore：

1. **parallel_tools**：固定 sleep 换成**有界集合点**——探针进入后登记 `active/peak`，然后在
   `Condvar` 上等「峰值达到期望值」或预算耗尽（10s）。并发成立时集合点毫秒级返回，串行执行时
   窗口耗尽后照常返回且 `peak` 保持 1；serial 负控用 200ms 有界观测窗口取证「第二路未出现」。
2. **run_command**：promote 开关增加 `#[cfg(test)] thread_local` 覆写槽
   （`set_promote_on_timeout_override`），用例只在**本线程**覆写，语义不变、影响面收敛；
   后台作业 marker 等待改为条件等待 + 独立失败上界（`B012_JOB_LIFETIME_SECS × 6`），
   断言口径（owner 必须读到 marker）不变。
3. **web deadline**：注入式 sender 记录每次调用的**采样时刻** `(url, timeout_ms, Instant)`，
   时序断言以采样时刻的剩余预算为基准自校准；链路预算断言改用
   「跳数 × 每跳延迟」的结构性上界（跑完整条链才可能达到），不再用 `budget + 固定 1s`。

## Alternatives considered

- **A. 抬高超时 / 加 sleep 兜底**：落选。只是把偶红概率换成更长的墙钟耗时，根因（时间假设）仍在，
  且会掩盖真实的并发回归。
- **B. 给 flaky 用例加 `#[ignore]` 或重试宏**：落选。门禁失去意义，回归不再可见；与 B137 的
  「单独运行恒绿 + 全量并行偶红 = 负载敏感」判定口径相悖。
- **C. 单线程跑全量 lib（`--test-threads=1`）**：落选。把并发缺陷藏起来，且全量耗时线性放大，
  不能作为长期门禁。
- **D. 结构性根治（采纳）**：有界集合点 / thread-local 覆写 / 采样时刻自校准，断言只依赖
  「并发是否发生」「开关是否生效」「链路是否跑完」等不变量。

## Consequences

- 用例墙钟耗时与负载解耦：并发成立时立即继续，串行时由有界预算兜底（10s / 200ms），
  最坏情况仍远低于原来的固定 sleep 累积。
- `thread_local` 覆写只在测试构建存在，生产路径零开销；Run 在调用线程同步执行是该覆写成立的前提
  （注释已写明）。
- 仍需在**全量并行**下验证：定向用例恒绿不等于偶红消失，验收以「连续多轮全量 lib 跑」为准。
- 已知残留（与本条同源、另行跟踪）：`agent::otel::log::tests::concurrent_writes_produce_parseable_single_line_json`
  与 `agent::control_plane::tests::append_turn_materializes_history_from_events_over_caller_text`
  的进程级全局态竞争，本次实测单跑恒绿、全量并行偶红，按 B137 口径记录而未在本条改动。
