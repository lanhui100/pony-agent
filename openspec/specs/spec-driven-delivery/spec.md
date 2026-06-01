# Spec-Driven Delivery

## Purpose

Define the repository contract for when complex development tasks must use OpenSpec and how OpenSpec artifacts stay synchronized with the existing task system.

## Requirements

### Requirement: Complex development tasks require an OpenSpec change
For complex development tasks, the repository SHALL create an OpenSpec change under `openspec/changes/<name>/` before implementation begins.

#### Scenario: Cross-boundary change is requested
- **WHEN** a task introduces a new capability, changes cross-layer boundaries, requires migration planning, or is expected to span multiple sessions
- **THEN** implementation planning SHALL start from an OpenSpec change with proposal, specs, design, and tasks artifacts

### Requirement: Simple tasks may use the lightweight task flow
The repository SHALL allow simple, low-risk tasks to proceed without a full OpenSpec change when the work is local, behavior-preserving, and unambiguous.

#### Scenario: Small local bug fix
- **WHEN** a task is limited to a small bug fix, copy change, comment update, or other single-session maintenance work
- **THEN** the task MAY proceed directly through the existing task card workflow without creating an OpenSpec change

### Requirement: Complex task cards reference OpenSpec artifacts
Every complex task tracked in `management/task-system/03_TASKS/` SHALL reference its OpenSpec change and relevant canonical specs.

#### Scenario: Complex task enters implementation
- **WHEN** a complex task moves into active execution
- **THEN** its task card SHALL record the OpenSpec change path, related canonical spec paths, and current spec status

### Requirement: Completion syncs task system and OpenSpec state
When a complex task is completed, the task system and OpenSpec state SHALL be updated together so implementation evidence, canonical specs, and archive status do not drift.

#### Scenario: Complex task is ready to close
- **WHEN** implementation and verification are complete for a complex task
- **THEN** the owner SHALL sync completion evidence back to the task card and logs, update canonical specs if needed, and archive the OpenSpec change when the workflow is finished
