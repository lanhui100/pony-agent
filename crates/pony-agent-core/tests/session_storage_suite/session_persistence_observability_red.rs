//! 红相验收：会话库持久化的「可观测性 + 原子性」契约（T2-3）。
//!
//! 缺陷面：
//! 1. `SessionBackend::save_store` 没有返回值（`session/backend.rs`），文件后端
//!    `FileSessionBackend::save_store`（`session/file_backend.rs`）里 `create_dir_all` 失败、
//!    序列化失败、`fs::write` 失败一律 `return` / `let _ =` 静默吞掉 —— 磁盘写不进去时上层
//!    完全不知道，会话丢失被当成“保存成功”。
//! 2. `fs::write` 就地截断写目标文件：不是 临时文件 + rename 的原子替换，进程在写一半被
//!    杀掉/断电会留下半截 JSON，整个会话库被破坏。
//!
//! 断言只锚定两个**可观察性质**，不锁定具体函数签名：
//!   (a) 写失败必须可被调用方观测；
//!   (b) 写盘必须原子（不得沿目标路径就地写）。
//! 若实现选择用侧信道（错误回调 / 错误收集器）而不是返回值上报失败，请在 `SaveReport`
//! 的 impl 处补上该侧信道读取；断言语义不变。

use pony_agent_core::agent::session::{FileSessionBackend, PersistedStore, SessionBackend};
use std::path::{Path, PathBuf};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let unique = format!(
            "pony-t2-{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before epoch")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write(&self, name: &str, content: &str) -> PathBuf {
        let file = self.path.join(name);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(&file, content).expect("write temp file");
        file
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// 归一化 `save_store` 的“失败上报形状”，从而不锁定实现签名：
///   - 保留 `()`：没有失败通道 ⇒ 任何失败都不可能被观测 ⇒ 视为“未上报”
///   - 改回 `Result<(), E>`：`Err` 即“已上报”
/// 其它形状（如 `Option<String>` / 结构化结果）按同样语义补 impl 即可。
trait SaveReport {
    fn failure_reported(&self) -> bool;
}

impl SaveReport for () {
    fn failure_reported(&self) -> bool {
        false
    }
}

impl<E: std::fmt::Debug> SaveReport for Result<(), E> {
    fn failure_reported(&self) -> bool {
        self.is_err()
    }
}

/// 正向钉子：正常路径必须真的落盘（保证下面两条用例不是“反正什么也没写”的空转）。
#[test]
fn save_store_on_a_writable_path_round_trips() {
    let dir = TempDir::new("persist-ok");
    let store_path = dir.path().join("sessions.json");
    let backend = FileSessionBackend::new(store_path.clone());

    let _ = backend.save_store(&PersistedStore::default());

    assert!(
        store_path.is_file(),
        "T2-3：可写路径下 save_store 必须真的产出会话库文件 {}",
        store_path.display()
    );
    assert!(
        backend.load_store().is_some(),
        "T2-3：刚保存的会话库必须能被 load_store 读回（否则持久化链路整体失效，后续断言无意义）"
    );
}

/// (a) 写失败必须可观测。
/// 让目标路径的父目录被一个**普通文件**占住：`create_dir_all` / `fs::write` 必然失败。
/// 现状：失败被静默 return，调用方拿到“一切正常”。
#[test]
fn save_store_failure_must_be_observable_by_the_caller() {
    let dir = TempDir::new("persist-failure");
    let blocker = dir.write("blocked", "this is a regular file, not a directory");
    let store_path = blocker.join("sessions.json");
    let backend = FileSessionBackend::new(store_path.clone());

    let outcome = backend.save_store(&PersistedStore::default());
    let reported = SaveReport::failure_reported(&outcome);

    assert!(
        !store_path.is_file(),
        "T2-3：父路径不可用时写盘必然失败，目标文件不应存在（用例前置条件被破坏：{}）",
        store_path.display()
    );
    assert!(
        reported,
        "T2-3：写盘失败必须可观测（返回错误 / 显式上报），现状静默吞掉失败，上层会把“会话丢失”当成保存成功"
    );
}

/// (b) 写盘必须原子：临时文件 + rename 替换，不得沿目标路径就地截断写入。
///
/// 观测手法：把目标路径做成指向哨兵文件（sentinel）的符号链接。
///   - 就地写（`fs::write`）会**跟随**符号链接，把哨兵文件内容原地截断成 JSON（现状 = 红）；
///   - 原子替换（写临时文件后 rename 到目标路径）会替换掉链接本身，哨兵内容保持不变（绿）。
#[cfg(unix)]
#[test]
fn save_store_must_replace_the_target_atomically_instead_of_writing_through_it() {
    let dir = TempDir::new("persist-atomic");
    let sentinel = dir.write("sentinel.json", "SENTINEL-MUST-STAY-UNTOUCHED");
    let store_path = dir.path().join("sessions.json");
    std::os::unix::fs::symlink(&sentinel, &store_path).expect("create symlink");

    let backend = FileSessionBackend::new(store_path.clone());
    let _ = backend.save_store(&PersistedStore::default());

    let sentinel_content = std::fs::read_to_string(&sentinel).expect("read sentinel");
    assert_eq!(
        sentinel_content, "SENTINEL-MUST-STAY-UNTOUCHED",
        "T2-3：写盘必须原子（临时文件 + rename）；现状就地写沿符号链接截断了另一份文件，\
         半截写入会直接损坏会话库。目标路径 {}",
        store_path.display()
    );
}
