# Tasks

- [x] Refactor streaming assistant rendering so pending assistant messages use a plain-text lightweight streaming render path inside a stable shell and completed assistant messages keep the final markdown path.
- [x] Remove the fade/stable handoff gap so fade content is not removed before the stable layer is ready.
- [x] Connect markdown render-complete events into timeline auto-follow and make that signal the primary streaming follow trigger instead of relying only on ResizeObserver catch-up.
- [x] Keep pending and completed assistant content inside a stable shell so terminal handoff does not replace the whole content structure.
- [x] Add focused tests for streaming render-path behavior, delta/fallback safety, checkout/terminal handoff regressions, and revision-mismatch fallback.
- [x] Run targeted validation commands and record results.

## Validation Results

- 2026-07-07: `cmd /c npm run test:unit -- tests/useStreamingPresentationState.spec.ts tests/MarkdownRenderer.spec.ts` passed (12 tests).
- 2026-07-07: `cmd /c npx vitest run tests/HomeWorkspace.spec.ts -t "streaming"` passed (11 selected tests).
- 2026-07-07: `cmd /c npx vue-tsc --noEmit` passed.
- 2026-07-07: `cmd /c npx vite build` passed with pre-existing Rolldown pure-annotation/chunk-size warnings.
- 2026-07-07: `cmd /c npm run build` was split after timing out at 180s; both component commands passed separately.
