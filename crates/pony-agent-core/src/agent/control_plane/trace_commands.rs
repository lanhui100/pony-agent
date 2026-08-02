// trace_commands: 前端 trace 落库与导出命令。
// 由 control_plane/mod.rs 的 HostControlPlane 巨型 impl 拆分而来，行为与结构保持一致。
use super::*;

impl HostControlPlane {
    pub fn append_frontend_trace_events(
        &self,
        command: FrontendTraceAppendCommand,
    ) -> Result<(), String> {
        self.frontend_diagnostics.append(command)
    }

    pub fn query_frontend_trace_window(
        &self,
        query: FrontendTraceQuery,
    ) -> Result<FrontendTraceQueryResult, String> {
        self.frontend_diagnostics.query_window(query)
    }

    pub fn query_frontend_stall_snapshots(
        &self,
        query: FrontendTraceQuery,
    ) -> Result<Vec<FrontendStallSnapshot>, String> {
        self.frontend_diagnostics.query_stall_snapshots(query)
    }

    pub fn clear_frontend_trace_before(&self, ts_wall_ms: i64) -> Result<(), String> {
        self.frontend_diagnostics.clear_before(ts_wall_ms)
    }

    pub fn export_frontend_trace_json(
        &self,
        query: FrontendTraceQuery,
    ) -> Result<FrontendTraceExportPayload, String> {
        self.frontend_diagnostics.export_json(query)
    }

    pub fn export_frontend_trace_chrome_trace(
        &self,
        query: FrontendTraceQuery,
    ) -> Result<FrontendTraceExportPayload, String> {
        self.frontend_diagnostics.export_chrome_trace(query)
    }
}
