pub mod agent;

/// 全局分配器：替代 glibc malloc。
///
/// 长驻多线程进程（TUI 会话实测 27–45 线程）在 glibc 下会为每个线程分配独立 arena，
/// arena 以 mmap 大块增长，跨线程 free 的内存无法归还 OS；实测单进程膨胀到 8.5GB
/// （628 个匿名 VMA、大量整段 128MB），4 个会话耗尽 30GB 内存 + 32GB swap。
/// mimalloc 用线程本地堆并按阈值主动 purge，会话空闲后能把内存还给系统。
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
