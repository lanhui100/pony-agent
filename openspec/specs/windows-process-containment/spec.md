# windows-process-containment Specification

## Purpose

Define Windows Job Object lifecycle containment for managed processes without changing sandbox authorization. This is best-effort post-spawn process-tree lifecycle protection, not an escape-proof boundary or filesystem/network sandbox; the scheduler-dependent spawn-to-assignment window is explicitly retained.

## Requirements

### Requirement: Windows managed processes SHALL own lifecycle Jobs

On Windows, ProcessManager SHALL create a private non-inheritable Job Object with kill-on-close enabled and without either breakaway permission flag for each managed child. It SHALL assign the child immediately after spawn and verify membership before returning an opaque process handle. This contract is best-effort post-spawn containment, not an escape-proof sandbox; pre-assignment descendants may escape and the assignment window has no fixed scheduling-independent time bound.

#### Scenario: Successful Windows start

- **WHEN** ProcessManager successfully starts a Windows child
- **THEN** the child is verified as a member of its private Job before its opaque handle is returned
- **AND** kill-on-close is enabled and breakaway permission flags are absent.

#### Scenario: Job setup or assignment fails

- **WHEN** Job creation, configuration, assignment or membership verification fails
- **THEN** start returns an operation-specific error and publishes no process handle
- **AND** any spawned direct child is terminated/reaped and owned Job handles are released rather than silently falling back to an uncontained success.

#### Scenario: A short-lived child exits before assignment

- **WHEN** the direct child exits before Job assignment or verification can succeed
- **THEN** start still reports that failure and publishes no handle, rather than inferring safety from parent exit
- **AND** if assignment and verification do succeed, the existing terminal polling contract is retained.

### Requirement: Windows cancellation SHALL terminate owned descendants

Kill and shutdown SHALL terminate all processes in the owned Job, including when the direct child has already exited. The final direct-child state SHALL remain pollable according to the existing lifecycle contract; terminal cleanup SHALL release Job ownership. Last-owner disposal SHALL close the Job and terminate remaining members.

#### Scenario: Kill a ready process tree

- **WHEN** a managed child has created a descendant after assignment and kill is requested by its owning session
- **THEN** the direct child and contained descendant terminate within a bounded test deadline
- **AND** the direct child's final status remains pollable until terminal handle cleanup.

#### Scenario: Parent exits before shutdown

- **WHEN** the direct child has exited while a contained descendant is still running
- **AND** the owning session is shut down
- **THEN** that descendant is terminated and the session process record is removed.

#### Scenario: Final owner is dropped

- **WHEN** the final owner of a managed process record is disposed
- **THEN** its Job handle is closed and all remaining Job members are terminated.

#### Scenario: Cross-session cleanup is rejected

- **WHEN** another session attempts to kill a process handle
- **THEN** the existing ownership check rejects it and neither that Job nor unrelated Jobs are terminated.

### Requirement: Containment SHALL remain independent of sandbox authorization

Job Object containment SHALL NOT register an available SandboxBackend, authorize autonomous Run, or change existing filesystem/network approval semantics. Non-Windows process behavior SHALL remain unchanged.

#### Scenario: No real sandbox backend

- **WHEN** an autonomous Run requires a sandbox and no real SandboxBackend is registered
- **THEN** execution remains fail-closed even though Windows Job containment exists.

#### Scenario: Non-Windows compilation

- **WHEN** core is compiled on a non-Windows platform
- **THEN** Win32 Job implementation and imports are excluded and the existing process lifecycle path is retained.
