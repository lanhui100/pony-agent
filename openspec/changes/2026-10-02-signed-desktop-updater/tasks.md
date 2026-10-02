# PA-103 Signed desktop self-update

- [ ] Confirm remote pproxy evidence and endpoint/signing assumptions. (Release-owner gate; no production evidence available.)
- [x] Add signed updater design/spec and ADR proposal; record three-way review decisions.
- [x] Add Tauri updater/process dependencies, plugin registration, signed endpoints/public key configuration and minimal capability permissions. (Configuration remains empty/fail-closed.)
- [x] Add updater adapter and extend update store with Tauri check/download/install/relaunch state machine.
- [x] Update settings UI for progress, one-click install and truthful failures; preserve browser check-only behavior.
- [x] Add/update unit and component tests for success, concurrency, browser fallback and failure paths. (Local disabled/source-separation regression added; release fixture remains gated.)
- [ ] Run version, targeted tests, typecheck, build, Rust check and full verify where baseline permits. (Typecheck passes; tests/build/Rust are blocked by baseline environment/lockfile/network constraints.)
- [ ] Run three-way implementation review, apply accepted fixes, update docs/ADR/task board/dashboard/session log, and archive the change. (Release-owner evidence and final archive remain follow-up.)
