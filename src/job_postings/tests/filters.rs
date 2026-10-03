use super::*;

#[test]
fn role_filter_searches_role_metadata_not_description_or_company() {
    let filters = ListingQuery {
        page: None,
        page_size: None,
        role_category: None,
        country: None,
        role: Some("intern".into()),
        location: None,
        workplace: None,
        include_global: None,
        include_unknown_workplace: None,
        employment: None,
        department: None,
        posted_after: None,
        posted_before: None,
        salary_min: None,
        salary_max: None,
        currency: None,
        salary_interval: None,
    };
    let mut query = QueryBuilder::<Postgres>::new("SELECT * FROM job_postings");
    append_listing_filters(&mut query, &filters);
    let sql = query.build().sql().to_owned();
    assert!(sql.contains("title ~*"));
    assert!(!sql.contains("department"));
    assert!(!sql.contains("team"));
    assert!(!sql.contains("description"));
    assert!(!sql.contains("company ILIKE"));
}

#[test]
fn role_search_uses_all_title_tokens_and_frontend_aliases_only() {
    let filters = ListingQuery {
        page: None,
        page_size: None,
        role_category: None,
        country: None,
        role: Some("software engineer".into()),
        location: None,
        workplace: None,
        include_global: None,
        include_unknown_workplace: None,
        employment: None,
        department: None,
        posted_after: None,
        posted_before: None,
        salary_min: None,
        salary_max: None,
        currency: None,
        salary_interval: None,
    };
    let mut query = QueryBuilder::<Postgres>::new("SELECT * FROM job_postings");
    append_listing_filters(&mut query, &filters);
    let sql = query.build().sql().to_owned();
    assert_eq!(sql.matches("title ILIKE").count(), 2);
    assert!(!sql.contains("company ILIKE") && !sql.contains("description ILIKE"));

    let filters = ListingQuery {
        role: Some("front-end".into()),
        ..filters.clone()
    };
    let mut query = QueryBuilder::<Postgres>::new("SELECT * FROM job_postings");
    append_listing_filters(&mut query, &filters);
    let sql = query.build().sql().to_owned();
    assert_eq!(sql.matches("title ILIKE").count(), 3);
    assert!(sql.contains("OR title ILIKE"));
}

#[test]
fn india_aliases_and_inclusive_filters_are_opt_in() {
    let filters = ListingQuery {
        page: None,
        page_size: None,
        role_category: None,
        country: None,
        role: None,
        location: Some("India".into()),
        workplace: Some("remote".into()),
        include_global: Some(true),
        include_unknown_workplace: Some(true),
        employment: None,
        department: None,
        posted_after: None,
        posted_before: None,
        salary_min: None,
        salary_max: None,
        currency: None,
        salary_interval: None,
    };
    let mut query = QueryBuilder::<Postgres>::new("SELECT * FROM job_postings");
    append_listing_filters(&mut query, &filters);
    let sql = query.build().sql().to_owned();
    assert!(sql.contains("worldwide") && sql.contains("anywhere"));
    let aliases = location_search_patterns("India");
    assert!(
        aliases.contains(&"%Bengaluru%".to_owned()) && aliases.contains(&"%Bangalore%".to_owned())
    );
    assert!(sql.matches("COALESCE(location, '') ILIKE").count() >= aliases.len());
    assert!(sql.contains("workplace_type IS NULL"));

    let strict = ListingQuery {
        include_global: None,
        include_unknown_workplace: None,
        ..filters
    };
    let mut query = QueryBuilder::<Postgres>::new("SELECT * FROM job_postings");
    append_listing_filters(&mut query, &strict);
    let sql = query.build().sql().to_owned();
    assert!(!sql.contains("worldwide") && !sql.contains("workplace_type IS NULL"));
}

#[test]
fn salary_filters_require_explicit_currency_and_supported_interval() {
    let mut query = empty_filters();
    query.salary_min = Some(100.0);
    assert!(validate_salary_filters(&query).is_err());
    query.currency = Some("USD".into());
    assert!(validate_salary_filters(&query).is_err());
    query.salary_interval = Some("decade".into());
    assert!(validate_salary_filters(&query).is_err());
    query.salary_interval = Some("1 YEAR".into());
    assert!(validate_salary_filters(&query).is_ok());
    query.salary_min = Some(f64::INFINITY);
    assert!(validate_salary_filters(&query).is_err());
}

#[test]
fn category_filters_reject_unsupported_values() {
    for role in ["sde-3", "intern%", "Intern"] {
        let filters = ListingQuery {
            role_category: Some(role.into()),
            ..empty_filters()
        };
        assert!(validate_salary_filters(&filters).is_err());
    }
    for country in ["USA", "India", "us", "GB"] {
        let filters = ListingQuery {
            country: Some(country.into()),
            ..empty_filters()
        };
        assert!(validate_salary_filters(&filters).is_err());
    }
}

#[tokio::test]
async fn intern_search_matches_whole_word_family_not_unrelated_prefixes() {
    let db = crate::test_database().await;
    let titles = [
        "Software Engineer Intern",
        "INTERN — Product",
        "Summer Internship",
        "Engineering Internships",
        "Design Interns",
        "Software Engineer (Intern)",
        "Software Engineer - Intern/Co-op",
        "Internal Auditor",
        "International Sales",
        "Internet Engineer",
        "Software Engineer",
        "InternshipCoordinator",
        "Winterintern",
    ];
    for (index, title) in titles.iter().enumerate() {
        sqlx::query("INSERT INTO job_postings (source, source_job_id, title, company, url, is_active, description_text) VALUES ('test', $1, $2, 'Intern Company', 'https://example.com', TRUE, 'intern internship')")
                .bind(index.to_string())
                .bind(title)
                .execute(&db)
                .await
                .unwrap();
    }
    for role in ["intern", "INTERN", "interns", "internship", "internships"] {
        let filters = ListingQuery {
            role: Some(role.into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &filters).await, 7, "role={role}");
    }
    let filters = ListingQuery {
        role: Some("software intern".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &filters).await, 3);
    let filters = ListingQuery {
        role: Some("internal".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &filters).await, 1);
    sqlx::query("UPDATE job_postings SET is_active = FALSE WHERE title = 'Summer Internship'")
        .execute(&db)
        .await
        .unwrap();
    let filters = ListingQuery {
        role: Some("intern".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &filters).await, 6);
}

#[tokio::test]
async fn database_filters_match_canonical_tokens_locations_employment_and_salary_units() {
    let db = crate::test_database().await;
    let fixtures = [
        (
            "Software Development Engineer",
            "Bengaluru, India",
            Some("remote"),
            Some("FullTime"),
            Some("Product"),
            Some("Core"),
            Some(130000.0),
            Some(150000.0),
            Some("USD"),
            Some("year"),
        ),
        (
            "Front-end Engineer",
            "India",
            None,
            Some("full time"),
            Some("Product"),
            None,
            Some(50.0),
            Some(70.0),
            Some("USD"),
            Some("hour"),
        ),
        (
            "UI Designer",
            "Remote",
            Some("remote"),
            Some("Part_Time"),
            None,
            None,
            Some(90000.0),
            Some(90000.0),
            Some("USD"),
            Some("year"),
        ),
        (
            "Engineer",
            "Global office",
            Some("onsite"),
            Some("Full-time"),
            None,
            None,
            Some(100000.0),
            Some(110000.0),
            Some("USD"),
            Some("year"),
        ),
        (
            "Engineer",
            "Mumbai, India",
            None,
            Some("Intern"),
            None,
            None,
            None,
            None,
            None,
            None,
        ),
        (
            "Engineer",
            "India",
            Some("hybrid"),
            Some("Contractor"),
            None,
            None,
            Some(120000.0),
            Some(130000.0),
            Some("EUR"),
            Some("year"),
        ),
        (
            "Engineer",
            "Remote; Dublin",
            Some("remote"),
            None,
            None,
            None,
            Some(140000.0),
            Some(150000.0),
            Some("USD"),
            None,
        ),
        (
            "Discount 50%_off\\\\sale",
            "India",
            None,
            Some("Full Time"),
            Some("Sales"),
            None,
            None,
            None,
            None,
            None,
        ),
    ];
    for (
        index,
        (title, location, workplace, employment, department, team, low, high, currency, interval),
    ) in fixtures.into_iter().enumerate()
    {
        sqlx::query("INSERT INTO job_postings (source, source_job_id, title, company, location, workplace_type, employment_type, department, team, salary_min, salary_max, salary_currency, salary_interval, url) VALUES ('test', $1, $2, 'Fixture', $3, $4, $5, $6, $7, $8, $9, $10, $11, 'https://example.test/job')")
                .bind(index.to_string()).bind(title).bind(location).bind(workplace).bind(employment)
                .bind(department).bind(team).bind(low).bind(high).bind(currency).bind(interval)
                .execute(&db).await.unwrap();
    }

    let roles = ListingQuery {
        role: Some("software engineer".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &roles).await, 1);
    let front = ListingQuery {
        role: Some("front end".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &front).await, 1);
    let city = ListingQuery {
        location: Some("India".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &city).await, 5);
    let other_country_remote = ListingQuery {
        location: Some("Canada".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &other_country_remote).await, 0);
    let other_country_inclusive = ListingQuery {
        location: Some("Canada".into()),
        include_global: Some(true),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &other_country_inclusive).await, 2);
    let inclusive = ListingQuery {
        location: Some("India".into()),
        include_global: Some(true),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &inclusive).await, 7);
    let no_unknown = ListingQuery {
        workplace: Some("remote".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &no_unknown).await, 3);
    let with_unknown = ListingQuery {
        workplace: Some("remote".into()),
        include_unknown_workplace: Some(true),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &with_unknown).await, 6);
    let full_time = ListingQuery {
        employment: Some("full-time".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &full_time).await, 4);
    let part_time = ListingQuery {
        employment: Some("parttime".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &part_time).await, 1);
    let contract = ListingQuery {
        employment: Some("fixed-term".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &contract).await, 1);
    let internship = ListingQuery {
        employment: Some("Internship".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &internship).await, 1);
    let salary = ListingQuery {
        salary_min: Some(120000.0),
        currency: Some("USD".into()),
        salary_interval: Some("annual".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &salary).await, 1);
    let hourly = ListingQuery {
        salary_max: Some(100.0),
        currency: Some("USD".into()),
        salary_interval: Some("hour".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &hourly).await, 1);
    let literal = ListingQuery {
        role: Some("50%_off\\\\sale".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &literal).await, 1);
    let percent = ListingQuery {
        role: Some("%".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &percent).await, 1);
    for filters in [
        ListingQuery {
            location: Some("%".into()),
            ..empty_filters()
        },
        ListingQuery {
            workplace: Some("%".into()),
            ..empty_filters()
        },
        ListingQuery {
            department: Some("%".into()),
            ..empty_filters()
        },
        ListingQuery {
            currency: Some("%".into()),
            ..empty_filters()
        },
    ] {
        assert_eq!(matching_count(&db, &filters).await, 0);
    }
    let global = ListingQuery {
        location: Some("India".into()),
        include_global: Some(true),
        ..empty_filters()
    };
    // A broad substring such as "Global office" is deliberately not considered worldwide.
    let global_count = matching_count(&db, &global).await;
    assert_eq!(global_count, 7);
}
