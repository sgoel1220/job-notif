use axum::Json;
use quick_xml::{events::Event, name::ResolveResult, reader::NsReader};
use serde::Serialize;

use crate::{errors::ApiError, AppState};

const JOBS_URL: &str = "https://37signals.com/feed/jobs.xml";
const ATOM_NS: &[u8] = b"http://www.w3.org/2005/Atom";

#[derive(Default, Debug, PartialEq)]
struct Job {
    id: String,
    title: String,
    url: String,
    text: String,
}

#[derive(Serialize)]
pub(crate) struct SyncResult {
    scanned: usize,
    matched: usize,
    message: &'static str,
}

fn resolved_namespace(resolve: ResolveResult<'_>) -> Result<Option<Vec<u8>>, String> {
    match resolve {
        ResolveResult::Unbound => Ok(None),
        ResolveResult::Bound(namespace) => Ok(Some(namespace.into_inner().to_vec())),
        ResolveResult::Unknown(prefix) => Err(format!(
            "undeclared XML namespace prefix: {}",
            String::from_utf8_lossy(&prefix)
        )),
    }
}

fn atom_element(namespace: Option<&[u8]>, name: &[u8]) -> bool {
    namespace == Some(ATOM_NS) && name != b""
}

fn append_reference(reference: &quick_xml::events::BytesRef<'_>) -> Result<String, String> {
    if let Some(character) = reference
        .resolve_char_ref()
        .map_err(|error| error.to_string())?
    {
        return Ok(character.to_string());
    }
    let name = reference.decode().map_err(|error| error.to_string())?;
    match name.as_ref() {
        "amp" => Ok("&".into()),
        "lt" => Ok("<".into()),
        "gt" => Ok(">".into()),
        "apos" => Ok("'".into()),
        "quot" => Ok("\"".into()),
        _ => Err(format!("unknown XML entity reference: &{name};")),
    }
}

fn http_link(href: &str, base: &reqwest::Url) -> Result<reqwest::Url, String> {
    let url = base.join(href.trim()).map_err(|error| error.to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("Atom links must resolve to HTTP(S) URLs".into());
    }
    Ok(url)
}

fn element_base(
    element: &quick_xml::events::BytesStart<'_>,
    parent: &reqwest::Url,
    decoder: quick_xml::encoding::Decoder,
) -> Result<reqwest::Url, String> {
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|error| error.to_string())?;
        if attribute.key.as_ref() == b"xml:base" {
            let value = attribute
                .decode_and_unescape_value(decoder)
                .map_err(|error| error.to_string())?;
            return http_link(&value, parent);
        }
    }
    Ok(parent.clone())
}

fn parse_feed(xml: &str) -> Result<Vec<Job>, String> {
    let mut reader = NsReader::from_str(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = true;

    // Each frame stores local name and its resolved namespace, enabling direct-child
    // checks and rejecting trailing/nested roots before any snapshot is reconciled.
    let mut stack: Vec<(String, Option<Vec<u8>>)> = Vec::new();
    let document_base = reqwest::Url::parse(JOBS_URL).map_err(|error| error.to_string())?;
    let mut bases: Vec<reqwest::Url> = Vec::new();
    let mut jobs = Vec::new();
    let mut current: Option<Job> = None;
    let mut entry_depth = None;
    let mut field: Option<String> = None;
    let mut root_seen = false;
    let mut root_closed = false;

    loop {
        let (resolve, event) = reader
            .read_resolved_event()
            .map_err(|error| error.to_string())?;
        let namespace = resolved_namespace(resolve)?;
        match event {
            Event::Start(ref element) => {
                let base = element_base(
                    element,
                    bases.last().unwrap_or(&document_base),
                    reader.decoder(),
                )?;
                let name = element.local_name();
                let local = name.as_ref();
                if stack.is_empty() {
                    if root_seen
                        || root_closed
                        || local != b"feed"
                        || !atom_element(namespace.as_deref(), b"feed")
                    {
                        return Err("document must have exactly one Atom feed root".into());
                    }
                    root_seen = true;
                } else if root_closed {
                    return Err("content after Atom feed root".into());
                } else if local == b"feed" && atom_element(namespace.as_deref(), b"feed") {
                    return Err("nested Atom feed root".into());
                }

                if local == b"entry"
                    && atom_element(namespace.as_deref(), b"entry")
                    && stack.len() != 1
                {
                    return Err("Atom entries must be direct children of the feed".into());
                }
                if stack.len() == 1
                    && local == b"entry"
                    && atom_element(namespace.as_deref(), b"entry")
                {
                    if current.is_some() {
                        return Err("nested Atom entry".into());
                    }
                    current = Some(Job::default());
                    entry_depth = Some(stack.len() + 1);
                } else if current.is_some()
                    && entry_depth == Some(stack.len())
                    && atom_element(namespace.as_deref(), local)
                {
                    if local == b"id" || local == b"title" {
                        field = Some(String::from_utf8_lossy(local).into_owned());
                    }
                    if local == b"link" {
                        let mut href = None;
                        let mut rel = None;
                        for attribute in element.attributes() {
                            let attribute = attribute.map_err(|error| error.to_string())?;
                            let value = attribute
                                .decode_and_unescape_value(reader.decoder())
                                .map_err(|error| error.to_string())?
                                .into_owned();
                            match attribute.key.local_name().as_ref() {
                                b"href" => href = Some(value),
                                b"rel" => rel = Some(value),
                                _ => {}
                            }
                        }
                        if rel.as_deref().is_none_or(|rel| rel == "alternate") {
                            if let Some(href) = href.filter(|href| !href.trim().is_empty()) {
                                current.as_mut().unwrap().url = http_link(&href, &base)?.into();
                            }
                        }
                    }
                }
                stack.push((String::from_utf8_lossy(local).into_owned(), namespace));
                bases.push(base);
            }
            Event::Empty(ref element) => {
                let base = element_base(
                    element,
                    bases.last().unwrap_or(&document_base),
                    reader.decoder(),
                )?;
                let name = element.local_name();
                let local = name.as_ref();
                if stack.is_empty() {
                    if root_seen
                        || root_closed
                        || local != b"feed"
                        || !atom_element(namespace.as_deref(), b"feed")
                    {
                        return Err("document must have exactly one Atom feed root".into());
                    }
                    root_seen = true;
                    root_closed = true;
                } else if root_closed {
                    return Err("content after Atom feed root".into());
                } else if local == b"feed" && atom_element(namespace.as_deref(), b"feed") {
                    return Err("nested Atom feed root".into());
                } else if local == b"entry" && atom_element(namespace.as_deref(), b"entry") {
                    return Err("empty Atom entry is incomplete".into());
                } else if current.is_some()
                    && entry_depth == Some(stack.len())
                    && local == b"link"
                    && atom_element(namespace.as_deref(), b"link")
                {
                    let mut href = None;
                    let mut rel = None;
                    for attribute in element.attributes() {
                        let attribute = attribute.map_err(|error| error.to_string())?;
                        let value = attribute
                            .decode_and_unescape_value(reader.decoder())
                            .map_err(|error| error.to_string())?
                            .into_owned();
                        match attribute.key.local_name().as_ref() {
                            b"href" => href = Some(value),
                            b"rel" => rel = Some(value),
                            _ => {}
                        }
                    }
                    if rel.as_deref().is_none_or(|rel| rel == "alternate") {
                        if let Some(href) = href.filter(|href| !href.trim().is_empty()) {
                            current.as_mut().unwrap().url = http_link(&href, &base)?.into();
                        }
                    }
                } else if current.is_some()
                    && entry_depth == Some(stack.len())
                    && (local == b"entry" || local == b"id" || local == b"title")
                {
                    return Err("empty Atom entry/id/title is incomplete".into());
                }
            }
            Event::End(ref element) => {
                let name = element.local_name();
                let local = name.as_ref();
                let (open_name, open_namespace) = stack.pop().ok_or("unexpected XML close tag")?;
                bases.pop();
                if open_name.as_bytes() != local || open_namespace != namespace {
                    return Err("mismatched XML element namespace or name".into());
                }
                if current.is_some() && atom_element(namespace.as_deref(), local) {
                    if entry_depth == Some(stack.len()) && (local == b"id" || local == b"title") {
                        field = None;
                    }
                    if entry_depth == Some(stack.len() + 1) && local == b"entry" {
                        let mut job = current
                            .take()
                            .ok_or("closing Atom entry without an open entry")?;
                        job.id = job.id.trim().to_owned();
                        job.title = job.title.trim().to_owned();
                        job.url = job.url.trim().to_owned();
                        if job.id.is_empty() || job.title.is_empty() || job.url.is_empty() {
                            return Err(
                                "Atom entry missing required id, title, or alternate link".into()
                            );
                        }
                        if jobs.iter().any(|existing: &Job| existing.id == job.id) {
                            return Err(format!("duplicate Atom entry id: {}", job.id));
                        }
                        jobs.push(job);
                        entry_depth = None;
                    }
                }
                if stack.is_empty() {
                    if local != b"feed"
                        || !atom_element(namespace.as_deref(), b"feed")
                        || !root_seen
                        || root_closed
                    {
                        return Err("document must have exactly one Atom feed root".into());
                    }
                    root_closed = true;
                }
            }
            Event::Text(ref text) => {
                let decoded = text.decode().map_err(|error| error.to_string())?;
                let value =
                    quick_xml::escape::unescape(&decoded).map_err(|error| error.to_string())?;
                if stack.is_empty() {
                    if !value.trim().is_empty() {
                        return Err("non-whitespace text outside Atom feed root".into());
                    }
                } else if stack.len() == 1 && !value.trim().is_empty() {
                    return Err("non-whitespace text directly inside Atom feed".into());
                } else if let Some(job) = current.as_mut() {
                    if let Some(active_field) = field.as_deref() {
                        match active_field {
                            "id" => job.id.push_str(&value),
                            "title" => job.title.push_str(&value),
                            _ => {}
                        }
                    }
                    if !value.trim().is_empty() {
                        job.text.push(' ');
                        job.text.push_str(value.trim());
                    }
                }
            }
            Event::CData(ref text) => {
                if stack.len() <= 1 {
                    return Err("CDATA outside an Atom child element".into());
                }
                let value = text.decode().map_err(|error| error.to_string())?;
                if let Some(job) = current.as_mut() {
                    if let Some(active_field) = field.as_deref() {
                        match active_field {
                            "id" => job.id.push_str(&value),
                            "title" => job.title.push_str(&value),
                            _ => {}
                        }
                    }
                    job.text.push_str(&value);
                }
            }
            Event::GeneralRef(ref reference) => {
                if stack.len() <= 1 {
                    return Err("entity reference outside an Atom child element".into());
                }
                let value = append_reference(reference)?;
                if let Some(job) = current.as_mut() {
                    if let Some(active_field) = field.as_deref() {
                        match active_field {
                            "id" => job.id.push_str(&value),
                            "title" => job.title.push_str(&value),
                            _ => {}
                        }
                    }
                    job.text.push_str(&value);
                }
            }
            Event::DocType(_) => return Err("DOCTYPE is not permitted in the Atom feed".into()),
            Event::Eof => break,
            Event::Decl(_) | Event::PI(_) | Event::Comment(_) => {
                if root_closed && !stack.is_empty() {
                    return Err("invalid trailing XML content".into());
                }
            }
        }
    }

    if !root_seen || !root_closed || !stack.is_empty() || current.is_some() {
        return Err("incomplete Atom feed".into());
    }
    Ok(jobs)
}

fn is_target_role(job: &Job) -> bool {
    let title = job.title.to_lowercase();
    let context = job.text.to_lowercase();
    let software = [
        "software",
        "engineer",
        "engineering",
        "developer",
        "devops",
        "sre",
        "quality assurance",
        " qa ",
    ]
    .iter()
    .any(|term| title.contains(term));
    let remote = context.contains("remote");
    let eligible = ["india", "global", "worldwide", "anywhere"]
        .iter()
        .any(|term| context.contains(term));
    software && remote && eligible
}

fn normalize_job(job: &Job) -> crate::job_postings::JobPosting {
    let mut posting = crate::job_postings::JobPosting::basic(
        "37signals-atom",
        &job.id,
        job.title.clone(),
        "37signals",
        None,
        None,
        job.url.clone(),
        serde_json::json!({"feedText": job.text}),
    );
    posting.description_text = Some(job.text.clone());
    posting
}

pub(crate) async fn sync(
    state: AppState,
    client: &reqwest::Client,
) -> Result<Json<SyncResult>, ApiError> {
    let response = client
        .get(JOBS_URL)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| {
            eprintln!("37signals jobs request failed: {error}");
            ApiError(
                axum::http::StatusCode::BAD_GATEWAY,
                "Could not fetch 37signals jobs.".into(),
            )
        })?;
    let xml = response.text().await.map_err(|error| {
        eprintln!("37signals jobs response failed: {error}");
        ApiError(
            axum::http::StatusCode::BAD_GATEWAY,
            "Could not read 37signals jobs.".into(),
        )
    })?;
    let jobs = parse_feed(&xml).map_err(|error| {
        eprintln!("37signals Atom feed was invalid: {error}");
        ApiError(
            axum::http::StatusCode::BAD_GATEWAY,
            "Could not parse 37signals jobs.".into(),
        )
    })?;
    let scanned = jobs.len();
    let normalized: Vec<_> = jobs.iter().map(normalize_job).collect();
    crate::job_postings::replace_source_snapshot(
        &state.db,
        "37signals",
        "37signals-atom",
        &normalized,
    )
    .await?;
    let matched = jobs.iter().filter(|job| is_target_role(job)).count();
    Ok(Json(SyncResult { scanned, matched, message: "37signals Atom feed synced; location and work arrangement are only set when stated in the feed." }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREFIXED: &str = r#"<a:feed xmlns:a="http://www.w3.org/2005/Atom"><a:entry><a:id>tag:example:1</a:id><a:title type="xhtml"><div xmlns="http://www.w3.org/1999/xhtml">Software &amp; Platform Engineer</div></a:title><a:link rel="self" href="https://example.test/api/1"/><a:link rel="alternate" href="https://example.test/jobs/1"></a:link><a:summary><![CDATA[Remote worldwide]]></a:summary></a:entry></a:feed>"#;

    #[test]
    fn prefixed_atom_fields_links_entities_and_xhtml_are_decoded() {
        let jobs = parse_feed(PREFIXED).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, "tag:example:1");
        assert_eq!(jobs[0].title, "Software & Platform Engineer");
        assert_eq!(jobs[0].url, "https://example.test/jobs/1");
        assert!(is_target_role(&jobs[0]));
    }

    #[test]
    fn cdata_is_preserved_in_id_and_title() {
        let xml = "<feed xmlns='http://www.w3.org/2005/Atom'><entry><id><![CDATA[tag:example:2]]></id><title><![CDATA[QA Engineer]]></title><link href='https://example.test/2'/></entry></feed>";
        let job = parse_feed(xml).unwrap().remove(0);
        assert_eq!(job.id, "tag:example:2");
        assert_eq!(job.title, "QA Engineer");
    }

    #[test]
    fn rejects_non_atom_incomplete_trailing_and_duplicate_documents() {
        for xml in [
            "",
            "<html><body>Maintenance</body></html>",
            "<feed xmlns='urn:not-atom'></feed>",
            "<feed xmlns='http://www.w3.org/2005/Atom'><entry><id>1</id>",
            "<feed xmlns='http://www.w3.org/2005/Atom'><entry><id>1</id><title>T</title></entry></feed>",
            "<feed xmlns='http://www.w3.org/2005/Atom'></feed><entry/>",
            "<feed xmlns='http://www.w3.org/2005/Atom'></feed><x:feed xmlns:x='http://www.w3.org/2005/Atom'/>",
            "<feed xmlns='http://www.w3.org/2005/Atom'>tail</feed>",
            "<feed xmlns='http://www.w3.org/2005/Atom'><entry><entry><id>1</id><title>T</title><link href='https://example.test'/></entry></entry></feed>",
            "<feed xmlns='http://www.w3.org/2005/Atom'><meta><feed/></meta></feed>",
            "<feed xmlns='http://www.w3.org/2005/Atom'><entry/></feed>",
            "<empty/><feed xmlns='http://www.w3.org/2005/Atom'/>",
            "preamble<feed xmlns='http://www.w3.org/2005/Atom'/>",
            "<feed xmlns='http://www.w3.org/2005/Atom'/><empty/>",
            "<feed xmlns='http://www.w3.org/2005/Atom'><entry><id>&bogus;</id><title>T</title><link href='https://example.test'/></entry></feed>",
            "<feed xmlns='http://www.w3.org/2005/Atom'><entry><id>1</id><title>T</title><link href='https://example.test'/></entry><entry><id>1</id><title>T2</title><link href='https://example.test/2'/></entry></feed>",
        ] {
            assert!(parse_feed(xml).is_err(), "accepted invalid fixture: {xml}");
        }
    }

    #[test]
    fn alternate_links_resolve_xml_base_and_reject_unsafe_schemes() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom" xml:base="https://example.test/careers/"><entry xml:base="engineering/"><id>1</id><title>Engineer</title><link href="42"/></entry></feed>"#;
        assert_eq!(
            parse_feed(xml).unwrap()[0].url,
            "https://example.test/careers/engineering/42"
        );
        let relative = r#"<feed xmlns="http://www.w3.org/2005/Atom"><entry><id>1</id><title>Engineer</title><link href="/jobs/42"/></entry></feed>"#;
        assert_eq!(
            parse_feed(relative).unwrap()[0].url,
            "https://37signals.com/jobs/42"
        );
        for url in [
            "javascript:alert(1)",
            "data:text/html,unsafe",
            "file:///tmp/job",
        ] {
            let unsafe_feed = relative.replace("/jobs/42", url);
            assert!(parse_feed(&unsafe_feed).is_err());
        }
        assert!(parse_feed(
            r#"<feed xmlns="http://www.w3.org/2005/Atom"><![CDATA[unexpected]]></feed>"#
        )
        .is_err());
        assert!(parse_feed(r#"<feed xmlns="http://www.w3.org/2005/Atom">&amp;</feed>"#).is_err());
    }

    #[test]
    fn accepts_empty_atom_with_explicit_or_prefixed_namespace() {
        assert!(parse_feed("<feed xmlns='http://www.w3.org/2005/Atom'/>")
            .unwrap()
            .is_empty());
        assert!(
            parse_feed("<a:feed xmlns:a='http://www.w3.org/2005/Atom'/>")
                .unwrap()
                .is_empty()
        );
    }
}
