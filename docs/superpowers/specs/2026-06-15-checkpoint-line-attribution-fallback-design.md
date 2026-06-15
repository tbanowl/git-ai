# Checkpoint Line Attribution Fallback Design

Date: 2026-06-15

## Problem

Some AI checkpoints can contain useful `line_attributions` while their character-level
`attributions` are empty. Later checkpoint processing builds the previous file state
from only `blob_sha` and `attributions`. When a following Human checkpoint edits the
file, it sees no prior character-level ownership and fills all unattributed ranges as
`human`. This can cause previously AI-authored lines to be recorded as human-authored.

The observed `LotHoldListDlg.cpp` checkpoint sequence shows this failure mode:

- Earlier AI checkpoint entries had AI `line_attributions` but empty `attributions`.
- Later Human checkpoint entries inherited only empty `attributions`.
- A later Human checkpoint filled the whole previous file state as `human`.

## Scope

This change fixes future checkpoint processing and makes it compatible with old working
log entries that still have AI `line_attributions`. It does not automatically rewrite
already-polluted final checkpoint entries where the latest entry has already been
materialized as all-human attribution.

## Goals

- Preserve AI attribution through later Human checkpoints when the previous checkpoint
  only has line-level attribution.
- Keep the fix local to checkpoint state reconstruction.
- Avoid rewriting existing checkpoint logs as part of normal checkpoint or commit flow.
- Add regression coverage for the line-level fallback path.

## Non-Goals

- Do not add a repair command for already-polluted checkpoint logs.
- Do not change the authorship note schema.
- Do not change how pure-human line attributions are filtered from line-level output.

## Proposed Approach

Update `src/commands/checkpoint.rs` so previous checkpoint state can carry both
character-level and line-level attribution.

`PreviousFileState` will store:

- `blob_sha`
- `attributions`
- `line_attributions`

`build_previous_file_state_maps()` will continue selecting the latest entry per file,
but it will copy both `entry.attributions` and `entry.line_attributions`.

When `get_checkpoint_entry_for_file()` reconstructs `previous_content` from the stored
blob, it will derive `prev_attributions` with this precedence:

1. Use `state.attributions` when it is non-empty.
2. Otherwise, if `state.line_attributions` is non-empty, convert those line ranges to
   character ranges with `line_attributions_to_attributions()` using `previous_content`.
3. Otherwise, keep the existing empty attribution behavior.

This mirrors the fallback pattern already used in the post-commit virtual attribution
path, but applies it earlier, before Human checkpoint processing can mark the entire
file as human.

## Data Flow

1. AI checkpoint writes an entry with `line_attributions` and possibly empty
   `attributions`.
2. A later checkpoint reads previous checkpoints.
3. The previous state map preserves the prior entry's line-level attribution.
4. If character-level attribution is missing, checkpoint processing reconstructs it
   from the stored blob content and line-level attribution.
5. Human checkpoint diff processing updates only the human-edited ranges while leaving
   unchanged AI ranges attributed to AI.

## Error Handling

The fallback should be deterministic and non-fatal:

- Empty content or empty line attribution yields an empty attribution list, preserving
  existing behavior.
- Invalid or out-of-range line ranges should follow the existing behavior of
  `line_attributions_to_attributions()`.
- If character-level attribution already exists, the fallback must not run or override
  it.

## Testing

Add a regression test that reproduces the failure:

1. Create a repo and file.
2. Inject or produce an AI checkpoint entry for the file where:
   - `attributions` is empty
   - `line_attributions` covers AI-owned lines
3. Make a Human checkpoint that edits a small part of the file.
4. Assert the generated Human checkpoint entry contains non-human character attribution
   for the preserved AI ranges.
5. Commit and assert line-level blame/authorship still marks unchanged AI lines as AI
   while human-edited lines are human.

The test should use explicit file writes and explicit checkpoint calls or controlled
working-log setup rather than `TestFile::set_contents`, because this scenario depends
on exact checkpoint ordering and attribution shape.

## Compatibility

Existing checkpoint entries remain readable. The added `PreviousFileState` field is
internal and does not change serialized checkpoint format.

The change should be backward-compatible with old working logs where entries have:

- both `attributions` and `line_attributions`
- only `attributions`
- only `line_attributions`
- neither attribution field populated

## Risks

- Line-level attribution is less precise than character-level attribution. Converting it
  to character ranges may over-attribute whole lines when prior data only had line
  granularity. This is still better than losing the AI attribution entirely, and it only
  applies when character-level attribution is unavailable.
- Tests that assert exact checkpoint JSON may need snapshot updates if they inspect new
  Human checkpoint character attribution produced by the fallback.

## Acceptance Criteria

- A Human checkpoint after an AI line-only checkpoint no longer turns the entire file
  into `human` attribution.
- Existing checkpoint flows with non-empty character-level attribution keep current
  behavior.
- Existing pure-human checkpoint flows keep current behavior.
- Regression tests cover checkpoint entry attribution and final committed authorship.
