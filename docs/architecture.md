# Architecture

- **Storage:** PostgreSQL (including Neon) stores normalized job postings and retains each provider's original listing payload in `details_json`. Versioned schema changes live in `migrations/` and run automatically at startup via SQLx migrations. The service requires `DATABASE_URL`; it does not open or modify the legacy `job-notif.db` file. Run `DATABASE_URL=postgres://… cargo test` against a PostgreSQL database for database-backed tests; each test uses its own temporary schema.
- **Sync:** The Rust Axum service runs a direct-source sync on demand (`POST /api/sync`) or when the last successful sync is at least 24 hours old. Manual calls require `Authorization: Bearer <SYNC_BEARER_TOKEN>`; missing/blank server configuration disables manual sync. Busy calls are rejected instead of queued, and accepted manual requests have a 30-second cooldown. Scheduled sync is independent of manual authentication. The sync is registry-driven: enabled companies in `companies.json` determine which direct feeds are queried.
- **Registry:** `companies.json` is the source registry. Schema version `1` records company name, enabled status, provider, board/source configuration, source provenance, and notes. `companies.csv` remains the original company-name seed list, not the runtime sync registry.
- **Direct ATS feeds:** `src/ats.rs` defines shared ATS types and the adapter facade; `src/ats/` contains one module per provider plus HTTP/dispatch, shared helpers, normalization, and grouped regression tests. Supported ATS providers are Greenhouse, Lever, Ashby, SmartRecruiters, Workable, Recruitee, Personio, BambooHR, and Workday.
- **Additional direct feeds:** The registry also accepts `remote-public` and `atom-feed` entries for non-ATS direct sources already represented by Rust direct-source modules.
- **Legacy snapshot:** The running sync never fetches the retired aggregate source. Previously imported `open-jobs-data/` rows remain active as fallback until the corresponding company's complete direct snapshot succeeds; failed or unresolved replacements do not retire fallback rows.
- **Runtime modules:** `src/main.rs` only boots the application. `src/state.rs` owns shared state. `src/sync/` separates scheduling, manual authorization, and source execution/reconciliation. `src/job_postings/` separates API pagination, query filters, metadata normalization, location handling, and transactional persistence. Oversized test suites are grouped by responsibility under the corresponding module directories.
- **Reconciliation:** Each direct company/provider feed is treated as its own complete source snapshot. A successful feed upserts seen rows and marks missing rows inactive only inside that source namespace. Failed or invalid feed responses must not replace existing data or mark prior listings inactive. Sources run concurrently with a bounded limit of eight; each source failure is collected independently so remaining sources continue, while the full-sync timestamp advances only if every source succeeds. ATS fetches have a five-minute company-level deadline, and complete manual/scheduled sync runs have a fifteen-minute deadline; timeouts release the sync lock and never reconcile an incomplete fetch.

## Registry and adapter support

Current bundled registry facts validated from `companies.json` and `src/company_registry.rs`:

- Schema version: `1`.
- Companies in registry: `274`.
- Enabled companies: `118`.
- Enabled provider mix: Ashby `25`, Greenhouse `49`, Lever `6`, Workday `29`, SmartRecruiters `6`, Workable `1`, `atom-feed` `1`, `remote-public` `1`. Remote.com's official endpoint was reverified HTTP 200 with 14 open, accepting jobs on 2026-10-03 and reenabled; the similarly named Greenhouse `remote` board is unrelated.
- Registry-supported providers: `greenhouse`, `lever`, `ashby`, `smartrecruiters`, `workable`, `recruitee`, `personio`, `bamboohr`, `workday`, `remote-public`, `atom-feed`.

Current generic ATS adapter support validated from `src/ats.rs`:

- Greenhouse: `GET https://boards-api.greenhouse.io/v1/boards/{slug}/jobs?content=true` (includes descriptions).
- Lever: `GET https://api.lever.co/v0/postings/{slug}?mode=json`.
- Ashby: `GET https://api.ashbyhq.com/posting-api/job-board/{slug}?includeCompensation=true`.
- SmartRecruiters: paginated `GET https://api.smartrecruiters.com/v1/companies/{slug}/postings`.
- Workable: `GET https://apply.workable.com/api/v1/widget/accounts/{slug}?details=false`.
- Recruitee: `GET https://{slug}.recruitee.com/api/offers/`.
- Personio: `GET https://{slug}.jobs.personio.de/xml`.
- BambooHR: `GET https://{slug}.bamboohr.com/careers/list`.
- Workday: paginated `POST https://{tenant}.{shard}.myworkdayjobs.com/wday/cxs/{tenant}/{site}/jobs` using tenant/site/shard config.

## Normalization contract

Direct adapters map provider payloads into `JobPosting` with:

- source namespace `ats/{provider}/{source_ref}` for generic ATS adapters;
- stable provider job ID when available, falling back only where adapter-specific logic defines it;
- normalized title, company, URL, location, workplace type, department, employment type, and posted timestamp when supplied;
- raw provider object retained in `details_json`.

The direct-feed design keeps source namespaces isolated. The runner retires company-scoped aggregate fallback rows only after successful direct replacement; failed feeds retain prior listings and prevent advancing the full-sync success timestamp. Manual and scheduled syncs share a mutex to avoid concurrent reconciliation; both persist the successful-sync timestamp before releasing that mutex. Aggregate fallback retirement is transactional. SmartRecruiters/Workday pagination must be complete: safety caps or malformed/incomplete pages fail the source instead of reconciling a partial snapshot.

## Listing filters and verification

The Rust homepage is `templates/jobs.html`; `web/` is a separate saved-job tracker frontend. Listing filters are applied on the server over all active jobs, not just the current page. The legacy `GET /api/listings` also uses server-side pagination: `page` defaults to `1`, `page_size` defaults to `25` and is capped at `100`. For example, `/api/listings?page=2&page_size=25` returns the next page in stable newest-first order. Its response remains a JSON array; an empty array indicates no rows on that page. Fetch subsequent pages explicitly if needed—requests no longer return the entire snapshot. The homepage uses fixed role (`Software intern`, `SDE-1`, `SDE-2`, `Others`) and country (`USA`, `India`) dropdowns, with an All option for each. API parameters are `role_category=intern|sde-1|sde-2|software-engineering|other` and `country=US|IN`; unsupported values return HTTP 400. PostgreSQL stored generated columns (`role_category`, `country_codes`) map every existing and future job automatically, including on title/location/employment updates. Migration `0004_job_categories.sql` backfills all rows and adds active-row category indexes. Multiple explicit countries produce multiple country codes; unknown/global locations produce an empty array. Software intern mapping requires a software-engineering title plus whole-word intern/interns/internship/internships in the title or canonical internship employment metadata, never an `intern*` prefix. SDE mapping requires software-engineer/developer or SDE titles with explicit I/1 or II/2 levels; junior/entry-level/new-grad maps to SDE-1 and mid-level to SDE-2. Senior/staff/lead/manager titles and unspecified software-engineer levels remain `other`, not guessed. Country mapping recognizes explicit country names/codes and curated city aliases (see the migration); it is a location label, not proof of hiring eligibility. The UI displays each job's role category. The legacy free-text API parameters remain supported: title search is title-only and matches all entered tokens, with boundary-safe intern-family matching; India searches also recognize supported Indian city names. Worldwide and unspecified-workplace inclusion are explicit opt-ins: these are potential matches, not confirmed hiring eligibility. Unknown arrangement is never silently classified as remote. Explicit hybrid labels take precedence over remote wording, and malformed raw metadata does not erase an already normalized location. `include_global=true` and `include_unknown_workplace=true` widen the corresponding filters only when requested; neither guarantees eligibility.

Migration `0005_software_engineering.sql` adds indexed stored generated `is_software_engineering`, based on explicit software/SDE, frontend/backend/full-stack, mobile/iOS/Android, and embedded engineering/developer title phrases. The API-only `software-engineering` filter remains available across all seniority levels, including software internships. Migration `0006_software_intern_categories.sql` refines the stored classifier so non-software internships become `other`, explicitly recalculating affected existing rows. The homepage offers only Software intern / SDE-1 / SDE-2 / Others (plus All); `other` includes every job outside the three named categories, including unspecified/senior software roles and non-software internships. Stored categories and displayed labels agree.

Salary amounts retain their original currency and pay interval. `salary_interval` is stored and returned as `hour`, `day`, `week`, `month`, or `year` when known and is shown beside the amount. Numeric salary filters require both `currency` and a supported `salary_interval` (otherwise HTTP 400); missing intervals are excluded rather than assumed annual. Migration `0003_salary_interval.sql` adds the nullable column; historical rows receive intervals on their next successful source sync. No amount conversion or historical guesswork is performed.

Bearer comparison uses the established BSD-3-Clause `subtle` crate (already a transitive dependency), now declared directly instead of a hand-written comparison.

Run Rust regressions against an isolated PostgreSQL database: `DATABASE_URL=postgres://… cargo test --locked --all-targets`. Do not use the live Neon database for tests. The database test helper requires an explicitly supplied `DATABASE_URL` and never loads the project's `.env` as a fallback. Browser regressions: `python3 tests/listings_ui.py` (requires Playwright and Chromium); they mock HTTP responses and test page shrink/refetch, inclusive-filter reset, stale-response rejection, and unsafe/missing job URLs without launching a sync. The homepage only renders HTTP(S) job links and checks request identity before applying results, even when a response arrives after its request was aborted.
