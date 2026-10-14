// 正向对照夹具（positive control）：模拟 C5 违规形态。
// 期望：check-lock-discipline.py 报 REENTRANT_CALL 并以 Exit Code 1 结束。
use std::sync::Mutex;

pub struct Demo {
    graph_runs: Mutex<u32>,
}

impl Demo {
    pub fn resolve_graph_run_submission_plan(&self, session_id: Option<&str>) -> Option<u32> {
        let store = self.graph_runs.lock().unwrap();
        let loaded = store.checked_add(1).or_else(|| {
            self.resolve_graph_run_for_retrieval(None, session_id)
        });
        loaded
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
