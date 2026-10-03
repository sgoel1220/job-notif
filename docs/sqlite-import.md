# One-time SQLite to PostgreSQL import

The utility copies `greetings`, `background_jobs`, `sync_state`, and `job_postings`, preserving IDs and values. JSON remains the original `TEXT` representation; timestamps remain text where the application schema uses text. Nullable `sync_state.last_successful_sync` is cast to PostgreSQL `TIMESTAMPTZ`. SQLite booleans are transferred as PostgreSQL booleans. Inserts are batched in one PostgreSQL transaction; failures roll back the import. Identity sequences are advanced after copying.

1. Back up both databases and confirm PostgreSQL is the intended destination. The importer applies pending SQLx migrations before copying data.
2. Set `DATABASE_URL` securely in your shell/environment; do not put credentials in command-line arguments or logs.
3. Run (from the repository root):

```sh
DATABASE_URL='postgres://…' cargo run --features sqlite-import --bin import_sqlite -- --db ./job-notif.db
```

The program opens SQLite read-only. It refuses if any target table has rows. `--force` explicitly permits appending; it does not truncate/replace rows, so duplicate IDs or unique keys will fail and roll back:

```sh
DATABASE_URL='postgres://…' cargo run --features sqlite-import --bin import_sqlite -- --db ./job-notif.db --force
```

Do not run against a live application database: table locks prevent concurrent writes during the transaction. The source database is never modified. No import is performed by building or testing this utility.
