# Tasks

- [ ] Confirm the current main conversation streaming render path and buffered flush behavior against implementation.
- [ ] Add or update design notes for the chosen streaming reveal architecture and rejected alternatives.
- [ ] Implement streaming-only markdown stabilization without changing final terminal transcript truthfulness.
- [ ] Implement smoother buffered reveal behavior for flushed text batches with reduced-motion support.
- [ ] Add a visible in-progress affordance and ensure assistant actions remain aligned with message terminal state.
- [ ] Add runtime rollback/observability controls needed to tune or disable the optimization.
- [ ] Add focused tests for markdown boundaries, reveal batching, terminal fallback, reduced motion, and scroll stability.
- [ ] Run typecheck, tests, and build validation; record results in the implementation handoff.
