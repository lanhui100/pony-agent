# Design

## Decision Summary

Use a single streaming markdown presentation model:

1. Pending assistant content is rendered by one `MarkdownRenderer` instance in streaming mode.
2. The renderer suppresses raw markdown fallback while forced markdown rendering is active, so raw text and parsed markdown are never visible at the same time.

The design must preserve two product truths:

- streaming content should feel continuous and stable;
- terminal transcript content must remain truthful and must not contain fabricated markdown closures.

## Chosen Direction

### 1. Streaming-only partial markdown stabilization

- Keep `renderMarkdown()` pure.
- Use a streaming-only wrapper such as `renderPartialMarkdown()` to auto-close fenced code blocks while the message is still `pending`.
- Keep synthetic completions render-only: never append them to message content, copy payloads, or persisted transcript state.
- Stabilize only conservative tail wrappers in this phase: fenced code blocks, inline code, and strong markers. Links, tables, HTML, and ambiguous nesting remain truthful raw streaming text until a safe boundary.
- When the message reaches `done`, `error`, or `cancelled`, force a final render through the original non-auto-closed markdown path.

### 2. Buffered reveal remains batch-driven, not raw-chunk-driven

- Keep the current character/time buffering model as the trigger.
- Do not reveal each network chunk immediately.
- Flush when either the character threshold or the time threshold is reached.
- Use a smaller threshold for the first visible batch and for code-fence-active batches.

### 3. Reveal layer should avoid unsafe markdown character splitting

The strongest review objection is that naive character-level animation inside markdown content can cut across syntax boundaries such as:

- fenced code markers;
- emphasis markers like `**`;
- inline code markers;
- links and tables.

Therefore the reveal layer must not assume that any arbitrary character boundary is render-safe.

The implementation should prefer one of these safe approaches:

- render the entire pending assistant message through a single streaming markdown surface; or
- reveal only safe trailing text outside unstable markdown structures; or
- keep batch-level fade for markdown-rich segments and reserve staggered reveal for plain trailing text segments.

This change explicitly does **not** require fully character-level markdown DOM animation across arbitrary rendered HTML.

Implementation note: the main conversation now renders pending assistant content through one streaming markdown component. It no longer uses a separate raw tail/fade layer for the main response, which avoids raw markdown and parsed markdown being visible together and removes remount-driven flashing.

### 4. In-progress affordance

- While assistant status is `pending`, render an inline streaming indicator within the assistant message area.
- This indicator must disappear when the message becomes terminal.

### 5. Runtime rollback and observability

- Add a runtime feature flag to disable the optimization locally.
- Add tuning overrides for batch-size and time thresholds.
- Record metrics for flush size distribution, partial-render usage, and render cadence.

## Rejected Alternatives

### Replace main transcript markdown with plain text

Rejected because it removes desired rendering capability instead of fixing streaming behavior.

### Arbitrary per-character DOM animation across rendered markdown HTML

Rejected for phase 1 because it is fragile at markdown syntax boundaries, increases DOM churn, and risks GPU/layout instability.

### Full custom incremental markdown renderer

Rejected for scope reasons. Too large for this optimization and unnecessary for the current acceptance target.

## Risks and Mitigations

### Risk: misleading pseudo-complete markdown during streaming

Mitigation: terminal-state render must use the truthful non-auto-closed path.

### Risk: reveal layer causes scroll jitter

Mitigation: co-design with timeline auto-scroll and apply debounced or compensated scroll updates around render-complete events.

### Risk: over-animation harms accessibility or performance

Mitigation: support `prefers-reduced-motion`; cap reveal granularity; prefer batch fade fallback for markdown-rich segments.

## Review Record

- Architect/consultant review: adopt a new change instead of extending `optimize-agent-stream-ui`; keep markdown; write explicit rollback and truthful-terminal requirements.
- Frontend adversarial review: do not commit to naive per-character markdown DOM splitting; keep reveal behavior safe around syntax boundaries and scroll/animation performance.
