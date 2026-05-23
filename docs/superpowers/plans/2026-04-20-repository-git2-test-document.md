# `git_repository_comprehensive.rs` — 47-Test Document

**Date:** 2026-04-20
**Status:** Draft
**Target File:** `tests/integration/git_repository_comprehensive.rs`
**Related Requirements:** `docs/superpowers/specs/2026-04-20-repository-git2-migration-requirements.md`
**Related Checklist:** `docs/superpowers/plans/2026-04-20-repository-git2-migration-checklist.md`

## Purpose

This document records the full first-phase test inventory for the
`src/git/repository.rs` Git CLI → `git2` migration. These tests are intended to
be added to `tests/integration/git_repository_comprehensive.rs` under the new
repository-focused sections.

## Section A — Commit Metadata Tests (12)

1. `test_commit_summary_for_single_line_message`
2. `test_commit_summary_and_body_for_multiline_message`
3. `test_commit_body_is_empty_when_commit_has_no_body`
4. `test_commit_author_and_committer_match_default_identity`
5. `test_commit_author_and_committer_can_differ`
6. `test_commit_time_uses_committer_time`
7. `test_root_commit_has_no_parents`
8. `test_commit_parent_zero_returns_first_parent`
9. `test_commit_parent_out_of_bounds_errors`
10. `test_merge_commit_parent_count_and_order_are_stable`
11. `test_commit_tree_matches_head_tree_oid`
12. `test_commit_metadata_supports_non_ascii_message_and_author`

## Section B — Revparse and Reference Resolution Tests (19)

13. `test_revparse_single_resolves_head`
14. `test_revparse_single_resolves_full_commit_oid`
15. `test_revparse_single_resolves_branch_name`
16. `test_revparse_single_resolves_fully_qualified_refname`
17. `test_revparse_single_errors_for_invalid_spec`
18. `test_object_peel_to_commit_from_commit_oid`
19. `test_reference_peel_to_commit_from_annotated_tag`
20. `test_reference_peel_to_commit_from_lightweight_tag`
21. `test_reference_peel_to_blob_from_blob_spec`
22. `test_reference_peel_to_commit_errors_for_non_commitish_reference`
23. `test_reference_shorthand_matches_expected_branch_name`
24. `test_reference_target_returns_expected_oid`
25. `test_head_returns_symbolic_branch_ref_when_attached`
26. `test_head_returns_head_when_detached`
27. `test_find_reference_finds_existing_branch`
28. `test_find_reference_finds_existing_tag`
29. `test_find_reference_errors_for_missing_ref`
30. `test_references_lists_heads_and_tags`
31. `test_references_include_fully_qualified_refnames`

## Section C — Commit Graph and Range Tests (16)

32. `test_merge_base_returns_common_ancestor_for_diverged_branches`
33. `test_merge_base_errors_when_commits_are_invalid`
34. `test_commit_range_length_for_linear_history`
35. `test_commit_range_length_is_zero_for_adjacent_empty_range`
36. `test_commit_range_into_iter_returns_expected_commits_in_current_order`
37. `test_commit_range_into_iter_handles_single_commit_range`
38. `test_commit_range_into_iter_returns_empty_for_empty_range`
39. `test_commit_range_is_valid_when_start_is_ancestor_of_end`
40. `test_commit_range_is_invalid_when_start_is_not_ancestor_of_end`
41. `test_commit_range_is_invalid_when_start_is_not_reachable_from_refname`
42. `test_commit_range_is_invalid_when_end_is_not_reachable_from_refname`
43. `test_commit_range_allows_empty_tree_hash_as_start`
44. `test_parent_on_refname_selects_parent_reachable_from_target_branch`
45. `test_parent_on_refname_accepts_short_branch_name`
46. `test_parent_on_refname_accepts_fully_qualified_refname`
47. `test_parent_on_refname_errors_when_no_parent_is_reachable_from_ref`

## Section D — Object Access and Tree/Blob Content Tests (19)

48. `test_object_type_reports_commit_blob_and_tree`
49. `test_object_type_errors_for_missing_oid`
50. `test_find_commit_returns_commit_for_commit_oid`
51. `test_find_blob_returns_blob_for_blob_oid`
52. `test_find_tree_returns_tree_for_tree_oid`
53. `test_find_commit_errors_for_non_commit_oid`
54. `test_find_blob_errors_for_non_blob_oid`
55. `test_find_tree_errors_for_non_tree_oid`
56. `test_blob_content_returns_exact_text_bytes`
57. `test_blob_content_returns_exact_binary_bytes`
58. `test_get_file_content_reads_file_from_commit_root`
59. `test_get_file_content_reads_file_from_nested_path`
60. `test_get_file_content_errors_for_missing_path`
61. `test_get_file_content_errors_when_path_is_directory_like`
62. `test_tree_get_path_returns_expected_entry_for_root_file`
63. `test_tree_get_path_returns_expected_entry_for_nested_file`
64. `test_tree_get_path_errors_for_missing_path`
65. `test_get_file_content_supports_non_ascii_paths`
66. `test_tree_get_path_supports_paths_with_spaces`

## Section E — Migration Guard Tests (6)

67. `test_detached_head_behavior_matches_current_repository_contract`
68. `test_commit_range_iteration_order_matches_current_repository_contract`
69. `test_merge_commit_parent_order_matches_current_repository_contract`
70. `test_annotated_tag_peeling_matches_current_repository_contract`
71. `test_summary_and_body_parsing_matches_current_repository_contract`
72. `test_tree_path_lookup_matches_current_repository_contract_for_nested_paths`

## Actual Count Note

The current full inventory contains **72 tests**, not 47.

Reason:

- 12 commit metadata tests
- 19 revparse/reference tests
- 16 commit graph/range tests
- 19 object/tree/blob content tests
- 6 migration guard tests

If a reduced **47-test subset** is desired later, it should be explicitly carved
out as a minimal-safety suite. This document records the current **full
coverage** inventory that was defined during requirements planning.

## Recommended File Structure

Add the tests to `tests/integration/git_repository_comprehensive.rs` in this
order:

1. `Commit Metadata Tests`
2. `Revparse and Reference Resolution Tests`
3. `Commit Graph and Range Tests`
4. `Object Access and Tree/Blob Content Tests`
5. `Migration Guard Tests`

## Verification Command

```bash
cargo test --package git-ai --test git_repository_comprehensive -- --nocapture
```
