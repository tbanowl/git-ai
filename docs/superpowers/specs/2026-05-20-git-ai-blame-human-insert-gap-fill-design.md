# Human Insert Gap-Fill Repair Design

## Problem

`git-ai blame` can attribute a human-inserted line to AI when the user does the following in a single commit cycle:

1. AI generates a contiguous block of code.
2. A human edits that same block by inserting or replacing content in the middle.
3. The user stages and commits once.
4. `git-ai blame` reports the inserted human line as AI.

This is not a blame-display bug. The evidence from the failing regression test shows the authorship note itself already contains the wrong AI range. The issue is most likely in the post-commit authorship generation gap-fill logic.

## Goals

- Prevent human-inserted lines inside an AI-generated block from being written into the authorship note as AI.
- Keep existing AI attribution behavior for normal contiguous AI blocks.
- Keep the fix conservative: minimize changes to unrelated attribution behavior.
- Cover adjacent scenarios that share the same failure mode:
  - human insertion in the middle of an AI block
  - human replacement of an AI line
  - human insertion at the edge of an AI block

## Non-Goals

- Do not change `git-ai blame` output formatting or CLI behavior.
- Do not redesign checkpointing or the working-log architecture.
- Do not attempt to reclassify every edge case in attribution history.
- Do not treat blame rendering as the root cause.

## Current Evidence

The newly added regression test reproduces the bug in a single commit cycle:

- base commit
- AI checkpoint for a contiguous block
- human checkpoint after inserting a line in the middle
- single commit

The test fails because line 3, `Human inserted line`, is blamed as `mock_ai`.
The printed authorship note shows the committed AI range already includes lines 2-5, which confirms the bug is introduced before blame reads the note.

## Proposed Approach

Use a conservative fix in the authorship-note generation path:

1. Keep the existing checkpoint and blame pipeline intact.
2. Tighten the gap-fill rule in `src/authorship/virtual_attribution.rs`.
3. Only fill a missing committed line as AI when the surrounding evidence strongly indicates an AI-generated gap.
4. Do not fill gaps when the missing content is more consistent with a human insertion or replacement.

### Why this approach

The current gap-fill logic was added to preserve AI attribution when diff matching leaves holes in a contiguous AI region. That same logic can also swallow a human-inserted line if it sits between two lines that are both attributed to the same AI author. A conservative filter is the smallest change that addresses the bug while preserving existing AI attribution behavior.

## Scope of the Fix

The fix should apply to the following related cases:

1. Human inserts a line in the middle of an AI block.
2. Human replaces one AI line with human text.
3. Human inserts a line at the beginning or end of an AI block.
4. Normal AI blocks with no human interruption remain attributed to AI.

## Suggested Rule Shape

The exact implementation can evolve, but the rule should be conservative:

- Gap-fill may only apply when the missing line is clearly an AI-preserving hole.
- If the missing line is plausibly a human insertion, leave it human.
- If the gap is ambiguous, prefer human attribution.

This preserves current AI attribution where confidence is high, but blocks the specific false positive family.

## Acceptance Criteria

The fix is complete when all of the following are true:

1. The new regression test passes.
   - The inserted human line is blamed as human.
2. Adjacent scenarios still pass.
   - Human replacement of an AI line stays human.
   - AI-only contiguous blocks remain AI.
3. The authorship note no longer writes the inserted human line into an AI attestation range.
4. Existing blame output formatting remains unchanged.
5. Related attribution tests for AI addition, formatting, rebase, squash, and amend still pass.

## Test Plan

Keep the new regression test in `tests/integration/simple_additions.rs`.

Required coverage:

- AI generates a block, human inserts a line in the middle, commit once, blame must show the inserted line as human.
- Human replaces an AI line, blame must show the replaced line as human.
- Human inserts at the edge of an AI block, blame must show the inserted line as human.
- Pure AI block remains AI.

The test should keep using explicit `fs::write` and checkpoint calls so the checkpoint order is fully controlled.

## Risks

- A stricter gap-fill rule may cause some borderline gaps to remain human instead of AI.
- Formatting-only or whitespace-only holes may need special handling to avoid regressions.
- Rebase, squash, and amend flows rely on the same authorship-note generation path, so they need regression coverage after the change.

## Files Likely Involved

- `src/authorship/virtual_attribution.rs`
- `src/authorship/attribution_tracker.rs`
- `src/authorship/authorship_log_serialization.rs`
- `tests/integration/simple_additions.rs`

## Summary

This is a conservative repair of the authorship-note generation path. The intended result is simple: if a human inserts content inside an AI block and commits once, `git-ai blame` must not rewrite that human insertion back to AI.
