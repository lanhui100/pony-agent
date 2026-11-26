pub use pony_tools::app_settings;
pub use pony_tools::ask_control;
pub use pony_storage::ask_persistence;
pub use pony_tools::attachment_import;
pub use pony_protocol::budget;
pub use pony_tools::capability_bridge;
pub use pony_tools::child_dispatch;
pub use pony_provider::compression;
pub use pony_provider::config;
pub mod context;
pub use pony_tools::system_prompt_template;
pub(crate) mod turn_prompt_content;
pub(crate) mod context_window;
pub mod control_plane;
pub use pony_protocol::cost;
pub use pony_tools::dispatcher;
pub use pony_tools::dispatcher_composites;
pub use pony_tools::document_conversion;
pub use pony_protocol::error_code;
pub use pony_protocol::event_bus;
pub use pony_tools::execution_control;
pub use pony_observability::frontend_diagnostics;
pub use pony_tools::git_search;
pub use pony_storage::goal;
pub use pony_storage::goal_command;
pub use pony_storage::goal_lease;
pub use pony_storage::goal_wrapup;
pub mod graph;
pub use pony_tools::governed_executor;
pub mod headless;
pub use pony_tools::hook_commands;
pub use pony_protocol::hooks;
pub use pony_tools::image_artifact;
pub use pony_protocol::input;
pub use pony_tools::instruction_touch;
pub mod jobs {
    pub use pony_tools::jobs::*;

    pub fn job_start(args: pony_tools::jobs::JobStartArgs) -> Result<pony_tools::jobs::JobStartResult, String> {
        crate::agent::ensure_runtime_bridges();
        pony_tools::jobs::job_start(args)
    }

    pub fn job_start_scoped(
        args: pony_tools::jobs::JobStartArgs,
        default_cwd: Option<&std::path::Path>,
        owner_session: Option<&str>,
    ) -> Result<pony_tools::jobs::JobStartResult, String> {
        crate::agent::ensure_runtime_bridges();
        pony_tools::jobs::job_start_scoped(args, default_cwd, owner_session)
    }
}
pub use pony_tools::lsp;
pub use pony_tools::parallel_tools;
pub use pony_tools::patch;
pub mod pipeline;
pub use pony_tools::mcp_client;
pub use pony_tools::mcp_config;
pub use pony_tools::mcp_resources;
pub use pony_protocol::metrics_snapshot;
pub use pony_protocol::model_preferences;
pub use pony_provider::model_window;
pub mod orchestration;
pub use pony_observability::otel;
pub mod planner;
pub use pony_tools::plugin_host;
pub use pony_tools::plan_state;
pub use pony_observability::ponysentry;
pub use pony_tools::path_permission;
pub use pony_protocol::present;
pub use pony_tools::process;
pub use pony_storage::projection;
pub use pony_provider::provider;
pub mod recovery;
pub use pony_provider::retry;
pub use pony_storage::retry_projection;
pub mod runtime;
pub use pony_protocol::runtime_helper;
pub use pony_tools::sandbox;
pub use pony_tools::search;
pub use pony_provider::secret_store;
pub use pony_storage::session;
pub use pony_tools::skill_body;
pub use pony_storage::sqlite_session;
pub use pony_protocol::spill;
pub mod subagent_runtime;
pub mod sse_adapter;
pub use pony_tools::stop_escalation;
pub use pony_protocol::stop_sequence;
pub use pony_protocol::telemetry;
pub mod teammate_identity;
pub use pony_tools::terminal;
pub use pony_protocol::todo;
pub use pony_tools::tool_runtime;
pub use pony_protocol::tool_pruner;
pub use pony_tools::tool_search_elevation;
pub mod tools;
pub use pony_storage::trace_persistence;
pub mod tui_preferences;
pub use pony_protocol::turn_event;
pub mod turn_flow;
pub mod updater;
pub use pony_storage::usage_governance;
pub use pony_observability::usage_ledger;
pub use pony_tools::web_access;
pub use pony_tools::workspace;
pub use pony_tools::workspace_hotloader;
pub(crate) mod tools_orchestration;

pub(crate) fn ensure_runtime_bridges() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        pony_protocol::turn_flow_registry::register_external_turn_event_emitter(
            turn_flow::emit_external_turn_event,
        );
        pony_protocol::host_hooks::register_teammate_git_env_provider(
            teammate_identity::git_identity_env_for_session,
        );
        pony_protocol::host_hooks::register_subagent_wired_probe(|| {
            subagent_runtime::subagent_engine().is_some()
        });
        pony_tools::tools::register_orchestration_bridge(std::sync::Arc::new(
            tools_orchestration::CoreOrchestrationBridge,
        ));
    });
}
