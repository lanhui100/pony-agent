# Agent Note: ACP 入站分发线程的关闭语义——`close()` 必须 join 三组工作线程

Status: implemented

## Problem

`AcpTransport`（`crates/pony-acp/src/transport.rs`）在 `spawn()` 里起了三组线程：stdout 读线程
（`acp-stdout`）、stderr 排空线程（`acp-stderr`）、入站请求分发 worker 池
（`acp-inbound-<n>`，见 `ACP_INBOUND_WORKERS`）。三组 `JoinHandle` 都被**直接丢弃**，
`close()` 只做「关 stdin → 宽限 → 终止进程树 → `kill()`/`wait()`」。

后果（B177 ADVISORY ②）：

- 「子进程已死」不等于「线程已退出」。分发 worker 可能还在 `respond_to_inbound_request` 里
  跑一个长 handler（`terminal/wait_for_exit` 的整轮预算可达 120s），于是**关闭返回之后**
  仍在动 transport 的 Arc 资源、仍往已关闭的 stdin 写应答、仍往 `stderr_tail` 记诊断；
- 「关闭完成」因此不是一个可断言的状态：调用方无法确定资源何时真正回收，
  测试也无法证明「没有线程泄漏」——只能靠 sleep 观察，属于典型的不可观测契约；
- worker 池把这个问题放大了（4 个线程而不是 1 个），但根因是**句柄被丢弃**，
  与池大小无关。

## Decision

1. **三组 `JoinHandle` 随结构体持有**：`reader: Option<JoinHandle<()>>`、
   `stderr_reader: Option<JoinHandle<()>>`、`inbound_workers: Vec<JoinHandle<()>>`。
2. **`close()` 在终止进程树之后统一 join**（`join_worker_threads`），顺序是语义必需而非风格：
   - 先 join stdout 读线程——它持有 `request_sender` 的唯一克隆；进程死后 stdout EOF，
     读线程退出并 drop 该 sender；
   - 再 join 入站 worker——它们的循环只在 `receiver.recv()` 返回 `Err` 时退出，而 `Err`
     要求**所有** sender 都已 drop（结构体侧的 `request_sender` 在 spawn 时已被 move 进
     读线程，故只需等第 1 步）；
   - 最后 join stderr 排空线程——同样靠 EOF 自然退出。
3. **启动失败路径同样要收尾**：worker 起不来 / stderr 线程起不来时，先 `drop(request_sender)`
   再 join 已起的 worker。顺序反过来会**永久阻塞**——worker 的 `recv()` 还握着活的 sender。
4. **任一线程 panic 不传播**：关闭路径的职责是回收，不是重新抛出别人的 panic（`let _ = join()`）。
5. **`close()` 保持幂等**：`terminated` 位仍是唯一闸门；重复调用直接返回。

## Consequences

- `close()` 返回 ⇒ 三组线程都已退出，资源回收时机变成可断言的确定性状态。
- 新增守卫（`transport::tests`，Linux 走 `/proc/self/task/*/comm` 数线程名）：
  - `close_joins_inbound_worker_threads`：spawn 后 `acp-inbound-*` 恰为基线 +
    `ACP_INBOUND_WORKERS`，`close()` 后回到基线。**判据是线程数这个确定性事实，不靠 sleep 抢跑**；
  - `close_is_idempotent_after_join`：`close()` 两次 + `Drop` 再调一次不 panic。
- **判据实现必须轮询而非瞬时取样**（2026-11-28 CI 实测修正）：`std::thread::Builder::name()`
  的线程名由**新线程自身**在启动后写入 `comm`，`spawn()` 返回瞬间它可能还叫父线程的名字——
  GitHub runner 上首跑实测 3/4 而红。现用 `wait_for_thread_count` 轮询等待「达到 N」与
  「回落到基线」这两个**必然发生的正向事件**（上限 5s，超时才判红），既消掉竞态又不放宽判据。
- 代价：`close()` 的最坏耗时从「宽限期 + kill」变成「宽限期 + kill + 最长 handler 剩余时间」。
  这是**正确的**方向——旧行为只是把这段时间藏起来，让调用方误以为已经关完。

## Alternatives considered

- **只 join stdout/stderr，不 join worker**：worker 才是「活到 handler 返回」的那一组
  （ADVISORY ② 的原话），漏掉它等于没修。
- **给 worker 设 join 超时（到点就放弃）**：Rust std 没有 `join` 超时；自建「判活 + 放弃」
  会重新引入「关闭完成后还有线程在跑」的不可观测状态，与本决策要消除的东西同构。
- **用 detached 线程 + `Arc` 弱引用兜底**：线程仍在跑，只是错误被吞掉；资源回收时机依旧不可断言，
  且弱引用升级失败的分支会变成新的静默失败点。
- **`close()` 里先 drop receiver 再 join**：receiver 是 `Arc<Mutex<Receiver>>`，drop 结构体侧
  的 Arc 不能让已持锁 worker 的 `recv()` 返回 `Err`——真正的解除条件是**sender 全 drop**，
  故必须按 1→2→3 的顺序等。

## Verification

- `bash scripts/cargo-lane.sh test -p pony-acp --lib transport::` → 2 passed（连跑 5 次全绿，确认不 flaky）
- `bash scripts/cargo-lane.sh test -p pony-agent-core --test acp_suite` → 144 passed / 0 failed / 0 ignored
- `bash scripts/cargo-lane.sh check -p pony-acp --all-targets` → 通过

## Reintroduction conditions

不适用（本条不删除任何对外能力，只补线程生命周期收口）。
