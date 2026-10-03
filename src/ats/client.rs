use super::{
    fetch_ashby, fetch_bamboohr, fetch_greenhouse, fetch_lever, fetch_personio, fetch_recruitee,
    fetch_smartrecruiters, fetch_workable, fetch_workday, invalid_feed, run_with_deadline,
    to_fetched_jobs, AtsCompanySource, AtsFetchError, AtsFetchOptions, AtsProvider,
    AtsSourceIdentity, FetchedAtsJob, COMPANY_FETCH_TIMEOUT, USER_AGENT,
};
use serde_json::Value;
use std::time::Duration;

pub(crate) fn http_client_with_timeout(
    timeout: Duration,
) -> Result<reqwest::Client, AtsFetchError> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(timeout)
        .build()
        .map_err(|error| AtsFetchError::ClientBuild {
            message: error.to_string(),
        })
}

pub(crate) async fn fetch_company_jobs(
    client: &reqwest::Client,
    company: &AtsCompanySource,
) -> Result<Vec<FetchedAtsJob>, AtsFetchError> {
    fetch_company_jobs_with_options(client, company, &AtsFetchOptions::default()).await
}

pub(crate) async fn fetch_company_jobs_with_options(
    client: &reqwest::Client,
    company: &AtsCompanySource,
    options: &AtsFetchOptions,
) -> Result<Vec<FetchedAtsJob>, AtsFetchError> {
    let identity = company.identity()?;
    run_with_deadline(
        COMPANY_FETCH_TIMEOUT,
        identity.provider,
        identity.company_name.clone(),
        async {
            let jobs = match company.provider {
                AtsProvider::Greenhouse => fetch_greenhouse(client, company, &identity).await?,
                AtsProvider::Lever => fetch_lever(client, company, &identity).await?,
                AtsProvider::Ashby => fetch_ashby(client, company, &identity).await?,
                AtsProvider::SmartRecruiters => {
                    fetch_smartrecruiters(client, company, &identity, options).await?
                }
                AtsProvider::Workable => fetch_workable(client, company, &identity).await?,
                AtsProvider::Recruitee => fetch_recruitee(client, company, &identity).await?,
                AtsProvider::Personio => fetch_personio(client, company, &identity).await?,
                AtsProvider::BambooHr => fetch_bamboohr(client, company, &identity).await?,
                AtsProvider::Workday => fetch_workday(client, company, &identity, options).await?,
            };
            Ok(to_fetched_jobs(identity, jobs))
        },
    )
    .await
}

pub(super) async fn get_text(
    client: &reqwest::Client,
    identity: &AtsSourceIdentity,
    url: &str,
) -> Result<String, AtsFetchError> {
    request_text(client, identity, reqwest::Method::GET, url, None).await
}

pub(super) async fn get_json(
    client: &reqwest::Client,
    identity: &AtsSourceIdentity,
    url: &str,
) -> Result<Value, AtsFetchError> {
    let text = get_text(client, identity, url).await?;
    serde_json::from_str(&text).map_err(|error| invalid_feed(identity, url, error.to_string()))
}

pub(super) async fn post_json(
    client: &reqwest::Client,
    identity: &AtsSourceIdentity,
    url: &str,
    body: &Value,
) -> Result<Value, AtsFetchError> {
    let text = request_text(client, identity, reqwest::Method::POST, url, Some(body)).await?;
    serde_json::from_str(&text).map_err(|error| invalid_feed(identity, url, error.to_string()))
}

pub(super) async fn request_text(
    client: &reqwest::Client,
    identity: &AtsSourceIdentity,
    method: reqwest::Method,
    url: &str,
    body: Option<&Value>,
) -> Result<String, AtsFetchError> {
    let mut request = client
        .request(method, url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT, "application/json,text/xml,*/*");
    if let Some(body) = body {
        request = request
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(body);
    }
    let response = request
        .send()
        .await
        .map_err(|error| AtsFetchError::Request {
            provider: identity.provider,
            company: identity.company_name.clone(),
            url: url.to_owned(),
            message: request_error_message(&error),
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(AtsFetchError::Http {
            provider: identity.provider,
            company: identity.company_name.clone(),
            url: url.to_owned(),
            status,
        });
    }
    response
        .text()
        .await
        .map_err(|error| AtsFetchError::Request {
            provider: identity.provider,
            company: identity.company_name.clone(),
            url: url.to_owned(),
            message: request_error_message(&error),
        })
}

pub(super) fn request_error_message(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        format!("request timed out: {error}")
    } else {
        error.to_string()
    }
}
