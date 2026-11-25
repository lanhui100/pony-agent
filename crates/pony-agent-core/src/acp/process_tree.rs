//! ACP agent 子进程树回收（B165）。
//!
//! 为什么需要这一层：`AcpTransport::spawn` 在 Unix 用 `process_group(0)` 建独立进程组，
//! `close()`/`Drop` 时对**整组**发 SIGKILL，因此 agent 派生的孙进程会被一并回收；Windows
//! 没有进程组，`Child::kill()` 只终止直接子进程，agent 派生的孙进程会残留到自然结束。
//!
//! 收口方式：Windows 上把子进程绑定到私有、不可继承、kill-on-close 的 Job Object
//! （复用 [`crate::agent::process::windows_job`]，不复制 Win32 调用），显式终止走
//! `TerminateJobObject`；句柄最终关闭时 kill-on-close 仍是兜底。
//!
//! 纪律：
//! - **spawn 即挂起**：Windows 上调用方以 `CREATE_SUSPENDED` 创建子进程，纳管成功后才恢复
//!   （与 `agent::process` 的 P2-1 同口径），收敛 `spawn → 纳管` 的逃逸窗口；
//! - **fail-closed**：纳管（创建/绑定/成员校验/恢复）任一步失败都不得"降级为仅杀直接子进程"
//!   静默放行——调用方据此回滚已 spawn 的子进程并报错；
//! - 非 Windows 上是**零字节占位**：进程组语义已在 `spawn` 时建立，调用点不需要 cfg 分支。

use std::process::Child;

#[cfg(windows)]
use crate::agent::process::windows_job::Job;

/// 一个已被纳管的子进程树。
///
/// Windows：持有 Job Object（`Some`）；其他平台：空结构（进程组语义在 `spawn` 侧）。
#[derive(Debug)]
pub(crate) struct ProcessTree {
    #[cfg(windows)]
    job: Option<Job>,
}

impl ProcessTree {
    /// 纳管 `child` 及其此后派生的整棵进程树。
    ///
    /// **前置契约（Windows）**：调用方必须以 `CREATE_SUSPENDED` 创建子进程——否则
    /// `spawn → 纳管` 之间是真实的逃逸窗口（子进程可能先派生一个不在 Job 内的孙进程）。
    /// `contain` 在「创建 Job + 绑定 + 成员校验」全部成功后恢复主线程；任一步失败都不恢复，
    /// 由调用方终止（挂起中的进程同样可被 `TerminateProcess` 终止）。
    ///
    /// 非 Windows 直接返回占位（进程组已覆盖）。
    #[cfg_attr(
        not(windows),
        expect(
            clippy::unnecessary_wraps,
            reason = "非 Windows 侧纳管不会失败；保留 Result 是为了让 spawn 失败路径不需要 cfg 分支"
        )
    )]
    pub(crate) fn contain(child: &Child) -> Result<Self, String> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            let handle = child.as_raw_handle();
            let job = Job::create_default()?;
            if let Err(error) = job.assign_and_verify(handle) {
                // 绑定失败：关闭 Job（kill-on-close）避免留下"半纳管"句柄，由调用方回收子进程。
                job.close();
                return Err(error);
            }
            // 纳管成功后才恢复执行；恢复失败必须 fail-closed（绝不留一个永不推进的进程）。
            if let Err(error) = crate::agent::process::resume_process(handle) {
                // 与 `agent::process` 的启动失败清理同口径：先显式终止整棵树，再关句柄
                // （kill-on-close 只作兜底，不作为唯一回收手段）。
                let _ = job.terminate(1);
                job.close();
                return Err(error);
            }
            Ok(Self { job: Some(job) })
        }
        #[cfg(not(windows))]
        {
            let _ = child;
            Ok(Self {})
        }
    }

    /// 终止整棵进程树（幂等）。非 Windows 无操作：由 `close()` 的进程组 SIGKILL 承担。
    #[cfg_attr(
        not(windows),
        expect(
            clippy::unused_self,
            reason = "非 Windows 侧没有 Job 句柄可读；保留 &self 是为了让调用点不需要 cfg 分支"
        )
    )]
    pub(crate) fn terminate(&self) {
        #[cfg(windows)]
        if let Some(job) = self.job.as_ref() {
            // 退出码 1：与 `agent::process` 的失败收口口径一致（非 0 = 非正常退出）。
            let _ = job.terminate(1);
        }
    }

    /// 释放纳管句柄（幂等）。Windows 上关闭 Job；kill-on-close 兜底覆盖"直接子进程已退出、
    /// 孙进程仍在"的路径。
    #[cfg_attr(
        not(windows),
        expect(
            clippy::unused_self,
            reason = "非 Windows 侧没有 Job 句柄可释放；保留 &mut self 是为了让调用点不需要 cfg 分支"
        )
    )]
    pub(crate) fn close(&mut self) {
        #[cfg(windows)]
        if let Some(job) = self.job.take() {
            job.close();
        }
    }
}

#[cfg(all(windows, test))]
mod tests {
    //! 只在 Windows 上编译（Linux/macOS 走进程组语义，没有 Job 可断言）：
    //! 真实子进程纳管后成员校验通过，且 `terminate` 能让它提前结束而不是跑满自然时长。
    use super::*;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    #[test]
    fn contain_verifies_membership_and_terminate_ends_child_early() {
        use std::os::windows::process::CommandExt;
        // 自然时长 ≈ 30s；若没有真正纳管/终止/恢复，本用例会明显超时。
        let mut child = Command::new("cmd");
        child
            .args(["/C", "ping -n 30 127.0.0.1 > NUL"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // 与生产同前置契约：先挂起，纳管成功（含 resume）后才开始执行。
            .creation_flags(windows_sys::Win32::System::Threading::CREATE_SUSPENDED);
        let mut child = child.spawn().expect("spawn 真实子进程");
        let tree = ProcessTree::contain(&child).expect("纳管必须成功（创建+绑定+成员校验）");
        assert!(tree.job.is_some(), "Windows 上必须持有 Job 句柄");

        let started = Instant::now();
        tree.terminate();
        let _ = child.wait();
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "终止必须让子进程提前结束（自然时长 ≈30s），实际 {:?}",
            started.elapsed()
        );

        let mut tree = tree;
        tree.close();
    }
}
