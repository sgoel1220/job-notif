# Company research tracking

`companies.json` remains the runtime registry. `companies.csv` is the target-company research ledger, not a runtime configuration source. Preserve `company_name` and synchronize `provider`, `board`, and `coverage_status` with the registry when merging results.

## CSV fields

- `coverage_status`: `enabled` or `unresolved`; describes configuration, not live source health.
- `research_status`: `in_progress`, `verified_live`, `valid_empty`, `enabled_not_rechecked`, `unsupported_provider`, `custom_careers`, `inactive_or_acquired`, `blocked`, `identity_conflict`, or `unresolved`.
- `research_owner`, `research_run`: exclusive worker claim and unique run identity. One company has exactly one row/owner per run.
- `last_checked`: actual evidence check date/time; never infer freshness from source attribution.
- `provider`, `board`: active registry mapping only. Candidate unsupported/conflicting mappings go in findings, not active columns.
- `root_cause`: observed failure or missing evidence; distinguish unknown from proven unsupported.
- `evidence_url`: actual official careers/hosted board/failing source URL supporting the result. If no current authoritative URL is established, leave it blank and explicitly document the missing source in `root_cause` and `feed_evidence`; never fabricate a citation.
- `careers_url`, `feed_url`, `identity_evidence`, `feed_evidence`: preserve the exact inspected source, candidate endpoint, employer proof, and observed payload/failure. A candidate feed URL may remain recorded on a disabled row; it is not an active mapping.
- `next_action`: concrete follow-up; do not repeat previous guesses without new evidence.
- `job_count`: point-in-time verified feed count, not filtered job count. Blank means unmeasured.

## Avoid duplicate research

1. Read the CSV and prior coverage report before assigning work. Skip enabled sources unless deliberately auditing freshness.
2. Claim unresolved rows **before** dispatch: set `in_progress`, one owner, and a unique run ID. Exclude rows already claimed by another active run.
3. Assign mutually exclusive lists of company names. Workers return per-company results in separate files; they never concurrently write the CSV/registry.
4. The parent validates exact assigned/result name sets, unique names, allowed statuses, identity evidence, and adapter-compatible feed responses. Reject stale results whose CSV owner/run no longer matches.
5. Merge registry and CSV together in one parent-controlled pass. Record every attempted company, including failures, to avoid repeating unsuccessful guesses. Keep original company names, and preserve existing validated mappings.
6. Do not retry a failed/inconclusive row blindly. Follow its `next_action`; retry blocked pages only when access conditions change or an alternative official source is found.
7. If a worker is cancelled/fails, release its claims to `unresolved` with a concrete interrupted-run note. An `in_progress` claim is not evidence of completed research and must not remain silently orphaned.

## Evidence gate

An enabled source needs an authoritative employer identity and a valid provider payload. A guessed slug returning HTTP 200 or generic empty Ashby JSON is insufficient. A genuinely empty board can be verified only through positive official linkage and a valid empty payload. Workday is supported and requires the actual `tenant/shard/site`, POST CXS validation, and existing pagination logic. Multiple branded live feeds require a current official application link to establish the authoritative board; otherwise retain an explicit conflict.

Research status does not guarantee ongoing availability. Recheck source health separately. The registry is compiled via `include_str!`; production changes require rebuild/deploy and authenticated sync.

## Tests

`cargo test --test company_registry` parses the CSV with the established `csv` crate and verifies unique company rows, registry agreement, exclusive claims, evidence, and next actions. Full Rust tests require an isolated PostgreSQL database; never use production for test migrations.
