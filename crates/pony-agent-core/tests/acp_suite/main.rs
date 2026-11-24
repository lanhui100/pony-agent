#[path = "../support/paths.rs"]
pub mod support;
#[path = "../support/env_lock.rs"]
mod env_lock;
#[path = "../support/test_name.rs"]
mod test_name;

mod acp_capabilities_contract;
mod acp_cli_capabilities;
mod acp_cli_resume_and_epipe;
mod acp_client_contract;
mod acp_config_regressions;
mod acp_delegate_adversarial;
mod acp_delegate_p0p2_contract;
mod acp_delegate_persistence_contract;
mod acp_host_surface;
mod acp_list_sessions;
mod acp_mcp_wiring;
mod acp_p0_acceptance;
mod acp_p0_adversarial;
mod acp_p1_blockers;
mod acp_proxy_autowire;
mod acp_realtime_permission;
mod acp_session_lifecycle;
mod acp_tool_permission_e2e;
mod acp_updates_meta;
