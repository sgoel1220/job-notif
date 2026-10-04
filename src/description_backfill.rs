use crate::{ats, company_registry};
use futures_util::{stream, StreamExt};
use sqlx::PgPool;
use std::{collections::BTreeSet, str::FromStr, time::Duration};

const MAX_CONCURRENCY: usize = 8;

#[derive(Clone, Debug)]
struct Target {
    company: String,
    source: String,
    source_job_id: String,
    config: ats::AtsCompanySource,
}

pub(crate) async fn run(
    db: &PgPool,
    company_scope: Option<&str>,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let registry = company_registry::load_registry()?;
    registry
        .validate()
        .map_err(|e| format!("invalid company registry: {e:?}"))?;

    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT company, source, source_job_id FROM job_postings WHERE is_active=TRUE AND (NULLIF(BTRIM(description),'') IS NULL OR NULLIF(BTRIM(description_text),'') IS NULL) ORDER BY company, source, source_job_id",
    )
    .fetch_all(db)
    .await?;

    let mut targets = Vec::new();
    for (company, source, source_job_id) in rows {
        if company_scope.is_some_and(|scope| !company.eq_ignore_ascii_case(scope)) {
            continue;
        }
        let Some(entry) = registry.enabled_companies().find(|entry| {
            entry.name.eq_ignore_ascii_case(&company)
                && matches!(
                    entry.provider.as_deref(),
                    Some("workday" | "smartrecruiters")
                )
        }) else {
            continue;
        };
        let provider = AtsProviderName::from_entry(entry.provider.as_deref().unwrap())?;
        let config = config_for(entry, provider)?;
        let canonical = config.identity()?.source_key();
        // Accept only the exact canonical key or exact legacy provider key.
        if source == canonical || source == provider.as_str() {
            targets.push(Target {
                company,
                source,
                source_job_id,
                config,
            });
        }
    }

    let total = targets.len();
    let scopes: BTreeSet<(String, String)> = targets
        .iter()
        .map(|target| (target.company.clone(), target.source.clone()))
        .collect();
    println!(
        "description backfill: {total} active jobs targeted{}",
        if dry_run {
            " (dry-run; no provider requests or writes)"
        } else {
            ""
        }
    );
    if dry_run || targets.is_empty() {
        return Ok(());
    }

    let client = ats::http_client_with_timeout(Duration::from_secs(15))?;
    let mut completed = 0usize;
    let mut filled = 0u64;
    let mut failed = 0usize;
    let mut stream = stream::iter(targets.into_iter().map(|target| {
        let client = client.clone();
        async move {
            let result =
                ats::fetch_job_description(&client, &target.config, &target.source_job_id).await;
            (target, result)
        }
    }))
    .buffer_unordered(MAX_CONCURRENCY);

    while let Some((target, result)) = stream.next().await {
        completed += 1;
        match result {
            Ok((description, description_text)) => {
                let updated = fill_missing_description(
                    db,
                    &target.company,
                    &target.source,
                    &target.source_job_id,
                    description.as_deref(),
                    description_text.as_deref(),
                )
                .await?;
                filled += updated;
                println!(
                    "progress {completed}/{total}: {} {} — {} row(s) updated",
                    target.company, target.source_job_id, updated
                );
            }
            Err(error) => {
                failed += 1;
                eprintln!(
                    "progress {completed}/{total}: {} {} — detail fetch failed: {error}",
                    target.company, target.source_job_id
                );
            }
        }
    }

    let mut remaining = 0i64;
    for (company, source) in scopes {
        remaining += incomplete_count(db, &company, &source).await?;
    }
    println!("description backfill finished: {completed}/{total} processed, {filled} rows updated, {failed} fetch failures, {remaining} targeted active rows remain incomplete");
    if failed > 0 || remaining > 0 {
        return Err(format!("description backfill incomplete: {failed} detail-fetch failure(s), {remaining} targeted row(s) remain incomplete").into());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AtsProviderName {
    Workday,
    SmartRecruiters,
}
impl AtsProviderName {
    fn from_entry(value: &str) -> Result<Self, Box<dyn std::error::Error>> {
        match ats::AtsProvider::from_str(value)? {
            ats::AtsProvider::Workday => Ok(Self::Workday),
            ats::AtsProvider::SmartRecruiters => Ok(Self::SmartRecruiters),
            _ => Err(format!("unsupported backfill provider {value}").into()),
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::Workday => "workday",
            Self::SmartRecruiters => "smartrecruiters",
        }
    }
}

fn config_for(
    entry: &company_registry::CompanyEntry,
    provider: AtsProviderName,
) -> Result<ats::AtsCompanySource, Box<dyn std::error::Error>> {
    let board = entry.board.as_deref().unwrap_or_default().trim();
    let (slug, workday) = if provider == AtsProviderName::Workday {
        let parts: Vec<_> = board.split('/').filter(|part| !part.is_empty()).collect();
        if parts.len() != 3 {
            return Err(format!("{}: invalid Workday board", entry.name).into());
        }
        (
            None,
            Some(ats::WorkdayConfig {
                tenant: parts[0].into(),
                shard: parts[1].into(),
                site: parts[2].into(),
            }),
        )
    } else {
        (Some(board.to_owned()), None)
    };
    Ok(ats::AtsCompanySource {
        company_id: None,
        company_name: entry.name.trim().into(),
        provider: ats::AtsProvider::from_str(provider.as_str())?,
        slug,
        workday,
    })
}

async fn incomplete_count(db: &PgPool, company: &str, source: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM job_postings WHERE company=$1 AND source=$2 AND is_active=TRUE AND (NULLIF(BTRIM(description),'') IS NULL OR NULLIF(BTRIM(description_text),'') IS NULL)")
        .bind(company)
        .bind(source)
        .fetch_one(db)
        .await
}

async fn fill_missing_description(
    db: &PgPool,
    company: &str,
    source: &str,
    source_job_id: &str,
    description: Option<&str>,
    description_text: Option<&str>,
) -> Result<u64, sqlx::Error> {
    if description.is_none_or(|value| value.trim().is_empty())
        && description_text.is_none_or(|value| value.trim().is_empty())
    {
        return Ok(0);
    }
    // One independently committed row per successful detail lookup. Never touches any
    // non-description field, and rechecks the exact active identity at write time.
    let result = sqlx::query(
        "UPDATE job_postings SET description=COALESCE(NULLIF(BTRIM(description),''),NULLIF(BTRIM($1),'')), description_text=COALESCE(NULLIF(BTRIM(description_text),''),NULLIF(BTRIM($2),'')) WHERE company=$3 AND source=$4 AND source_job_id=$5 AND is_active=TRUE AND (NULLIF(BTRIM(description),'') IS NULL OR NULLIF(BTRIM(description_text),'') IS NULL)",
    )
    .bind(description)
    .bind(description_text)
    .bind(company)
    .bind(source)
    .bind(source_job_id)
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn insert(
        db: &PgPool,
        company: &str,
        source: &str,
        id: &str,
        desc: Option<&str>,
        text: Option<&str>,
        active: bool,
    ) {
        sqlx::query("INSERT INTO job_postings (source,source_job_id,title,company,description,description_text,url,details_json,is_active) VALUES ($1,$2,'keep title',$3,$4,$5,'https://keep.test','{\"keep\":true}',$6)")
            .bind(source).bind(id).bind(company).bind(desc).bind(text).bind(active).execute(db).await.unwrap();
    }

    #[tokio::test]
    async fn fills_only_exact_active_identity_and_preserves_every_other_field() {
        let db = crate::test_database().await;
        let canonical = "ats/smartrecruiters/board";
        let legacy = "smartrecruiters";
        insert(
            &db,
            "Target",
            canonical,
            "match",
            Some("  "),
            Some("already here"),
            true,
        )
        .await;
        insert(&db, "Target", legacy, "legacy", None, None, true).await;
        insert(&db, "Neighbor", canonical, "match", None, None, true).await;
        insert(
            &db,
            "Target",
            "ats/smartrecruiters/board-v2",
            "match",
            None,
            None,
            true,
        )
        .await;
        insert(&db, "Target", canonical, "different-id", None, None, true).await;
        insert(&db, "Target", canonical, "inactive", None, None, false).await;

        assert_eq!(
            fill_missing_description(
                &db,
                "Target",
                canonical,
                "match",
                Some(" fetched "),
                Some("new text")
            )
            .await
            .unwrap(),
            1
        );
        assert_eq!(
            fill_missing_description(
                &db,
                "Target",
                legacy,
                "legacy",
                Some("legacy desc"),
                Some("legacy text")
            )
            .await
            .unwrap(),
            1
        );
        assert_eq!(
            fill_missing_description(
                &db,
                "Target",
                canonical,
                "missing",
                Some("not inserted"),
                Some("not inserted")
            )
            .await
            .unwrap(),
            0
        );
        assert_eq!(incomplete_count(&db, "Target", canonical).await.unwrap(), 1);
        let (description, text, title, details, active): (Option<String>, Option<String>, String, String, bool) = sqlx::query_as("SELECT description,description_text,title,details_json,is_active FROM job_postings WHERE company='Target' AND source=$1 AND source_job_id='match'").bind(canonical).fetch_one(&db).await.unwrap();
        assert_eq!(description.as_deref(), Some("fetched"));
        assert_eq!(text.as_deref(), Some("already here"));
        assert_eq!(title, "keep title");
        assert_eq!(details, "{\"keep\":true}");
        assert!(active);
        let untouched: i64 = sqlx::query_scalar("SELECT count(*) FROM job_postings WHERE (company='Neighbor' AND description IS NULL) OR (source='ats/smartrecruiters/board-v2' AND description IS NULL) OR (source=$1 AND source_job_id IN ('inactive','different-id') AND description IS NULL)").bind(canonical).fetch_one(&db).await.unwrap();
        assert_eq!(untouched, 4);
    }

    async fn incomplete_count(
        db: &PgPool,
        company: &str,
        source: &str,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT count(*) FROM job_postings WHERE company=$1 AND source=$2 AND is_active=TRUE AND (NULLIF(BTRIM(description),'') IS NULL OR NULLIF(BTRIM(description_text),'') IS NULL)")
            .bind(company).bind(source).fetch_one(db).await
    }
}
