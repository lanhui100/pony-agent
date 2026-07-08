# Tasks

- [x] Confirm the current main conversation streaming render path and buffered flush behavior against implementation.
- [x] Add or update design notes for the chosen streaming reveal architecture and rejected alternatives.
- [x] Implement streaming-only markdown stabilization without changing final terminal transcript truthfulness.
- [x] Implement smoother buffered reveal behavior for flushed text batches with reduced-motion support.
- [x] Add a visible in-progress affordance and ensure assistant actions remain aligned with message terminal state.
- [x] Add runtime rollback/observability controls needed to tune or disable the optimization.
- [x] Add focused tests for markdown boundaries, reveal batching, terminal fallback, reduced motion, and scroll stability.
- [ ] Run typecheck, tests, and build validation; record results in the implementation handoff.

## Validation Notes

- Passed: `cmd /c npm run test:unit -- tests/markdown.spec.ts tests/MarkdownRenderer.spec.ts tests/useStreamingPresentationState.spec.ts tests/HomeWorkspace.spec.ts`
- Passed: `cmd /c npm run build`
- Blocked full unit gate: `cmd /c npm run test:unit` and `cmd /c npm run test:unit -- tests/HomeSidebar.spec.ts` currently fail in `tests/HomeSidebar.spec.ts` because `tools-panel-toggle` / `Tools` are absent from the rendered sidebar. This change does not modify `HomeSidebar.vue` or its tests.
