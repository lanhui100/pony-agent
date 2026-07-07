# optimize-agent-stream-ui

## Background

Agent messages in the main conversation still jump vertically during streaming. The visible jitter is not caused by markdown syntax alone. It comes from four systems mutating the same visual region on different clocks:

- streaming fade/stable presentation state
- async markdown partial rendering
- ResizeObserver-driven auto-scroll compensation
- pending-to-final DOM structure switching

The result is that the message body reflows while scroll logic tries to catch up after layout changes, producing visible up-down wobble instead of a single smooth upward motion.

## Goals

- Keep assistant streaming growth visually monotonic so the transcript moves upward smoothly.
- Reduce layout reflow during streaming by using a lighter render path while the assistant is pending.
- Keep the final completed assistant transcript on the normal markdown render path.
- Make scroll follow behave like bottom-anchor following, not repeated resize compensation chasing.
- Preserve existing reasoning/tool/error behavior unless directly needed for streaming smoothness.

## Streaming render-path decision

For this change, the lightweight pending-assistant render path means:

- pending assistant main text is rendered as plain text inside a stable shell container
- the pending path SHALL NOT call the full markdown-to-HTML render pipeline for every flushed delta
- the completed assistant path still uses the normal MarkdownRenderer so final formatting is preserved

This change is intentionally scoped to the pending assistant body only. It does not redefine final transcript rendering, stored content, or copy payloads.

## Non-goals

- Remove markdown rendering from completed assistant messages.
- Change persisted transcript content, copy payloads, or backend message semantics.
- Redesign the whole conversation card layout.
- Solve every possible long-document markdown performance issue outside the streaming path.

## Validation

- Unit tests cover streaming render-path selection, revision-safe state transitions, and auto-follow stability hooks.
- Focused workspace tests cover pending assistant rendering and terminal handoff behavior.
- Build and targeted unit tests pass for the changed frontend code.
- Manual verification confirms the viewport moves in a single forward-follow direction during normal streaming and no visible fade-to-stable empty gap remains.
