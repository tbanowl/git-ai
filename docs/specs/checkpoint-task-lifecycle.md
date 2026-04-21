# Checkpoint Task Lifecycle and Recovery Spec

**Status:** Draft
**Scope:** Current-lineage checkpoint durability and recovery
**Audience:** git-ai maintainers
**Decision:** Track checkpoint closure by task lifecycle, not prehook/posthook pairing

---

## 1. Summary

This spec defines a durable checkpoint task system for `git-ai` that guarantees closure of checkpoint-related work for the **current base commit / current lineage only**.

A checkpoint payload received from an AI tool becomes a durable `CheckpointTask`. A task is considered **closed** when it reaches one of two terminal outcomes:

1. **Applied/Completed**: successfully incorporated into the current lineage's attribution state
2. **Obsolete**: explicitly invalidated because lineage changed (reset, rebase, switch)

This design intentionally does **not** require old-lineage tasks to continue across rewrite operations.

---

## 2. Problem Statement

In VM environments with poor disk IO, background processing around git command execution may occasionally hang. The system needs a reliable way to ensure:

- checkpoint payloads are durably captured
- interrupted processing can be recovered
- commit-time correctness can be enforced
- stale tasks from old lineages do not block unrelated future commits

The existing system is strong at base-commit-scoped working logs, but lacks a first-class durable task lifecycle with explicit recovery semantics.

---

## 3. Goals

### 3.1 Goals
- Durably persist checkpoint payloads before they are fully applied
- Provide explicit lifecycle states for checkpoint processing
- Allow recovery after crash/hang/interruption
- Block commit only when **relevant current-lineage** tasks are incomplete
- Mark old-lineage unfinished tasks as obsolete after lineage-changing operations
- Support idempotent recovery using explicit applied evidence

### 3.2 Non-Goals
- Guarantee continuation of unfinished tasks across reset/rebase/switch
- Replace existing `working_logs/<base_commit>/` storage model
- Solve all underlying performance issues in working log writes
- Introduce a global repo-wide FIFO checkpoint queue
- Require prehook/posthook symmetry as the definition of correctness

---

## 4. Definitions

### 4.1 CheckpointTask
A durable work unit created from an incoming checkpoint payload.

### 4.2 Relevant Task
A task is **relevant** to the current commit if all are true:
- same `repo_workdir`
- same `lineage_epoch`
- same effective `base_commit`
- state is not `Obsolete`
- state is not `Completed`
- state is not `Applied`

### 4.3 Lineage Epoch
A monotonically increasing repo-local counter that advances whenever repository history/context changes enough that unfinished old tasks should no longer affect current commit correctness.

Examples: reset, rebase completion, switch, checkout to different lineage.

### 4.4 Applied Evidence
Durable proof that a task has already been successfully applied to attribution state. Recovery must consult this before retrying apply.

### 4.5 Closure
A task is considered **closed** when it reaches `Applied`/`Completed` or `Obsolete`.

---

## 5. Design Overview

The system models checkpoint processing as a task lifecycle:

```
Captured -> Ready -> Applying -> Applied -> Completed
                    \-> FailedRetryable -> Ready
Any non-terminal state -> Obsolete
```

The correctness boundary is:

> Every relevant task must eventually become either Applied/Completed or Obsolete.

---

## 6. State Machine

### 6.1 States

- **`Captured`**: payload durably stored, not yet validated
- **`Ready`**: validated, eligible to be processed
- **`Applying`**: actively being applied to attribution state
- **`Applied`**: successfully incorporated into current-lineage attribution state
- **`Completed`**: optional cleanup/finalization done
- **`FailedRetryable`**: processing failed, retry allowed
- **`Obsolete`**: belongs to old lineage, no longer relevant

### 6.2 Transition Table

| Current State | Event | Condition | Action | Next State |
|---|---|---|---|---|
| none | payload received | durable payload write succeeds | create task record | `Captured` |
| `Captured` | prepare/validate | validation succeeds | enrich task metadata | `Ready` |
| `Captured` | prepare fails | retryable error | record error/retry | `FailedRetryable` |
| `Ready` | processing starts | task relevant, lock acquired | set processing timestamp, increment attempts | `Applying` |
| `Ready` | processing start fails | retryable error | record error/retry | `FailedRetryable` |
| `Applying` | apply succeeds | working log updated and evidence recorded | set applied timestamp | `Applied` |
| `Applying` | apply fails | retryable error | record error/retry, release lease | `FailedRetryable` |
| `Applying` | recovery scan | processing stale | consult applied evidence | `Applied` or `FailedRetryable` |
| `FailedRetryable` | retry due | task still relevant | reschedule | `Ready` |
| `Applied` | final cleanup done | optional | cleanup/archive payload | `Completed` |
| any non-terminal | lineage advanced | task no longer relevant | record obsolete reason | `Obsolete` |

---

## 7. Commit Correctness Rules

### 7.1 Commit Gate

Before allowing a commit, the system must:

1. identify relevant tasks for the current repo/base-commit/epoch
2. attempt recovery/drain once
3. re-scan relevant tasks
4. reject commit if any relevant task remains in: `Captured`, `Ready`, `Applying`, `FailedRetryable`

Tasks in `Applied`, `Completed`, `Obsolete` must **not** block commit.

### 7.2 Commit Gate Error Behavior

```
Pre-commit failed: found N pending checkpoint task(s) relevant to the current base commit.
Run recovery or retry checkpoint processing before commit.
```

---

## 8. Recovery Model

### 8.1 Recovery Triggers
Recovery may run from any of these entry points:
1. before processing a new checkpoint
2. during commit pre-check
3. via explicit maintenance command (`git-ai checkpoint-recover`)

### 8.2 Recovery Rules

**Stale `Applying`**: if `processing_started_at` exceeds timeout threshold:
- check applied evidence
- if evidence exists → mark `Applied`
- otherwise → mark `FailedRetryable`

**`FailedRetryable`**: if `next_retry_at <= now` and task remains relevant → transition back to `Ready`

**Old Epoch / Old Base Commit**: if task no longer relevant due to lineage change → mark `Obsolete`

---

## 9. Lineage Handling

### 9.1 Lineage Policy
Only guarantee closure for the current base commit / current lineage. Old-lineage unfinished tasks may be marked obsolete.

### 9.2 Epoch Advancement
The repository-local `lineage_epoch` must advance on:
- reset
- rebase completion
- switch
- checkout to different lineage

After epoch advancement, unfinished tasks from older epochs are no longer relevant and should be marked `Obsolete`.

---

## 10. Idempotency

### 10.1 Principle
State transitions alone do **not** provide safe recovery. Recovery correctness requires durable **applied evidence**.

### 10.2 Required Keys

- **`task_id`**: unique task identity
- **`dedupe_key`**: semantic fingerprint to prevent duplicate logical tasks

Recommended dedupe_key inputs: canonical repo identity/path, `base_commit`, `lineage_epoch`, checkpoint kind, explicit paths, payload content hash, agent session/thread identity.

### 10.3 Applied Evidence
Before marking a task `Applied`, the system must durably record:
- `task_id`
- `dedupe_key`
- `base_commit`
- `applied_at`
- optional result hash

Recovery: if evidence exists, task is already applied; only tasks without evidence may be retried.

---

## 11. Persistence Model

### 11.1 Logical Components
1. task records
2. payload blobs/files
3. applied evidence
4. lineage epoch state

### 11.2 Recommended Model
- **SQLite** for task metadata/evidence/epoch
- **filesystem payload files** for raw checkpoint payloads

### 11.3 Schema

**`checkpoint_tasks`**
- `task_id TEXT PRIMARY KEY`
- `repo_workdir TEXT NOT NULL`
- `base_commit TEXT NOT NULL`
- `lineage_epoch INTEGER NOT NULL`
- `kind TEXT NOT NULL`
- `author TEXT NOT NULL`
- `state TEXT NOT NULL`
- `payload_ref TEXT NOT NULL`
- `dedupe_key TEXT NOT NULL UNIQUE`
- `explicit_paths TEXT NOT NULL`
- `is_pre_commit INTEGER NOT NULL`
- `captured_at_ms INTEGER NOT NULL`
- `processing_started_at_ms INTEGER`
- `applied_at_ms INTEGER`
- `completed_at_ms INTEGER`
- `obsolete_at_ms INTEGER`
- `attempts INTEGER NOT NULL DEFAULT 0`
- `last_error TEXT`
- `next_retry_at_ms INTEGER`

Indexes: `(repo_workdir, lineage_epoch, base_commit, state)`, `(state, next_retry_at_ms)`

**`checkpoint_applied_evidence`**
- `task_id TEXT PRIMARY KEY`
- `dedupe_key TEXT NOT NULL`
- `base_commit TEXT NOT NULL`
- `applied_at_ms INTEGER NOT NULL`
- `apply_result_hash TEXT`

**`checkpoint_lineage_state`**
- `repo_workdir TEXT PRIMARY KEY`
- `current_epoch INTEGER NOT NULL`
- `updated_at_ms INTEGER NOT NULL`

---

## 12. Module Responsibilities

### 12.1 New Modules

- **`checkpoint_tasks/store`**: create/read/update task records, query relevant tasks, record failure, mark obsolete
- **`checkpoint_tasks/recovery`**: recover stale applying tasks, retry failed tasks, drain relevant tasks
- **`checkpoint_tasks/evidence`**: record applied evidence, lookup applied evidence, determine whether task was already applied
- **`checkpoint_tasks/lineage`**: get/bump current epoch, evaluate relevance, obsolete old-epoch tasks
- **`checkpoint_tasks/runner`**: bridge task lifecycle to existing checkpoint execution backend

### 12.2 Existing Code Integration

- **`src/commands/checkpoint.rs`**: durable capture, validation/preparation, apply execution, applied evidence recording
- **`src/authorship/pre_commit.rs`**: commit gate — drain/recover relevant tasks, reject commit if relevant unfinished tasks remain
- **Lineage-changing hooks**: reset, checkout, switch, rebase completion — bump epoch, obsolete stale tasks

---

## 13. Failure Scenario Matrix

### Capture Phase

| Crash Point | Durable State | Recovery |
|---|---|---|
| before payload write | nothing persisted | no recovery needed |
| payload written, task not written | orphan payload | cleanup orphan payload |
| task persisted as `Captured` | valid durable task | prepare/recover later |

### Apply Phase

| Crash Point | State | Recovery |
|---|---|---|
| before entering `Applying` | `Ready` | retry normally |
| after entering `Applying`, before actual apply | `Applying` | stale timeout recovery |
| working log updated, evidence missing | `Applying` | detect via dedupe/evidence strategy before retry |
| evidence written, task still `Applying` | `Applying` | promote to `Applied` during recovery |

### Lineage Change

| Event | Old Task State | Recovery Action |
|---|---|---|
| reset | non-terminal | mark `Obsolete` |
| switch | non-terminal | mark `Obsolete` |
| rebase completion | non-terminal | mark `Obsolete` |
| checkout to different lineage | non-terminal | mark `Obsolete` |

---

## 14. Invariants

1. A task belongs to exactly one `repo_workdir + base_commit + lineage_epoch`.
2. Only relevant unfinished tasks may block commit.
3. Recovery must check applied evidence before retrying apply.
4. Old-epoch unfinished tasks must not block current commit.
5. `Completed` and `Obsolete` are terminal states.
6. `Applied` is sufficient for commit correctness and must not block commit.

---

## 15. Minimal Viable Scope

### Included in MVP
- durable task records
- lifecycle states: `Captured`, `Ready`, `Applying`, `Applied`, `FailedRetryable`, `Obsolete`
- applied evidence
- commit gate
- lineage epoch bumping and obsoletion

### Deferred
- sophisticated cleanup/archive
- advanced diagnostics/UI
- multi-repo orchestration

---

## 16. Alternatives Considered

### 16.1 Four-State Prehook/Posthook Model
Rejected because it centers hooks rather than task correctness, cannot naturally model obsolete tasks, does not represent interrupted in-progress apply, and does not itself provide idempotent recovery.

### 16.2 Global Repo-Wide Drain-All Queue
Rejected because it conflicts with base-commit-scoped current architecture, introduces head-of-line blocking, and can let old-lineage tasks block unrelated commits.

---

## 17. Open Questions

1. Should `amend` always advance lineage epoch, or only some amend paths?
2. Should `Applied` payload cleanup be eager or deferred?
3. Should task/evidence storage be repo-local only, or also mirrored into existing internal DB?
4. What timeout threshold should define stale `Applying` tasks?

---

## 18. Recommendation

Proceed with an implementation that:
- introduces durable checkpoint tasks
- uses lifecycle closure rather than hook-pair closure
- scopes correctness to current lineage only
- relies on explicit applied evidence for safe recovery
- marks old-lineage unfinished tasks obsolete on rewrite/navigation events
