# optimize-agent-stream-ui

## Background

Agent messages in the main conversation currently go through markdown rendering, which can delay updates and cause layout jumps during streaming. Small stream deltas also create a choppy typewriter feel, and message action buttons appear while the assistant is still streaming.

## Goals

- Render assistant messages in the main conversation as plain text.
- Add stable system-prompt guidance that replies should avoid markdown and stay concise by default.
- Batch streamed text by character and time thresholds so frontend fade-in presentation is smoother.
- Hide assistant message action buttons until streaming has ended.

## Non-goals

- Remove markdown utilities globally.
- Change user-message rendering.
- Change stored transcript content or copy payloads.

## Validation

- Unit tests cover plain assistant rendering, action-button visibility, stream buffering, and prompt guidance.
- Build/typecheck should pass for the changed frontend and Rust core code.
