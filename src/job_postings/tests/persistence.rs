use super::*;

#[tokio::test]
async fn exact_source_reconciliation_does_not_prefix_match_neighbor_sources() {
    let db = crate::test_database().await;
    sqlx::query("INSERT INTO job_postings (source, source_job_id, title, company, url) VALUES ('lever', 'old', 'Old', 'Example', 'https://example.test/old'), ('lever-v2', 'neighbor', 'Neighbor', 'Example', 'https://example.test/neighbor')")
            .execute(&db)
            .await
            .unwrap();

    let snapshot = vec![JobPosting::basic(
        "lever",
        "new",
        "New".into(),
        "Example",
        None,
        None,
        "https://example.test/new".into(),
        serde_json::json!({}),
    )];
    assert!(replace_source_snapshot(&db, "Example", "lever", &snapshot)
        .await
        .is_ok());

    let old_active: bool =
        sqlx::query_scalar("SELECT is_active FROM job_postings WHERE source_job_id = 'old'")
            .fetch_one(&db)
            .await
            .unwrap();
    let neighbor_active: bool =
        sqlx::query_scalar("SELECT is_active FROM job_postings WHERE source_job_id = 'neighbor'")
            .fetch_one(&db)
            .await
            .unwrap();
    let new_active: bool =
        sqlx::query_scalar("SELECT is_active FROM job_postings WHERE source_job_id = 'new'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(!old_active);
    assert!(neighbor_active);
    assert!(new_active);
}

#[tokio::test]
async fn exact_source_reconciliation_rejects_mismatched_snapshot_before_deactivate() {
    let db = crate::test_database().await;
    sqlx::query("INSERT INTO job_postings (source, source_job_id, title, company, url) VALUES ('lever', 'old', 'Old', 'Example', 'https://example.test/old')")
            .execute(&db)
            .await
            .unwrap();

    let snapshot = vec![JobPosting::basic(
        "lever-v2",
        "new",
        "New".into(),
        "Example",
        None,
        None,
        "https://example.test/new".into(),
        serde_json::json!({}),
    )];
    assert!(replace_source_snapshot(&db, "Example", "lever", &snapshot)
        .await
        .is_err());

    let old_active: bool =
        sqlx::query_scalar("SELECT is_active FROM job_postings WHERE source_job_id = 'old'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(old_active);
}
