//! 后台 trace 持久化队列。
//!
//! 可观测性（trace）数据落库必须在后台完成，绝不允许阻塞主对话执行路径：
//! - 主对话线程只做内存更新 + 尽力入队（`try_enqueue`，队列满时丢弃并回退同步）；
//! - 后台 worker 线程负责获取写锁并执行 SQLite 写入。

use crate::agent::session::{SessionStore, SessionTraceMutation};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};

/// 一条待落库的 trace 变更。
pub struct TracePersistenceCommand {
    pub session_id: String,
    pub mutation: SessionTraceMutation,
}

/// 有界队列容量：超出后丢弃（可观测性数据允许丢失，绝不阻塞调用方）。
const TRACE_PERSISTENCE_QUEUE_CAPACITY: usize = 1024;

/// 主对话侧持有的发送端；Drop 时断开 channel，worker 自动退出。
pub struct TracePersistenceHandle {
    sender: SyncSender<TracePersistenceCommand>,
    _worker: Option<JoinHandle<()>>,
}

impl TracePersistenceHandle {
    /// 尽力入队；队列已满或 channel 已断开时返回 Err(原命令)，
    /// 由调用方决定回退同步落库或丢弃。
    pub fn try_enqueue(
        &self,
        command: TracePersistenceCommand,
    ) -> Result<(), TracePersistenceCommand> {
        match self.sender.try_send(command) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(command)) | Err(TrySendError::Disconnected(command)) => {
                Err(command)
            }
        }
    }
}

/// 启动后台 trace 持久化 worker。返回句柄（句柄持有 sender，drop 后 worker 退出）。
pub fn spawn_trace_persistence_worker(
    sessions: Arc<RwLock<SessionStore>>,
) -> TracePersistenceHandle {
    let (sender, receiver) = mpsc::sync_channel(TRACE_PERSISTENCE_QUEUE_CAPACITY);
    let worker = thread::Builder::new()
        .name("pony-trace-persist".to_string())
        .spawn(move || trace_persistence_worker_loop(sessions, receiver))
        .ok();
    TracePersistenceHandle {
        sender,
        _worker: worker,
    }
}

fn trace_persistence_worker_loop(
    sessions: Arc<RwLock<SessionStore>>,
    receiver: Receiver<TracePersistenceCommand>,
) {
    for command in receiver.iter() {
        let mut store = sessions.write().unwrap_or_else(|e| {
            eprintln!("[pony-agent][trace-persist] sessions rwlock poisoned: {e}, recovering");
            e.into_inner()
        });
        store.persist_trace_mutation_from_worker(&command.session_id, command.mutation);
    }
}
