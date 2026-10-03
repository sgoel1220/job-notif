use super::*;

#[tokio::test]
async fn category_migration_backfills_and_reclassifies_jobs_on_update() {
    let db = crate::test_database().await;
    // Recreate the pre-migration shape to verify backfill, not only new inserts.
    sqlx::raw_sql("ALTER TABLE job_postings DROP COLUMN role_category, DROP COLUMN country_codes; DROP FUNCTION job_role_category(TEXT, TEXT); DROP FUNCTION job_country_codes(TEXT);")
            .execute(&db).await.unwrap();
    sqlx::query("INSERT INTO job_postings (source, source_job_id, title, company, location, url) VALUES ('test', 'old', 'Software Engineer Intern', 'Example', 'Bengaluru', 'https://example.com')")
            .execute(&db).await.unwrap();
    sqlx::raw_sql(include_str!("../../../migrations/0004_job_categories.sql"))
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO job_postings (source, source_job_id, title, company, location, url) VALUES ('test', 'design', 'Design Intern', 'Example', 'India', 'https://example.com')")
            .execute(&db).await.unwrap();
    let pre_update_interns = ListingQuery {
        role_category: Some("intern".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &pre_update_interns).await, 2);
    sqlx::raw_sql(include_str!(
        "../../../migrations/0006_software_intern_categories.sql"
    ))
    .execute(&db)
    .await
    .unwrap();
    let filters = ListingQuery {
        role_category: Some("intern".into()),
        country: Some("IN".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &filters).await, 1);
    let others = ListingQuery {
        role_category: Some("other".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &others).await, 1);
    sqlx::query("UPDATE job_postings SET title = 'Software Engineer II', location = 'United States' WHERE source_job_id = 'old'")
            .execute(&db).await.unwrap();
    assert_eq!(matching_count(&db, &filters).await, 0);
    let filters = ListingQuery {
        role_category: Some("sde-2".into()),
        country: Some("US".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &filters).await, 1);
}

#[tokio::test]
async fn database_category_rules_are_conservative_and_exact() {
    let db = crate::test_database().await;
    let fixtures = [
        (
            "Software Engineer Intern",
            None,
            "India",
            "intern",
            vec!["IN"],
        ),
        ("Summer Internship", None, "US", "other", vec!["US"]),
        (
            "Software Engineer",
            Some("Internship"),
            "USA",
            "intern",
            vec!["US"],
        ),
        ("Internal Auditor", None, "London", "other", vec![]),
        ("International Sales", None, "Worldwide", "other", vec![]),
        ("Internet Engineer", None, "Remote", "other", vec![]),
        (
            "Software Engineer I",
            None,
            "Bengaluru",
            "sde-1",
            vec!["IN"],
        ),
        (
            "Software Development Engineer 1",
            None,
            "New Delhi",
            "sde-1",
            vec!["IN"],
        ),
        ("SDE-1", None, "U.S.A.", "sde-1", vec!["US"]),
        ("SDE1", None, "San Francisco", "sde-1", vec!["US"]),
        (
            "Junior Software Developer",
            None,
            "Seattle",
            "sde-1",
            vec!["US"],
        ),
        (
            "Software Engineer, New Graduate",
            None,
            "Mumbai, India",
            "sde-1",
            vec!["IN"],
        ),
        (
            "Software Engineer II",
            None,
            "United States of America",
            "sde-2",
            vec!["US"],
        ),
        (
            "Software Development Engineer 2",
            None,
            "USA; India",
            "sde-2",
            vec!["US", "IN"],
        ),
        ("SDE2", None, "United States", "sde-2", vec!["US"]),
        ("SDE-2", None, "U.S.", "sde-2", vec!["US"]),
        (
            "Mid-level Software Engineer",
            None,
            "Hyderabad",
            "sde-2",
            vec!["IN"],
        ),
        (
            "Software Engineer, Level II",
            None,
            "Pune",
            "sde-2",
            vec!["IN"],
        ),
        (
            "Senior Software Engineer II",
            None,
            "Indiana, US",
            "other",
            vec!["US"],
        ),
        ("SDE-10", None, "Indianapolis", "other", vec![]),
        ("Software Engineer III", None, "Australia", "other", vec![]),
        ("Software Engineer", None, "Remote", "other", vec![]),
        ("Junior Accountant", None, "Canada", "other", vec![]),
    ];
    for (index, (title, employment, location, expected_role, expected_countries)) in
        fixtures.iter().enumerate()
    {
        let (role, countries): (String, Vec<String>) = sqlx::query_as("INSERT INTO job_postings (source, source_job_id, title, company, employment_type, location, url) VALUES ('test', $1, $2, 'Example', $3, $4, 'https://example.com') RETURNING role_category, country_codes")
                .bind(index.to_string()).bind(title).bind(employment).bind(location)
                .fetch_one(&db).await.unwrap();
        assert_eq!(role, *expected_role, "title={title}");
        assert_eq!(countries, *expected_countries, "location={location}");
    }
    for role in ["intern", "sde-1", "sde-2", "other"] {
        for country in [None, Some("US"), Some("IN")] {
            let expected = fixtures
                .iter()
                .filter(|(_, _, _, category, countries)| {
                    *category == role && country.is_none_or(|c| countries.contains(&c))
                })
                .count() as i64;
            let filters = ListingQuery {
                role_category: Some(role.into()),
                country: country.map(str::to_owned),
                ..empty_filters()
            };
            assert_eq!(
                matching_count(&db, &filters).await,
                expected,
                "role={role}, country={country:?}"
            );
        }
    }
    let filters = ListingQuery {
        country: Some("IN".into()),
        ..empty_filters()
    };
    let strict = matching_count(&db, &filters).await;
    let inclusive = ListingQuery {
        include_global: Some(true),
        ..filters.clone()
    };
    assert_eq!(matching_count(&db, &inclusive).await, strict + 3);
    sqlx::query(
        "UPDATE job_postings SET is_active = FALSE WHERE title = 'Software Engineer Intern'",
    )
    .execute(&db)
    .await
    .unwrap();
    let filters = ListingQuery {
        role_category: Some("intern".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &filters).await, 1);
}

#[tokio::test]
async fn software_engineering_filter_spans_levels_and_other_catches_remaining_jobs() {
    let db = crate::test_database().await;
    let software_titles = [
        "Software Engineer Intern",
        "Software Engineer I",
        "Software Engineer II",
        "Senior Software Engineer",
        "Software Engineer",
        "Software Developer",
        "Software Engineering Manager",
        "SDE-2",
        "SDE10",
        "Frontend Engineer",
        "Back-end Developer",
        "Full Stack Engineer",
        "Mobile Engineer",
        "iOS Developer",
        "Android Software Engineer",
        "Embedded Software Engineer",
    ];
    let other_titles = [
        "Accountant",
        "Internal Auditor",
        "International Sales",
        "Internet Engineer",
        "Hardware Engineer",
        "Software Sales Manager",
        "Software EngineeringIntern",
    ];
    for (index, title) in software_titles
        .iter()
        .chain(other_titles.iter())
        .chain([&"Design Intern"])
        .enumerate()
    {
        sqlx::query("INSERT INTO job_postings (source, source_job_id, title, company, location, url) VALUES ('test', $1, $2, 'Example', 'India', 'https://example.com')")
                .bind(index.to_string()).bind(title).execute(&db).await.unwrap();
    }
    let filters = ListingQuery {
        role_category: Some("software-engineering".into()),
        country: Some("IN".into()),
        ..empty_filters()
    };
    assert!(validate_salary_filters(&filters).is_ok());
    assert_eq!(
        matching_count(&db, &filters).await,
        software_titles.len() as i64
    );
    let other = ListingQuery {
        role_category: Some("other".into()),
        ..empty_filters()
    };
    // Others also includes senior/unspecified software roles and non-software interns.
    assert_eq!(
        matching_count(&db, &other).await,
        other_titles.len() as i64 + 13
    );
    let interns = ListingQuery {
        role_category: Some("intern".into()),
        ..empty_filters()
    };
    assert_eq!(matching_count(&db, &interns).await, 1);
    sqlx::query("UPDATE job_postings SET title = 'Backend Engineer' WHERE title = 'Accountant'")
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        matching_count(&db, &filters).await,
        software_titles.len() as i64 + 1
    );
    assert_eq!(
        matching_count(&db, &other).await,
        other_titles.len() as i64 + 13
    );
}
