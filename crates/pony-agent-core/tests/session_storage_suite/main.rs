#[path = "../support/paths.rs"]
pub mod support;
#[path = "../support/env_lock.rs"]
mod env_lock;
#[path = "../support/test_name.rs"]
mod test_name;

mod b214_retry_event_persistence;
mod b219_degraded_turn_acceptance;
mod b221_fallback_body_acceptance;
mod b222_history_rebuild_idempotent;
mod b239_log_rebuild_event_times;
mod b240_refold_retry_attribution;
mod b254_stream_only_followup;
mod non_streaming_provider_session_id_red;
mod p0_log_first_durability;
mod p1_multiprocess_writers;
mod pa174_history_wiring_and_followup_contract;
mod session_bundle_export_import;
mod session_event_carriers;
mod session_fold_rebuild;
mod session_id_contract;
mod session_log_migration;
mod session_log_store;
mod session_log_wiring;
mod session_persistence_observability_red;
mod session_read_from_log;
mod session_rebuild_compare;
mod session_row_heal;
mod session_stop_blob_write;
mod sparse_session_store_test;
mod storage_stage0_busy_classification;
mod storage_stage0_busy_locked_retry;
mod storage_stage0_orphan_backfill;
mod storage_stage0_recover_trace;
mod storage_stage0_tombstone_no_resurrect;
mod storage_stage0_write_amplification;
mod targeted_session_load_test;
mod tui_rewind_acceptance;
mod tui_rewind_capture_seam;
mod tui_session_resilience_chunk_split;
mod turn_failed_keeps_partial_text_test;
