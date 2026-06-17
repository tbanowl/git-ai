# Rebase Conflict Manual Commit Authorship Design

## Context

When a local AI-authored commit is pushed after another client has already
updated the remote branch, the code push can fail after the authorship note has
already been pushed. A common recovery flow is:

1. Local history contains `A -> B`.
2. `B` has AI authorship notes and commit metrics.
3. Remote history contains `A -> C`.
4. The user runs `git pull --rebase`, hits a conflict while replaying `B`, and
   resolves it.
5. Some clients commit the resolution directly with `git commit`, producing
   `A -> C -> D`, instead of using `git rebase --continue`.

In that flow, `B` drops out of `git log`, but the code from `B` is present in
`D`. Git AI must preserve `B`'s AI attribution on `D`.

Git leaves `.git/REBASE_HEAD` and `rebase-merge/stopped-sha` in place after a
direct conflict-resolution commit. A later ordinary commit `E` can still read
the stale `REBASE_HEAD`. Treating those files as a live source during every
`git commit` incorrectly maps `B -> E`, causing stale prompt metrics and AI
attribution to leak into the follow-up commit.

## Goals

- Preserve AI authorship when a rebase conflict is resolved by direct
  `git commit` instead of `git rebase --continue`.
- Ensure each stopped rebase pick is consumed at most once.
- Prevent stale Git rebase state from affecting later normal commits.
- Keep wrapper and daemon behavior consistent.
- Preserve remote server correctness when old notes and metrics for `B` were
  already pushed before `D` is created.

## Non-Goals

- Do not infer arbitrary commit equivalence from patch similarity.
- Do not delete historical notes from the remote service.
- Do not add broad rebase refactors unrelated to the manual conflict-resolution
  path.

## Design Summary

Introduce a Git AI owned, worktree-scoped `PendingRebasePick` state. Git AI
captures this state only when a rebase is known to be paused on a conflict.
Subsequent commits consume the pending pick through a one-shot, validated path.

Direct `git commit` must not read `.git/REBASE_HEAD` as an ongoing source of
truth. The Git rebase files are inputs for creating pending state at the moment
the rebase pauses, not for classifying every later commit.

For remote REST notes, add a rewrite API that atomically writes the new note,
marks the old note as superseded, and records a rewrite edge. This prevents the
server from counting both `B` and `D` as active AI-authored commits.

## Pending Rebase Pick State

`PendingRebasePick` is scoped to the repository worktree, not only the common
git directory. It should be protected by the same style of file locking used
for other Git AI repository state so the wrapper and daemon cannot consume it
twice.

Suggested fields:

```text
source_commit: String       # B, the stopped commit being replayed
expected_parent: String     # C, HEAD at the conflict pause
original_head: String       # original local rebase head, often B for one commit
onto_head: Option<String>   # resolved rebase target, usually C for pull --rebase
operation: String           # pull_rebase_conflict | rebase_conflict
status: Pending | Consumed | Aborted | Skipped
created_at_ms: i64
consumed_by: Option<String> # D when consumed
```

The state can be stored under `.git/ai/` or the worktree git dir, but the path
must distinguish linked worktrees. A worktree-local file is preferable because
`REBASE_HEAD` and the rebase directories are worktree-local.

## Creating Pending State

Create a pending pick only when Git AI has evidence that a rebase is paused:

- `pull_post_command_hook` sees `pull --rebase` exit unsuccessfully and
  `rebase-merge` or `rebase-apply` exists.
- `rebase_hooks` sees a failed rebase command while a rebase directory exists.

At that point Git AI may read:

- `.git/REBASE_HEAD`
- `.git/rebase-merge/stopped-sha`
- `.git/rebase-apply/stopped-sha`
- `.git/rebase-merge/onto` or equivalent onto context when available

The read result becomes `source_commit`. The current HEAD becomes
`expected_parent`. The pre-command HEAD or existing `RebaseStart` provides
`original_head`.

If no valid stopped source commit is available, Git AI should not create
pending state.

## Consuming Pending State

On successful non-amend `git commit`, Git AI checks for an unconsumed pending
pick. It consumes the pending pick only when all of these hold:

- `pre_command_base_commit == pending.expected_parent`
- the new commit's first parent is `pending.expected_parent`
- `pending.status == Pending`
- the pending `source_commit` still identifies the stopped pick captured when
  the rebase paused

When consumed, Git AI emits one rewrite event mapping:

```text
source_commits = [pending.source_commit]
new_commits = [new_commit]
original_head = pending.expected_parent
new_head = new_commit
```

The short-term implementation may reuse `CherryPickComplete` because it already
expresses a source commit being replayed as a new commit. The source of that
event must be the consumed pending state, not a direct `REBASE_HEAD` read in
the commit path.

After side effects are applied, mark the pending pick as `Consumed` with
`consumed_by = new_commit`, or delete it after the rewrite has been durably
recorded. Keeping a consumed record is useful for `rebase --continue`
deduplication and debugging.

## Follow-Up Commits

A later ordinary commit `E` must not reuse `B`.

This is guaranteed by two independent checks:

- There is no `Pending` record after `B -> D` is consumed.
- Even if a consumed record remains for audit, `E`'s parent is `D`, not
  `pending.expected_parent = C`.

Directly reading `.git/REBASE_HEAD` during `commit_post_command_hook` or the
daemon's `CommitCreated` handling is explicitly unsafe.

## Rebase Continue After Direct Commit

Git allows `git rebase --continue` after the user has manually committed the
resolution. In that case Git clears the rebase directory and HEAD remains at
`D`.

If the pending pick was already consumed by the direct commit, the later rebase
completion path must not rewrite `B -> D` again. The rebase completion logic
should either:

- detect the consumed pending record and exclude that source/new pair from the
  generated rebase mapping, or
- detect that all stopped picks were already consumed and skip authorship
  rewrite while allowing normal rebase cleanup.

## Abort And Skip

When `git rebase --abort` runs, any matching pending pick should become
`Aborted` or be removed.

When `git rebase --skip` runs, the current pending pick should become `Skipped`
or be removed.

If the rebase directory disappears without a matching successful continuation,
Git AI should clean up pending state instead of carrying it into an unrelated
future rebase.

## Daemon And Wrapper Coordination

Wrapper hooks and daemon semantic event processing must use the same pending
state API:

```text
create_pending_rebase_pick(...)
take_pending_rebase_pick_for_commit(pre_head, new_commit)
mark_pending_rebase_pick_aborted(...)
mark_pending_rebase_pick_skipped(...)
```

`take_pending_rebase_pick_for_commit` must be atomic. If the wrapper consumes
the pending pick, the daemon must observe it as consumed and emit a normal
commit event or no duplicate rewrite side effect.

The daemon must not classify a `CommitCreated` event as a rebase replay by
reading `REBASE_HEAD`, `stopped-sha`, or `rebase-apply/stopped-sha` directly.

## REST Notes Rewrite API

The existing REST notes sync has `list`, `batch`, and `push`. `push` upserts
notes by `commit_sha`, but it does not express that one note supersedes another.
That is insufficient when `B` was already pushed to the server and `D` later
replaces it.

Add:

```text
POST /worker/authorship_notes/rewrite
```

Request:

```json
{
  "repo_url": "https://github.com/org/repo",
  "rewrite_id": "sha256(repo_url + operation + source + target + target_note_blob_oid)",
  "operation": "rebase_conflict_manual_commit",
  "branch": "main",
  "original_head": "B",
  "new_head": "D",
  "mappings": [
    {
      "source_commit": "B",
      "target_commit": "D",
      "source_note_blob_oid": "old-note-blob",
      "target_note_blob_oid": "new-note-blob",
      "target_content": "{...authorship log...}",
      "commit_time": 1710000000,
      "author_name": "User",
      "author_email": "user@example.com",
      "disposition": "supersede_source"
    }
  ]
}
```

Response:

```json
{
  "ok": true,
  "data": {
    "created": 1,
    "updated": 0,
    "superseded": 1,
    "unchanged": 0,
    "conflicts": []
  }
}
```

Server semantics:

1. Validate the request and idempotency key.
2. Write or update the target commit note.
3. Mark each source commit note as `superseded`, not deleted.
4. Store a rewrite edge `source_commit -> target_commit`.
5. Make default metrics and active-note queries ignore superseded source notes.
6. Keep superseded notes available for audit and historical lookup.

Superseded source metadata should include:

```json
{
  "commit_sha": "B",
  "status": "superseded",
  "superseded_by": "D",
  "rewrite_id": "...",
  "superseded_at": 1710000000
}
```

The endpoint must be idempotent by `rewrite_id`. Replaying the same request
should not duplicate metrics or rewrite edges.

Conflict behavior:

- If the target note content hash matches the request, count it as unchanged.
- If the target note differs but the rewrite mapping is the same, allow update
  and count it as updated.
- If the target note differs and the mapping differs, return a conflict item
  rather than silently overwriting.

Example conflict:

```json
{
  "source_commit": "B",
  "target_commit": "D",
  "reason": "target_note_conflict",
  "remote_content_hash": "...",
  "local_content_hash": "..."
}
```

## Client REST Sync Behavior

For normal new notes, keep using `/worker/authorship_notes/push`.

When Git AI applies a rewrite event locally and `notes_store=rest`, send a
rewrite request to `/worker/authorship_notes/rewrite`. This should happen as
part of rewrite side effects, not only as a later best-effort generic push,
because the server needs the source-to-target relationship to suppress the old
active note.

The rewrite API can be used for:

- pending rebase pick consumption (`B -> D`)
- rebase complete mappings
- cherry-pick complete mappings
- amend mappings

Only mappings that actually replace prior active authorship should supersede
the source. Cases that copy authorship while both commits remain meaningful
should use a different disposition or the existing push path.

## Test Plan

Existing regression tests:

- `test_pull_rebase_conflict_after_failed_push_commit_preserves_ai_notes`
  verifies that direct conflict-resolution commit `D` preserves `B`'s AI
  authorship.
- `test_pull_rebase_conflict_manual_commit_does_not_reuse_stale_rebase_head`
  verifies that follow-up commit `E` does not inherit stale `B` prompt metrics.

Additional tests to add with implementation:

- Direct commit `D`, then `git rebase --continue`, does not rewrite or count
  `B -> D` twice.
- `git rebase --abort` clears pending state.
- `git rebase --skip` clears or skips pending state.
- Wrapper and daemon modes do not both consume the same pending pick.
- REST rewrite request supersedes `B` and activates `D` exactly once.
- Replaying the same REST rewrite request is idempotent.
- Conflicting target note content returns a conflict response.

## Migration And Compatibility

Existing servers that only support `/push` can still receive target notes, but
they cannot suppress already-pushed source notes. Clients should feature-detect
or version-gate `/rewrite` and log a warning when falling back to `/push` for a
rewrite.

Local Git notes remain the source of truth for offline operation. REST rewrite
improves server-side active metrics and auditability when notes were already
uploaded before history was rewritten.

## Open Decisions

- Exact on-disk location and serialization format for `PendingRebasePick`.
- Whether consumed pending records are retained for a short audit window or
  immediately deleted after `rebase --continue` cleanup.
- Whether REST rewrite should be one generic endpoint for all rewrite types or
  initially limited to authorship note rewrites.
