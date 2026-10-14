// 负向对照夹具（negative control）：锁作用域已收敛，作用域内无二次加锁调用。
// 期望：check-lock-discipline.py 报 ok 并以 Exit Code 0 结束。
use std::sync::Mutex;

pub struct Demo {
    graph_runs: Mutex<u32>,
}

impl Demo {
    pub fn resolve_graph_run_submission_plan(&self, session_id: Option<&str>) -> Option<u32> {
        let loaded = {
            let store = self.graph_runs.lock().unwrap();
            store.checked_add(1)
        };
        let fallback = self.resolve_graph_run_for_retrieval(None, session_id);
        loaded.or(fallback)
    }

    pub fn resolve_graph_run_for_retrieval(
        &self,
        _run_id: Option<&str>,
        _session_id: Option<&str>,
    ) -> Option<u32> {
        let _guard = self.graph_runs.lock().unwrap();
        Some(0)
    }
}
