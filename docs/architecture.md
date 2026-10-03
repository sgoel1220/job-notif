# Architecture

- **Storage:** PostgreSQL (including Neon) stores normalized job postings and retains each provider's original listing payload in `details_json`. Versioned schema changes live in `migrations/` and run automatically at startup via SQLx migrations. The service requires `DATABASE_URL`; it does not open or modify the legacy `job-notif.db` file. Run `DATABASE_URL=postgres://… cargo test` against a PostgreSQL database for database-backed tests; each test uses its own temporary schema.
- **Sync:** The Rust Axum service runs a direct-source sync on demand (`POST /api/sync`) or when the last successful sync is at least 24 hours old. The sync is registry-driven: enabled companies in `companies.json` determine which direct feeds are queried.
- **Registry:** `companies.json` is the source registry. Schema version `1` records company name, enabled status, provider, board/source configuration, source provenance, and notes. `companies.csv` remains the original company-name seed list, not the runtime sync registry.
- **Direct ATS feeds:** `src/ats.rs` contains Rust adapters for public unauthenticated ATS feeds. Supported ATS providers are Greenhouse, Lever, Ashby, SmartRecruiters, Workable, Recruitee, Personio, BambooHR, and Workday.
- **Additional direct feeds:** The registry also accepts `remote-public` and `atom-feed` entries for non-ATS direct sources already represented by Rust direct-source modules.
- **Retired snapshot:** `src/open_jobs_data.rs` remains as unused legacy code; the running sync never fetches it. Previously imported `open-jobs-data/` rows are retained for history but marked inactive.
- **Reconciliation:** Each direct company/provider feed is treated as its own complete source snapshot. A successful feed upserts seen rows and marks missing rows inactive only inside that source namespace. Failed or invalid feed responses must not replace existing data or mark prior listings inactive. Sources run concurrently with a bounded limit of eight; each source failure is collected independently so remaining sources continue, while the full-sync timestamp advances only if every source succeeds.

## Registry and adapter support

Current bundled registry facts validated from `companies.json` and `src/company_registry.rs`:

- Schema version: `1`.
- Companies in registry: `196`.
- Enabled companies: `17`.
- Enabled provider mix: Ashby `7`, Greenhouse `6`, Lever `2`, Workable `1`, `atom-feed` `1`. Remote.com is disabled because its public endpoint returns HTTP 403; the similarly named Greenhouse `remote` board is unrelated.
- Registry-supported providers: `greenhouse`, `lever`, `ashby`, `smartrecruiters`, `workable`, `recruitee`, `personio`, `bamboohr`, `workday`, `remote-public`, `atom-feed`.

Current generic ATS adapter support validated from `src/ats.rs`:

- Greenhouse: `GET https://boards-api.greenhouse.io/v1/boards/{slug}/jobs?content=true` (includes descriptions).
- Lever: `GET https://api.lever.co/v0/postings/{slug}?mode=json`.
- Ashby: `GET https://api.ashbyhq.com/posting-api/job-board/{slug}`.
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

The direct-feed design keeps source namespaces isolated. The runner retires imported snapshot rows after processing direct sources; failures on individual feeds retain their prior direct-source rows and prevent advancing the full-sync success timestamp. Manual and scheduled syncs share a mutex to avoid concurrent reconciliation.
