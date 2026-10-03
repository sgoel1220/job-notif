use super::{
    dedupe, get_text, invalid_feed, percent_encode_component, workplace_type, AtsCompanySource,
    AtsFetchError, AtsSourceIdentity, NormalizedAtsJob,
};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::json;

pub(super) async fn fetch_personio(
    client: &reqwest::Client,
    company: &AtsCompanySource,
    identity: &AtsSourceIdentity,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let slug = company.slug()?;
    let url = format!(
        "https://{}.jobs.personio.de/xml",
        percent_encode_component(slug)
    );
    let xml = get_text(client, identity, &url).await?;
    map_personio_jobs(identity, &url, &xml)
}

pub(super) fn map_personio_jobs(
    identity: &AtsSourceIdentity,
    url: &str,
    xml: &str,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let positions = parse_personio_xml(identity, url, xml)?;
    Ok(positions
        .into_iter()
        .map(|position| NormalizedAtsJob {
            id: position.id.clone(),
            title: position.name.clone(),
            department: position.department.clone(),
            workplace_type: workplace_type(&position.locations, None, false),
            employment_type: position.employment_type.clone(),
            apply_url: format!(
                "https://{}.jobs.personio.de/job/{}",
                identity.source_ref,
                percent_encode_component(&position.id)
            ),
            posted_at: position.created_at.clone(),
            locations: position.locations.clone(),
            details: json!({
                "id": position.id,
                "name": position.name,
                "department": position.department,
                "employmentType": position.employment_type,
                "locations": position.locations,
                "createdAt": position.created_at,
            }),
        })
        .collect())
}

#[derive(Default)]
pub(super) struct PersonioPosition {
    id: String,
    name: String,
    department: Option<String>,
    employment_type: Option<String>,
    locations: Vec<String>,
    created_at: Option<String>,
}

pub(super) fn parse_personio_xml(
    identity: &AtsSourceIdentity,
    url: &str,
    xml: &str,
) -> Result<Vec<PersonioPosition>, AtsFetchError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut positions = Vec::new();
    let mut current: Option<PersonioPosition> = None;
    let mut stack: Vec<String> = Vec::new();
    let mut root_closed = false;
    let mut field_text = String::new();
    let mut field_tag: Option<String> = None;
    loop {
        let event = reader
            .read_event()
            .map_err(|error| invalid_feed(identity, url, error.to_string()))?;
        match event {
            Event::Start(element) => {
                let tag = String::from_utf8_lossy(element.name().as_ref()).into_owned();
                if stack.is_empty() {
                    if root_closed || tag != "workzag-jobs" {
                        return Err(invalid_feed(
                            identity,
                            url,
                            "expected one workzag-jobs root element",
                        ));
                    }
                } else if stack == ["workzag-jobs"] {
                    if tag != "position" {
                        return Err(invalid_feed(
                            identity,
                            url,
                            "unexpected element beneath Personio root",
                        ));
                    }
                    current = Some(PersonioPosition::default());
                } else if stack == ["workzag-jobs", "position"] {
                    field_tag = Some(tag.clone());
                    field_text.clear();
                } else {
                    return Err(invalid_feed(
                        identity,
                        url,
                        "nested markup inside a Personio text field is invalid",
                    ));
                }
                stack.push(tag);
            }
            Event::Empty(element) => {
                let tag = String::from_utf8_lossy(element.name().as_ref()).into_owned();
                if stack.is_empty() {
                    if root_closed || tag != "workzag-jobs" {
                        return Err(invalid_feed(
                            identity,
                            url,
                            "expected one workzag-jobs root element",
                        ));
                    }
                    root_closed = true;
                } else if stack == ["workzag-jobs"] {
                    if tag != "position" {
                        return Err(invalid_feed(
                            identity,
                            url,
                            "unexpected element beneath Personio root",
                        ));
                    }
                    return Err(invalid_feed(
                        identity,
                        url,
                        "Personio position missing mandatory id/name",
                    ));
                } else if stack == ["workzag-jobs", "position"] {
                    field_tag = Some(tag);
                    field_text.clear();
                    commit_personio_field(
                        identity,
                        url,
                        current.as_mut().ok_or_else(|| {
                            invalid_feed(identity, url, "position field outside position")
                        })?,
                        field_tag.as_deref().unwrap(),
                        "",
                    )?;
                    field_tag = None;
                } else {
                    return Err(invalid_feed(
                        identity,
                        url,
                        "nested empty markup inside Personio field",
                    ));
                }
            }
            Event::Text(text) => {
                let decoded = text
                    .decode()
                    .map_err(|error| invalid_feed(identity, url, error.to_string()))?;
                let decoded = quick_xml::escape::unescape(&decoded)
                    .map_err(|error| invalid_feed(identity, url, error.to_string()))?;
                if field_tag.is_some() {
                    field_text.push_str(&decoded);
                } else if !decoded.trim().is_empty() {
                    return Err(invalid_feed(
                        identity,
                        url,
                        "unexpected text outside a Personio field",
                    ));
                }
            }
            Event::CData(text) => {
                let decoded = text
                    .decode()
                    .map_err(|error| invalid_feed(identity, url, error.to_string()))?;
                if field_tag.is_some() {
                    field_text.push_str(&decoded);
                } else if !decoded.trim().is_empty() {
                    return Err(invalid_feed(
                        identity,
                        url,
                        "unexpected CDATA outside a Personio field",
                    ));
                }
            }
            Event::GeneralRef(reference) => {
                let name = reference
                    .decode()
                    .map_err(|error| invalid_feed(identity, url, error.to_string()))?;
                let resolved = if let Some(character) = reference
                    .resolve_char_ref()
                    .map_err(|error| invalid_feed(identity, url, error.to_string()))?
                {
                    character.to_string()
                } else {
                    quick_xml::escape::resolve_predefined_entity(&name)
                        .ok_or_else(|| {
                            invalid_feed(identity, url, format!("undeclared XML entity &{name};"))
                        })?
                        .to_owned()
                };
                if field_tag.is_some() {
                    field_text.push_str(&resolved);
                } else {
                    return Err(invalid_feed(
                        identity,
                        url,
                        "entity reference outside a Personio field",
                    ));
                }
            }
            Event::End(element) => {
                let tag = String::from_utf8_lossy(element.name().as_ref()).into_owned();
                if stack.last().map(String::as_str) != Some(tag.as_str()) {
                    return Err(invalid_feed(
                        identity,
                        url,
                        "mismatched Personio XML closing tag",
                    ));
                }
                if stack.len() == 3 {
                    let position = current.as_mut().ok_or_else(|| {
                        invalid_feed(identity, url, "field close outside Personio position")
                    })?;
                    commit_personio_field(identity, url, position, &tag, field_text.trim())?;
                    field_tag = None;
                    field_text.clear();
                } else if stack.len() == 2 && tag == "position" {
                    let mut position = current.take().ok_or_else(|| {
                        invalid_feed(identity, url, "unexpected Personio position close")
                    })?;
                    dedupe(&mut position.locations);
                    if position.id.trim().is_empty() || position.name.trim().is_empty() {
                        return Err(invalid_feed(
                            identity,
                            url,
                            "Personio position missing mandatory id/name",
                        ));
                    }
                    positions.push(position);
                } else if stack.len() == 1 && tag == "workzag-jobs" {
                    root_closed = true;
                }
                stack.pop();
            }
            Event::Eof => break,
            Event::Decl(_) | Event::Comment(_) | Event::PI(_) => {}
            Event::DocType(_) => {
                return Err(invalid_feed(
                    identity,
                    url,
                    "DOCTYPE is not allowed in Personio XML",
                ))
            }
        }
    }
    if !root_closed || !stack.is_empty() || current.is_some() {
        return Err(invalid_feed(
            identity,
            url,
            "incomplete Personio XML document",
        ));
    }
    Ok(positions)
}

pub(super) fn commit_personio_field(
    identity: &AtsSourceIdentity,
    url: &str,
    position: &mut PersonioPosition,
    tag: &str,
    value: &str,
) -> Result<(), AtsFetchError> {
    let value = value.trim();
    match tag {
        "id" => position.id = value.to_owned(),
        "name" => position.name = value.to_owned(),
        "department" if !value.is_empty() => position.department = Some(value.to_owned()),
        "employmentType" if !value.is_empty() => position.employment_type = Some(value.to_owned()),
        "createdAt" if !value.is_empty() => position.created_at = Some(value.to_owned()),
        "office" if !value.is_empty() => position.locations.extend(
            value
                .split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned),
        ),
        _ => {}
    }
    let _ = (identity, url);
    Ok(())
}
