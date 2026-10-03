use super::*;

#[test]
fn maps_greenhouse_fixture() {
    let data = json!({
        "jobs": [{
            "id": 42,
            "title": "Platform Engineer",
            "absolute_url": "https://boards.greenhouse.io/example/jobs/42",
            "first_published": "2026-10-02T11:31:50-04:00",
            "location": {"name": "Remote - EU"},
            "departments": [{"name": "Engineering"}]
        }]
    });
    let identity = identity(AtsProvider::Greenhouse);
    let fetched = to_fetched_jobs(
        identity.clone(),
        map_greenhouse_jobs(&identity, "fixture", &data).unwrap(),
    );
    let job = &fetched[0].posting;
    assert_eq!(job.source, "ats/greenhouse/example");
    assert_eq!(job.company, "Example");
    assert_eq!(job.source_job_id, "42");
    assert_eq!(job.title, "Platform Engineer");
    assert_eq!(job.department.as_deref(), Some("Engineering"));
    assert_eq!(job.location.as_deref(), Some("Remote - EU"));
    assert_eq!(job.workplace_type.as_deref(), Some("remote"));
    assert_eq!(job.url, "https://boards.greenhouse.io/example/jobs/42");
    assert_eq!(job.posted_at.as_deref(), Some("2026-10-02T11:31:50-04:00"));
}

#[test]
fn maps_lever_fixture_and_millis_date() {
    let data = json!([{
        "id": "abc",
        "text": "Backend Engineer",
        "applyUrl": "https://jobs.lever.co/example/abc",
        "createdAt": 1751559111614i64,
        "workplaceType": "hybrid",
        "categories": {
            "department": "Product Engineering",
            "commitment": "Full-time",
            "allLocations": ["London", "Remote - UK"]
        }
    }]);
    let identity = identity(AtsProvider::Lever);
    let fetched = to_fetched_jobs(
        identity.clone(),
        map_lever_jobs(&identity, "fixture", &data).unwrap(),
    );
    let job = &fetched[0].posting;
    assert_eq!(job.source_job_id, "abc");
    assert_eq!(job.title, "Backend Engineer");
    assert_eq!(job.location.as_deref(), Some("London; Remote - UK"));
    assert_eq!(job.employment_type.as_deref(), Some("Full-time"));
    assert_eq!(job.department.as_deref(), Some("Product Engineering"));
    assert_eq!(job.posted_at.as_deref(), Some("2025-07-03T16:11:51.614Z"));
}

#[test]
fn explicit_hybrid_overrides_remote_flags() {
    let ashby_id = identity(AtsProvider::Ashby);
    for remote in [true, false] {
        let data = json!({"jobs": [{"id":"a", "title":"Role", "applyUrl":"https://apply", "workplaceType":"Hybrid", "isRemote":remote, "location":"Canada - Remote"}]});
        let posting = &to_fetched_jobs(
            ashby_id.clone(),
            map_ashby_jobs(&ashby_id, "fixture", &data).unwrap(),
        )[0]
        .posting;
        assert_eq!(posting.workplace_type.as_deref(), Some("hybrid"));
    }
    for (workplace, expected) in [
        ("Remote", "remote"),
        ("OnSite", "onsite"),
        ("On-site", "onsite"),
    ] {
        let data = json!({"jobs": [{"id":"a", "title":"Role", "applyUrl":"https://apply", "workplaceType":workplace, "isRemote":false, "location":"In-office"}]});
        let posting = &to_fetched_jobs(
            ashby_id.clone(),
            map_ashby_jobs(&ashby_id, "fixture", &data).unwrap(),
        )[0]
        .posting;
        assert_eq!(posting.workplace_type.as_deref(), Some(expected));
    }
    let lever_id = identity(AtsProvider::Lever);
    let data = json!([{"id":"l", "text":"Role", "applyUrl":"https://apply", "workplaceType":"hybrid", "categories":{"allLocations":["Canada - Remote"]}}]);
    let posting = &to_fetched_jobs(
        lever_id.clone(),
        map_lever_jobs(&lever_id, "fixture", &data).unwrap(),
    )[0]
    .posting;
    assert_eq!(posting.workplace_type.as_deref(), Some("hybrid"));
    let sr_id = identity(AtsProvider::SmartRecruiters);
    let data = json!({"content":[{"id":"s", "name":"Role", "location":{"remote":false,"hybrid":true,"fullLocation":"Toronto"}}]});
    let posting = &to_fetched_jobs(
        sr_id.clone(),
        map_smartrecruiters_jobs(&sr_id, "fixture", &data, "fallback").unwrap(),
    )[0]
    .posting;
    assert_eq!(posting.workplace_type.as_deref(), Some("hybrid"));
}

#[test]
fn preserves_greenhouse_secondary_offices_and_workday_details() {
    let gh = identity(AtsProvider::Greenhouse);
    let data = json!({"jobs":[{"id":1,"title":"Role","absolute_url":"https://apply","location":{"name":"New York"},"offices":[{"name":"India"}]}]});
    let posting = &to_fetched_jobs(
        gh.clone(),
        map_greenhouse_jobs(&gh, "fixture", &data).unwrap(),
    )[0]
    .posting;
    assert_eq!(posting.location.as_deref(), Some("New York; India"));
    let workday = identity(AtsProvider::Workday);
    let summary = json!({"externalPath":"/job/foo","locationsText":"United States","postedOn":"Posted 3 Days Ago"});
    let enriched = merge_workday_detail(
        summary,
        json!({"jobPostingInfo":{"location":"Austin, TX","additionalLocations":["India"],"startDate":"2026-10-01"}}),
    );
    let result = map_workday_jobs(
        &workday,
        "fixture",
        &json!({"jobPostings":[enriched]}),
        &WorkdayConfig {
            tenant: "example".into(),
            site: "External".into(),
            shard: "wd1".into(),
        },
    )
    .unwrap();
    assert_eq!(
        result[0].locations,
        ["United States", "Austin, TX", "India"]
    );
    assert_eq!(result[0].posted_at.as_deref(), Some("2026-10-01"));
    let relative = map_workday_jobs(
        &workday,
        "fixture",
        &json!({"jobPostings":[{"externalPath":"/job/rel","postedOn":"Posted 3 Days Ago"}]}),
        &WorkdayConfig {
            tenant: "example".into(),
            site: "External".into(),
            shard: "wd1".into(),
        },
    )
    .unwrap();
    assert_eq!(relative[0].posted_at, None);
    assert!(is_absolute_iso_date("2026-10-03"));
    assert!(!is_absolute_iso_date("Posted 3 Days Ago"));
}

#[test]
fn maps_matching_smartrecruiters_job_after_index_500() {
    let id = identity(AtsProvider::SmartRecruiters);
    let jobs = (0..501).map(|index| json!({"id":format!("job-{index}"),"name":format!("Role {index}"),"location":{"fullLocation":"India"}})).collect::<Vec<_>>();
    let mapped =
        map_smartrecruiters_jobs(&id, "fixture", &json!({"content":jobs}), "example").unwrap();
    assert_eq!(mapped.len(), 501);
    assert_eq!(mapped[500].id, "job-500");
}

#[test]
fn maps_workday_fixture() {
    let data = json!({
        "jobPostings": [{
            "externalPath": "/job/US-Remote/Staff-Engineer_R1",
            "title": "Staff Engineer",
            "locationsText": "United States - Remote",
            "remoteType": "Remote",
            "timeType": "Full time"
        }]
    });
    let workday = WorkdayConfig {
        tenant: "example".to_owned(),
        site: "External".to_owned(),
        shard: "wd1".to_owned(),
    };
    let identity = AtsSourceIdentity {
        provider: AtsProvider::Workday,
        company_id: None,
        company_name: "Example".to_owned(),
        source_ref: "example/wd1/External".to_owned(),
    };
    let fetched = to_fetched_jobs(
        identity.clone(),
        map_workday_jobs(&identity, "fixture", &data, &workday).unwrap(),
    );
    let job = &fetched[0].posting;
    assert_eq!(job.source, "ats/workday/example/wd1/External");
    assert_eq!(job.source_job_id, "/job/US-Remote/Staff-Engineer_R1");
    assert_eq!(job.workplace_type.as_deref(), Some("remote"));
    assert_eq!(job.employment_type.as_deref(), Some("Full time"));
    assert_eq!(
        job.url,
        "https://example.wd1.myworkdayjobs.com/External/job/US-Remote/Staff-Engineer_R1"
    );
}

#[test]
fn maps_ashby_fixture() {
    let data = json!({
        "jobs": [{
            "id": "job_123",
            "title": " Product Designer ",
            "department": "Design",
            "employmentType": "FullTime",
            "location": "New York",
            "secondaryLocations": [{"location": "Remote - US"}, {"location": "New York"}],
            "isRemote": true,
            "applyUrl": "https://jobs.ashbyhq.com/example/job_123/application",
            "publishedAt": "2026-10-01T09:00:00Z"
        }]
    });
    let identity = identity(AtsProvider::Ashby);
    let fetched = to_fetched_jobs(
        identity.clone(),
        map_ashby_jobs(&identity, "fixture", &data).unwrap(),
    );
    let job = &fetched[0].posting;
    assert_eq!(job.source, "ats/ashby/example");
    assert_eq!(job.source_job_id, "job_123");
    assert_eq!(job.title, "Product Designer");
    assert_eq!(job.location.as_deref(), Some("New York; Remote - US"));
    assert_eq!(job.workplace_type.as_deref(), Some("remote"));
    assert_eq!(job.department.as_deref(), Some("Design"));
    assert_eq!(job.employment_type.as_deref(), Some("FullTime"));
}

#[test]
fn maps_smartrecruiters_fixture() {
    let data = json!({
        "content": [{
            "id": "743999999999999",
            "name": "Data Engineer",
            "company": {"identifier": "ExampleCo"},
            "department": {"label": "Data"},
            "typeOfEmployment": {"label": "Full-time"},
            "releasedDate": "2026-09-30T12:00:00.000Z",
            "location": {"fullLocation": "Berlin, Germany", "remote": false}
        }]
    });
    let identity = identity(AtsProvider::SmartRecruiters);
    let fetched = to_fetched_jobs(
        identity.clone(),
        map_smartrecruiters_jobs(&identity, "fixture", &data, "fallback").unwrap(),
    );
    let job = &fetched[0].posting;
    assert_eq!(job.source, "ats/smartrecruiters/example");
    assert_eq!(job.source_job_id, "743999999999999");
    assert_eq!(job.workplace_type.as_deref(), Some("onsite"));
    assert_eq!(job.location.as_deref(), Some("Berlin, Germany"));
    assert_eq!(
        job.url,
        "https://jobs.smartrecruiters.com/ExampleCo/743999999999999"
    );
}

#[test]
fn maps_workable_fixture() {
    let data = json!({
        "jobs": [{
            "shortcode": "ABC123",
            "title": "Engineering Manager",
            "department": "Engineering",
            "city": "Paris",
            "state": "Ile-de-France",
            "country": "France",
            "telecommuting": true,
            "employment_type": "Full-time",
            "application_url": "https://apply.workable.com/example/j/ABC123/apply/",
            "published_on": "2026-09-29"
        }]
    });
    let identity = identity(AtsProvider::Workable);
    let fetched = to_fetched_jobs(
        identity.clone(),
        map_workable_jobs(&identity, "fixture", &data).unwrap(),
    );
    let job = &fetched[0].posting;
    assert_eq!(job.source, "ats/workable/example");
    assert_eq!(job.source_job_id, "ABC123");
    assert_eq!(
        job.location.as_deref(),
        Some("Paris; Ile-de-France; France")
    );
    assert_eq!(job.workplace_type.as_deref(), Some("remote"));
    assert_eq!(job.employment_type.as_deref(), Some("Full-time"));
}

#[test]
fn maps_recruitee_fixture() {
    let data = json!({
        "offers": [{
            "id": 987,
            "title": "Security Engineer",
            "department": "Security",
            "employment_type_code": "full_time",
            "careers_url": "https://example.recruitee.com/o/security-engineer",
            "published_at": "2026-09-28T08:00:00Z",
            "remote": false,
            "hybrid": true,
            "locations": [{"name": "Amsterdam"}, {"name": "Amsterdam"}]
        }]
    });
    let identity = identity(AtsProvider::Recruitee);
    let fetched = to_fetched_jobs(
        identity.clone(),
        map_recruitee_jobs(&identity, "fixture", &data).unwrap(),
    );
    let job = &fetched[0].posting;
    assert_eq!(job.source, "ats/recruitee/example");
    assert_eq!(job.source_job_id, "987");
    assert_eq!(job.location.as_deref(), Some("Amsterdam"));
    assert_eq!(job.workplace_type.as_deref(), Some("hybrid"));
    assert_eq!(job.department.as_deref(), Some("Security"));
}

#[test]
fn maps_bamboohr_fixture() {
    let data = json!({
        "result": [{
            "id": 55,
            "jobOpeningName": " Support Specialist ",
            "departmentLabel": "Customer Success",
            "employmentStatusLabel": "Contractor",
            "isRemote": false,
            "location": {"city": "Austin", "state": "TX"},
            "atsLocation": {"city": "Austin", "state": "TX", "country": "US"}
        }]
    });
    let identity = identity(AtsProvider::BambooHr);
    let fetched = to_fetched_jobs(
        identity.clone(),
        map_bamboohr_jobs(&identity, "fixture", &data, "example").unwrap(),
    );
    let job = &fetched[0].posting;
    assert_eq!(job.source, "ats/bamboohr/example");
    assert_eq!(job.source_job_id, "55");
    assert_eq!(job.title, "Support Specialist");
    assert_eq!(job.location.as_deref(), Some("Austin; TX; US"));
    assert_eq!(job.workplace_type.as_deref(), Some("onsite"));
    assert_eq!(job.url, "https://example.bamboohr.com/careers/55");
}
