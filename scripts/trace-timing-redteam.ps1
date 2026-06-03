$ErrorActionPreference = "Stop"

Write-Host "trace timing redteam"
Write-Host "targets: tests/HomeSidebar.spec.ts tests/runtime-store.spec.ts"

npm exec vitest run tests/HomeSidebar.spec.ts tests/runtime-store.spec.ts
cargo test --lib start_turn_stream_emits_first_token_latency_on_reasoning_delta --manifest-path src-tauri/Cargo.toml
cargo test --lib start_turn_stream_sync_fallback_for_initial_decision_does_not_emit_fake_ttft --manifest-path src-tauri/Cargo.toml
cargo test --lib start_turn_stream_uses_live_stream_for_deepseek_tool_followup --manifest-path src-tauri/Cargo.toml
