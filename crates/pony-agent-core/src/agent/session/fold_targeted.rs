//! B148：**单会话定向装载**的日志真源合并（`SessionBackend::load_session_targeted` 的读路径）。
//!
//! ## 为什么需要它
//!
//! 全量装载（`fold::merge_log_truth_into_sessions`）的语义是「日志 fold ∪ 索引 grandfather」，
//! 其中单会话的合并规则集中在私有函数 `apply_index_projection`。定向装载只服务**一个**
//! 会话，不能为了复用这段规则去扫全库日志；但也不能另写一份合并规则——那会让
//! 「全量装载」与「定向装载」对同一会话给出不同结果，正是 B148 的验收红线。
//!
//! 因此本模块只做三件事：定位该会话日志、复用 `fold::read_session_state` +
//! `fold::apply_index_projection`、把结果收敛成四态。合并规则本身仍只有一份（fold 内）。
//!
//! ## 四态语义
//!
//! - `Merged`：日志存在且可折叠（或日志不存在但有索引行）⇒ 返回该会话的完整状态；
//! - `Deleted`：日志 fold 出 `RemoveSession` 墓碑 ⇒ 视同不存在（D3）；
//! - `Absent`：日志与索引都没有该会话；
//! - `Failed`：日志不可读 / fold 失败（非墓碑）。调用方**必须**回落索引视图，不得丢会话。

use std::path::Path;

use super::fold::{apply_index_projection, read_session_state, FoldError};
use super::log_store::SessionLogStore;
use super::types::SessionState;

/// 单会话「日志真源 ∪ 索引」合并结果（见模块文档四态语义）。
#[derive(Clone, Debug)]
pub(crate) enum SingleSessionTruth {
    Merged(SessionState),
    Deleted,
    Absent,
    Failed(String),
}

/// 读单会话日志并按全量装载的同一规则与索引行合并。
///
/// `indexed` = 该会话的索引视图（legacy blob / normalized 投影；`None` = 索引里没有这一行）。
pub(crate) fn read_single_session_truth(
    log_root: &Path,
    session_id: &str,
    indexed: Option<SessionState>,
) -> SingleSessionTruth {
    let store = SessionLogStore::new(log_root);
    match read_session_state(&store, session_id) {
        Ok(folded) if folded.deleted => SingleSessionTruth::Deleted,
        Ok(folded) => {
            let state = match indexed {
                Some(indexed) => apply_index_projection(&indexed, folded.state),
                None => folded.state,
            };
            SingleSessionTruth::Merged(state)
        }
        Err(FoldError::SessionNotFound { .. }) => match indexed {
            Some(indexed) => SingleSessionTruth::Merged(indexed),
            None => SingleSessionTruth::Absent,
        },
        Err(error) => SingleSessionTruth::Failed(error.to_string()),
    }
}
