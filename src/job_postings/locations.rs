use super::normalization::{nested_string, value_to_string};

pub(super) fn merge_locations(
    location: Option<String>,
    value: &serde_json::Value,
) -> Option<String> {
    let mut all = Vec::<String>::new();
    if let Some(location) = location {
        all.push(location);
    } else if let Some(primary) = nested_string(value, &["categories", "location"]) {
        all.push(primary);
    }
    if let Some(primary_address) = value.pointer("/address/postalAddress") {
        if let Some(text) = location_item_text(primary_address) {
            all.push(text);
        }
    }
    for key in ["locations", "secondaryLocations", "offices"] {
        if let Some(items) = value.get(key).and_then(serde_json::Value::as_array) {
            for item in items {
                let text = location_item_text(item);
                if let Some(text) = text {
                    all.push(text);
                }
            }
        }
    }
    if let Some(primary) = nested_string(value, &["categories", "location"]) {
        all.push(primary);
    }
    if let Some(items) = value
        .pointer("/categories/allLocations")
        .and_then(serde_json::Value::as_array)
    {
        all.extend(items.iter().filter_map(value_to_string));
    }
    let mut seen = std::collections::HashSet::new();
    all.retain(|item| seen.insert(item.to_lowercase()));
    let complete = all.clone();
    all.retain(|item| {
        !complete.iter().any(|other| {
            !other.eq_ignore_ascii_case(item)
                && other
                    .split([',', ';'])
                    .any(|part| part.trim().eq_ignore_ascii_case(item.trim()))
        })
    });
    (!all.is_empty()).then(|| all.join("; "))
}

fn location_item_text(item: &serde_json::Value) -> Option<String> {
    if let Some(text) = value_to_string(item) {
        return Some(text);
    }
    let mut parts = Vec::new();
    'paths: for path in [
        &["name"][..],
        &["location"][..],
        &["addressLocality"][..],
        &["addressRegion"][..],
        &["addressCountry"][..],
        &["address", "postalAddress"][..],
        &["address", "postalAddress", "addressLocality"][..],
        &["address", "postalAddress", "addressRegion"][..],
        &["address", "postalAddress", "addressCountry", "name"][..],
        &["address", "postalAddress", "addressCountry"][..],
        &["address", "addressLocality"][..],
        &["address", "addressRegion"][..],
        &["address", "addressCountry", "name"][..],
        &["address", "addressCountry"][..],
        &["addressCountry", "name"][..],
        &["addressCountry"][..],
        &["country"][..],
        &["city"][..],
    ] {
        let mut current = item;
        for key in path {
            current = match current.get(*key) {
                Some(v) => v,
                None => continue 'paths,
            };
        }
        if let Some(text) = value_to_string(current) {
            parts.push(text);
        }
    }
    let mut seen = std::collections::HashSet::new();
    parts.retain(|part| seen.insert(part.to_lowercase()));
    (!parts.is_empty()).then(|| parts.join(", "))
}
