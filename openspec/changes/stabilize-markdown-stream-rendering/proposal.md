# stabilize-markdown-stream-rendering

## Background

The main conversation area still renders assistant messages through the markdown pipeline during streaming. Although the frontend already buffers stream text by character and time thresholds, the current presentation still updates in batch-sized jumps and can visibly reflow when markdown structure becomes renderable mid-stream.

The existing `optimize-agent-stream-ui` change focused on plain-text rendering and prompt-side markdown avoidance. That direction does not match the current product goal: keep markdown rendering capability in the main conversation while making streaming presentation feel smoother and more stable.

## Goals

- Keep assistant markdown rendering in the main conversation area.
- Reduce visible layout jumps while assistant markdown is still incomplete during streaming.
- Preserve buffered character/time flushing, but present each flushed batch through a smoother streaming reveal.
- Make streaming state visually obvious so partially rendered markdown is not mistaken for a final answer.
- Keep the solution observable and reversible with runtime flags.

## Non-goals

- Replacing the markdown renderer with a new incremental parser.
- Removing markdown from assistant responses.
- Solving every markdown partial-render edge case in one pass.
- Changing stored transcript content, copy payloads, or backend stream protocol.

## Scope

This change covers the main conversation assistant message area only. It includes partial markdown stabilization for streaming, buffered reveal behavior, in-progress affordances, rollback controls, and related tests/documentation.

## Risks

- Splitting streaming content at unsafe markdown boundaries can cause incorrect intermediate rendering.
- Character-level DOM animation can cause excessive node count, GPU pressure, and scroll instability.
- Auto-closing markdown fences during streaming can mislead users if final terminal states do not revert to truthful rendering.

## Validation

- Unit/component tests cover streaming partial markdown stabilization, buffered reveal thresholds, terminal-state fallback, and in-progress affordances.
- Validation covers reduced-motion behavior, rollback flag behavior, and scroll stability near the bottom of the timeline.
- Frontend typecheck, test, and build commands pass for the changed code.
