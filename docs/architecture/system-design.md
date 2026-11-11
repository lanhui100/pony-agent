# System Design: Pony Agent Skill System Refactoring

Status: Proposed

## 1. Context
Refactoring `pony-agent` skill discovery and invocation to align with DSH architecture (Wave-58/D5' criteria).

## 2. Core Goals
- Implement Skill Invocation Policy (4-quadrant gate).
- Ensure skill discovery across workspace boundaries (non-env-dependent).
- Catalog preservation despite budget constraints.

## 3. Architecture
- **Skill Discovery**: Config-based `skillDirs` + Hot-reload (Watchdog).
- **Invocation Policy**: `disable-model-invocation` + `user-invocable` frontmatter fields.
- **Triage CLI**: Robust ID generation (Max Index + 1).

## 4. Backlog
- [B-001] Protocol Stub & NFR Baseline
- [B-002] Workspace Hotloader Logic Implementation
- [B-003] Triage CLI Robustness

