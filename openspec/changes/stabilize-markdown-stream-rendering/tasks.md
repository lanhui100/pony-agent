# Tasks

- [x] Confirm the current main conversation streaming render path and buffered flush behavior against implementation.
- [x] Add or update design notes for the chosen streaming reveal architecture and rejected alternatives.
- [x] Implement streaming-only markdown stabilization without changing final terminal transcript truthfulness.
- [x] Implement smoother buffered reveal behavior for flushed text batches with reduced-motion support.
- [x] Add a visible in-progress affordance and ensure assistant actions remain aligned with message terminal state.
- [x] Add runtime rollback/observability controls needed to tune or disable the optimization.
- [x] Add focused tests for markdown boundaries, reveal batching, terminal fallback, reduced motion, and scroll stability.
- [x] Run typecheck, tests, and build validation; record results in the implementation handoff.

## Validation Notes

- Passed: `cmd /c npm run test:unit -- tests/HomeSidebar.spec.ts`
- Passed: `cmd /c npx vue-tsc --noEmit`
- Passed: `cmd /c npm run test:unit`
- Passed: `cmd /c npm run build`
- Build emitted non-blocking warnings from dependencies/tooling: Rolldown `INVALID_ANNOTATION` notices in `@vueuse/core`, a chunk-size warning for `dist/assets/index-*.js`, and plugin timing diagnostics. The build still completed successfully.
