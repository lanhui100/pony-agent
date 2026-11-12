# Agent Note: 长驻会话的内存与 IO 卫生（mimalloc + 日志轮转 + WAL 上限）

Status: implemented

## Problem

2026-11-12 dev 主机（16 核 / 30GB）发生整机级资源耗尽：

- 4 个 `pony resume` TUI 会话合计 RSS 17.3GB + Swap 26.3GB，占满 30GB 内存与 32GB swap；
- load average 131.9（16 核），CPU `%sy` 38.7%，kswapd0 常驻 79% CPU；
- 单进程 VmHWM 8.5GB，而对应主会话 transcript 仅 19–215MB，内存放大近 300 倍；
- 内存形态为 628 个匿名 VMA、大量整段 128MB 全驻留，`Pss_Anon ≈ Private_Dirty ≈ 100% RSS`；
- 45 秒采样 RSS 不增长 ⇒ 不是运行期泄漏，而是**峰值后不归还**。

根因链条：

1. 进程没有 `#[global_allocator]`，走 glibc malloc。27–45 个线程各自 arena，arena 以 mmap 大块
   增长，跨线程 free 的块无法合并归还 OS；glibc 只 trim arena 顶部。
2. TUI 把 stderr 全量 dup2 到 `~/.pony-agent/tui-logs/tui-runtime.log`，无轮转，实测 552MB。
3. SQLite WAL 依赖 `wal_checkpoint(TRUNCATE)` 回收，多进程并发读时必然 busy 而退化 PASSIVE，
   实测 `sessions.db-wal` 积到 1.5GB（库本体 1.2GB）。

## Decision

1. **全局分配器改用 mimalloc**（`crates/pony-agent-core/src/lib.rs`）：

   ```rust
   #[global_allocator]
   static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
   ```

   放在 lib 而非各 bin：`pony` / `ponycli` / `src-tauri` 都链接该 lib，一处生效。

2. **TUI 运行日志 64MB 轮转**：`run_tui` 在 `SaveFailureWitness::capture` 之前调用
   `rotate_runtime_log_if_oversized`，超限把 `tui-runtime.log` 改名为 `.log.1` 后由后续
   `OpenOptions::append` 重建。轮转早于 witness 起点记录，保证旧文件的失败痕迹不算进本次结论。

3. **WAL 加 `PRAGMA journal_size_limit=67108864`**：与既有 `wal_autocheckpoint=256` 同批设置，
   让每次 checkpoint 后自动截断 `-wal`，不依赖 `TRUNCATE` 是否抢到锁。

## Alternatives considered

- **仅设 `MALLOC_ARENA_MAX=2` 环境变量**：零代码改动，但依赖每个启动入口（herdr、CI、桌面端）
  都记得注入；漏一处就复发。代码级分配器让约束随二进制走。
- **jemalloc**：同为成熟方案，但构建复杂度与静态链接体积高于 mimalloc；mimalloc 对短生命周期
  多线程分配更友好，且 Windows（src-tauri）支持完整。
- **调低压缩水位（0.8 → 0.6）**：能削峰值，但本次明确不动产品语义；另开工单评估。
- **引入跨进程日志框架（tracing-appender 等）**：能正确处理多进程轮转，但为诊断日志引入新依赖
  与运维面不划算；单文件 + 一个归档已覆盖主要场景。
- **WAL 改为 busy 后重试 TRUNCATE**：仍受「是否有并发读事务」支配，长会话场景重试成功率低；
  `journal_size_limit` 是 SQLite 官方针对该形态的机制。

## Consequences

- 预期单会话 RSS 从 8.5GB 峰值降到 2–3GB，且 idle 后归还 OS；需在新二进制上实测确认。
- 日志最多占约 128MB（当前文件 + 一个归档）。
- WAL 上限 64MB；大事务仍需临时超过上限，但 checkpoint 后自动回落。
- **未覆盖**：subagent 并发无硬上限（backlog:B125）、图片解码无内存预算（backlog:B126）——
  两者是峰值放大的次要来源，另行收口。
- 分配器切换改变所有 crate 的堆行为，需跑完整 Rust 测试与 TUI 冒烟；`dhat`/`heaptrack`
  未接入，后续如需定位单点泄漏仍需外部工具。

## Verification

- `cargo check -p pony-agent-core --all-targets` → **EXIT=0**（mimalloc 0.1.52 + libmimalloc-sys 编译通过，5m24s）
- `cargo test -p pony-agent-core --bin pony io_hygiene -- --test-threads=1` → **2 passed / 0 failed**
  （未超限不动文件；超限移为 `.1`；重复轮转覆盖旧归档且不级联 `.1.1`）
- SQLite 回归：`cargo test -p pony-agent-core --lib sqlite_session -- --test-threads=1` → **49 passed / 0 failed**
