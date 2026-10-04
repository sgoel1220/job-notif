# Description-only backfill

Run `DATABASE_URL=... cargo run -- --backfill-descriptions [--dry-run] [--company NAME]`.

Targets active Workday and SmartRecruiters postings in enabled `companies.json` entries when either `description` or `description_text` is blank. The backfill fetches each posting directly by its stored `source_job_id`: Workday uses the configured CXS base URL plus the stored `externalPath`; SmartRecruiters uses the configured company slug and posting ID. It never enumerates or paginates a company feed. Canonical ATS source keys and the exact legacy provider key are supported; source keys are never matched by prefix.

At most eight detail requests run concurrently. Each request has a 15-second timeout and up to three attempts for transient failures. Each successful lookup updates only blank description fields for the exact active `(company, source, source_job_id)` row and commits independently, so a later timeout cannot discard earlier repairs. No rows are inserted, deactivated, or otherwise synchronized, and timestamps or other posting fields are not changed. Progress is reported per posting; failed requests are reported and cause a nonzero exit.

A dry run reports how many active jobs are targeted and their exact identities; it makes no provider requests and performs no writes. This mode does not run migrations, start the web server/scheduler, or touch `sync_state`. Use a test `DATABASE_URL` for validation; never use production credentials for tests.
