use super::*;

#[tokio::test]
async fn smartrecruiters_http_pagination_fetches_all_jobs_past_500() {
    let (base, server) = mock_smart_server(6, |offset, limit| {
        let jobs = (offset..(offset + limit).min(501))
            .map(|i| json!({"id":format!("j{i}"),"name":format!("Role {i}")}))
            .collect::<Vec<_>>();
        json!({"totalFound":501,"content":jobs}).to_string()
    });
    let id = identity(AtsProvider::SmartRecruiters);
    let result = fetch_smartrecruiters_at(
        &reqwest::Client::new(),
        &id,
        "test",
        &AtsFetchOptions::default(),
        &base,
    )
    .await
    .unwrap();
    let seen = server.join().unwrap();
    assert_eq!(
        seen.iter().map(|(offset, _)| *offset).collect::<Vec<_>>(),
        [0, 100, 200, 300, 400, 500]
    );
    assert_eq!(result.len(), 501);
    assert_eq!(result[500].id, "j500");
}

#[tokio::test]
async fn smartrecruiters_rejects_missing_total_oversized_pages_and_caps() {
    let id = identity(AtsProvider::SmartRecruiters);
    let (base, server) = mock_smart_server(1, |_, _| json!({"content":[]}).to_string());
    assert!(fetch_smartrecruiters_at(
        &reqwest::Client::new(),
        &id,
        "test",
        &AtsFetchOptions::default(),
        &base
    )
    .await
    .is_err());
    server.join().unwrap();

    let (base, server) = mock_smart_server(1, |_, _| {
        json!({"totalFound":1,"content":[{"name":"No ID"}]}).to_string()
    });
    assert!(fetch_smartrecruiters_at(
        &reqwest::Client::new(),
        &id,
        "test",
        &AtsFetchOptions::default(),
        &base
    )
    .await
    .is_err());
    server.join().unwrap();

    let (base, server) = mock_smart_server(1, |_, _| {
        let content = (0..101)
            .map(|i| json!({"id":format!("{i}"),"name":"Role"}))
            .collect::<Vec<_>>();
        json!({"totalFound":101,"content":content}).to_string()
    });
    assert!(fetch_smartrecruiters_at(
        &reqwest::Client::new(),
        &id,
        "test",
        &AtsFetchOptions::default(),
        &base
    )
    .await
    .is_err());
    server.join().unwrap();

    let (base, server) = mock_smart_server(1, |_, limit| {
        let content = (0..limit)
            .map(|i| json!({"id":format!("{i}"),"name":"Role"}))
            .collect::<Vec<_>>();
        json!({"totalFound":101,"content":content}).to_string()
    });
    let options = AtsFetchOptions {
        max_jobs_per_company: 100,
    };
    assert!(
        fetch_smartrecruiters_at(&reqwest::Client::new(), &id, "test", &options, &base)
            .await
            .is_err()
    );
    server.join().unwrap();
}

#[tokio::test]
async fn smartrecruiters_rejects_duplicate_ids_and_changing_totals() {
    let id = identity(AtsProvider::SmartRecruiters);
    for change_total in [false, true] {
        let (base, server) = mock_smart_server(2, move |offset, limit| {
            let total = if change_total && offset > 0 { 102 } else { 101 };
            let content = if offset == 0 {
                (0..limit)
                    .map(|i| json!({"id":format!("j{i}"),"name":"Role"}))
                    .collect::<Vec<_>>()
            } else {
                vec![json!({"id": if change_total { "j100" } else { "j0" },"name":"Role"})]
            };
            json!({"totalFound":total,"content":content}).to_string()
        });
        assert!(fetch_smartrecruiters_at(
            &reqwest::Client::new(),
            &id,
            "test",
            &AtsFetchOptions::default(),
            &base
        )
        .await
        .is_err());
        server.join().unwrap();
    }
}

#[tokio::test]
async fn company_deadline_returns_an_error_without_partial_success() {
    let result: Result<(), AtsFetchError> = run_with_deadline(
        Duration::from_millis(5),
        AtsProvider::Workday,
        "Example".into(),
        std::future::pending(),
    )
    .await;
    assert!(matches!(result, Err(AtsFetchError::Timeout { .. })));
}

#[test]
fn pagination_can_continue_past_500_jobs_and_is_bounded() {
    let mut total = 0;
    let mut offsets = Vec::new();
    while let Some(limit) = next_page_limit(100, total, 10_000) {
        offsets.push(total);
        total += if total == 600 { 50 } else { limit };
        if total >= 650 {
            break;
        }
    }
    assert_eq!(offsets, [0, 100, 200, 300, 400, 500, 600]);
    assert_eq!(total, 650);
    assert_eq!(next_page_limit(100, 10_000, 10_000), None);
    let id = identity(AtsProvider::Workday);
    assert!(ensure_complete_snapshot(&id, "fixture", 650, 650).is_ok());
    assert!(ensure_complete_snapshot(&id, "fixture", 500, 650).is_err());
}

#[test]
fn rejects_malformed_feed_instead_of_empty_success() {
    let identity = identity(AtsProvider::Lever);
    assert!(map_lever_jobs(&identity, "fixture", &json!({"jobs": []})).is_err());
    assert!(map_greenhouse_jobs(&identity, "fixture", &json!({})).is_err());
    assert!(map_workday_jobs(
        &identity,
        "fixture",
        &json!({"jobs": []}),
        &WorkdayConfig {
            tenant: "example".to_owned(),
            site: "External".to_owned(),
            shard: "wd1".to_owned(),
        }
    )
    .is_err());
}

#[test]
fn safe_pagination_limits_are_capped() {
    assert_eq!(next_page_limit(100, 0, 500), Some(100));
    assert_eq!(next_page_limit(100, 450, 500), Some(50));
    assert_eq!(next_page_limit(100, 500, 500), None);
    assert_eq!(next_page_limit(20, 495, 500), Some(5));
    assert_eq!(
        AtsFetchOptions {
            max_jobs_per_company: 900
        }
        .capped_max_jobs(),
        900
    );
}

#[test]
fn http_client_builders_configure_timeouts() {
    http_client_with_timeout(Duration::from_millis(50)).expect("custom timeout client builds");
}

#[test]
fn provider_parser_accepts_common_aliases() {
    assert_eq!(
        "smart-recruiters".parse::<AtsProvider>().unwrap(),
        AtsProvider::SmartRecruiters
    );
    assert_eq!(
        "bamboo-hr".parse::<AtsProvider>().unwrap(),
        AtsProvider::BambooHr
    );
    assert!("unknown".parse::<AtsProvider>().is_err());
}

#[test]
fn source_identity_uses_provider_and_slug() {
    let source = AtsCompanySource {
        company_id: Some("acme".to_owned()),
        company_name: "Acme".to_owned(),
        provider: AtsProvider::Ashby,
        slug: Some("acme-careers".to_owned()),
        workday: None,
    };
    assert_eq!(
        source.identity().unwrap().source_key(),
        "ats/ashby/acme-careers"
    );
}
