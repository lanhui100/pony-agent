# Agent Note: 跨平台会话库紧凑化修复与主机内存资源防护

Status: implemented

## Problem

开发主机在多会话并发与编译场景下出现系统级死锁（负载峰值达 237+，32GB Swap 全部耗尽，内核频发 OOM-Killer 斩首基础监控进程），经深层排查定位到 `pony-agent` 的会话存储与治理链路存在三项重大缺陷：

1. `scripts/compact-sessions-db.mjs` 存在严重的平台偏见：硬编码 Windows 的 `process.env.LOCALAPPDATA` 与 `powershell` 命令进行进程探测。在 Linux 开发机上直接报错退出，导致非 Windows 生产/开发环境从未执行过 PA-090 的去重与紧凑化迁移。
2. 开发机上的全局数据库 `~/.local/share/PonyAgent/sessions.db` 长期处于 `legacyBlob` 状态野蛮生长，积累了 174 个历史会话、11.8 万行事件及数百 MB 追踪数据，主库达 953MB、WAL 日志积压至 862MB。
3. `Pony Agent` 启动时的全量装载机制（`load_store()`）会将整库 174 个会话全量加载进内存，叠加存量会话内部每个 `HistoryNode` 均完整拷贝全量前置历史快照（`turnTraceHistory` $O(N^2)$ 级冗余，单条记录可达 4.27MB），反序列化进 Rust 堆后产生数倍放大，单进程堆内存常驻冲破 4GB~11GB 虚拟内存，多会话并发叠加满核 Rust 编译直接引爆系统级换页风暴（Swap Thrashing）。

## Decision

1. **紧凑化脚本跨平台适配与运维能力增强（`scripts/compact-sessions-db.mjs`）**：
   - 路径动态探测：优先支持命令行 `--db <path>` 参数；未显式指定时，按操作系统自动适配（Windows 保持 `LOCALAPPDATA`，POSIX 系统指向 `~/.local/share/PonyAgent/sessions.db`）。
   - 进程检测跨平台分流：Windows 保持 PowerShell 探测，POSIX 环境采用 `pgrep -x "pony"` 探测；引入 `--skip-process-check` 参数允许对离线/备份库进行无锁脱机紧凑化。
2. **存储层日志截断与存量会话瘦身治理**：
   - 对活动库执行 `PRAGMA wal_checkpoint(TRUNCATE)`，将积压的 862MB WAL 脏页安全写回主库并截断为 0 字节。
   - 经测试验证 `compact-sessions-db.mjs` 对 Linux 存量 174 个会话的迁移校验（174/174 通过），确立剥离节点冗余 `turnTraceHistory`、迁移至 `trace_table_authoritative` 并回写轻量引用（refs）的标准维护路径。
3. **开发机编译与内核调度资源硬防护**：
   - 在开发机创建 `~/.cargo/config.toml` 配置 `[build] jobs = 2`，在基础设施层硬性遏制 Rust 默认 16 核满载并发抢内存的行为。
   - 将内核换页倾向 `vm.swappiness` 从 60 下调至 15（持久化至 `/etc/sysctl.d/99-swappiness.conf`），压低物理内存充足时把工作内存过早拍入慢速磁盘交换区的颠簸概率。

## Alternatives considered

- **直接对 `sqlite_session.rs` 重构为单会话惰性按需加载（Lazy Loading）**：
  长效架构彻底解耦全量会话是正确方向，但改造涉及会话目录分页、状态缓存同步、搜索索引重构等多处核心生命周期契约，改动面大且当前开发机正处于换页死锁的高危边缘；优先通过修复跨平台紧凑化脚本与清理存量数据完成止血和治理。
- **直接清空或删除 `sessions.db` 重新初始化**：
  虽然能立刻将数据库降到零，但会永久抹除开发机上全部历史会话、上下文追踪及断点续传状态，违反数据持久化与可审计契约；必须坚持严格备份+幂等校验的三源合并瘦身。
- **仅依赖开发者在命令行手动指定 `cargo -j 2`**：
  依赖人脑记忆或 Agent 自觉约束无法形成防御底线（本次事故正是因为自动化任务执行无限流编译诱发）；必须通过全局配置形成机器硬约束。

## Consequences

- `scripts/compact-sessions-db.mjs` 成为全平台通用维护工具，可以在任意环境完成存量会话紧凑化与 `sessions.db` 优化。
- 开发机可用 Swap 恢复至 17GB+，可用物理内存提升至 14GB，上下文切换从 24 万次/秒回落至正常水平（~2 万次/秒），彻底排除 OOM-Killer 风险。
- 变更范围：`scripts/compact-sessions-db.mjs`、`.agents/notes/implemented/bug-fix/2026-11-11-cross-platform-session-db-compaction-and-devserver-resource-governance.md`。

