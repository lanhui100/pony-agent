#[path = "../support/env_lock.rs"]
mod env_lock;

mod approval_resume_e2e;
mod ask_pending_adversarial;
mod ask_pending_contract;
mod ask_persistence_acceptance;
mod ask_resume_e2e;
mod ask_user_acceptance;
mod b193_2_goal_stop_visibility_contract;
mod graph_run_lock_concurrency_red;
mod graph_run_submission_plan_deadlock;
mod harness_optimizations_test;
mod job_session_scope_test;
mod pa104_terminal_acceptance;
mod pa105_job_acceptance;
#[cfg(unix)]
mod process_group_reap_test;
mod promote_script_cleanup_test;
#[cfg(unix)]
mod range_quiescence_test;
#[cfg(unix)]
mod run_exit_code_class_test;
mod run_timeout_promote_test;
mod stop_contract_wave60;
mod stop_escalation_wave60;
mod stop_graph_run_wave64;
mod stop_interruptible_read_wave60;
mod stop_sequence_wave60;
mod timeout_signal_promote_test;
mod tool_timeout_policy_test;
mod wave2_b074_watchdog;
mod wave62_tool_cancel;
mod wave62_tool_cancel_e2e;
mod wave9_adversarial_test;
mod wave9_green_acceptance_test;
