//! Independent first-party producer guard. Legacy renderer fallback remains
//! readable, but newly produced letter controls must carry usable metadata.

use radar_core::view::{ActionSpec, Block, ChromeView, Key};
use serde_json::Value;

pub fn key_gap(keys: &str, words: &str, at: Option<usize>, shift: bool) -> Option<String> {
    let mut chars = keys.chars();
    let letter = chars
        .next()
        .filter(|c| c.is_alphabetic() && chars.next().is_none());
    if let Some(letter) = letter {
        let Some(position) = at else {
            return Some(format!("{keys:?} / {words:?}: missing mnemonic"));
        };
        let Some(_) = words.chars().nth(position) else {
            return Some(format!(
                "{keys:?} / {words:?}: out-of-range mnemonic {position}"
            ));
        };
        let wanted: String = letter.to_lowercase().collect();
        let suffix: String = words.chars().skip(position).collect();
        if !suffix.to_lowercase().starts_with(&wanted) || shift != letter.is_uppercase() {
            return Some(format!(
                "{keys:?} / {words:?}: wrong letter or Shift at {position}"
            ));
        }
    } else {
        if at.is_some() || shift {
            return Some(format!("nonletter {keys:?} has mnemonic metadata"));
        }
        if keys.contains('/')
            && keys.split('/').any(|part| {
                let mut chars = part.trim().chars();
                chars.next().is_some_and(|c| c.is_alphabetic()) && chars.next().is_none()
            })
        {
            return Some(format!(
                "composite letter alias {keys:?} must use meaningful controls"
            ));
        }
    }
    None
}

pub fn chrome_gaps(chrome: &ChromeView) -> Vec<String> {
    let mut gaps = Vec::new();
    let hints = chrome
        .status
        .hints
        .iter()
        .chain(chrome.history.iter().flat_map(|h| &h.footer));
    for hint in hints {
        if let Some(gap) = key_gap(
            &hint.keys,
            &hint.label,
            hint.mnemonic_at,
            hint.mnemonic_shift,
        ) {
            gaps.push(format!("hint: {gap}"));
        }
    }
    if let Some(menu) = &chrome.menu {
        for choice in &menu.items {
            if choice.heading || choice.note || choice.shown_only {
                if choice.mnemonic_at.is_some() || choice.mnemonic_shift {
                    gaps.push(format!("menu {} shown-only row has a mnemonic", menu.id));
                }
                continue;
            }
            let words = if menu.question && !choice.hint.is_empty() {
                &choice.hint
            } else {
                &choice.label
            };
            if let Some(gap) = key_gap(
                &choice.keys,
                words,
                choice.mnemonic_at,
                choice.mnemonic_shift,
            ) {
                gaps.push(format!("menu {}: {gap}", menu.id));
            }
        }
    }
    gaps
}

fn action_gap(action: &ActionSpec) -> Option<String> {
    let keys = match action.key {
        Some(chord) if !action.heading && !chord.ctrl && !chord.alt => match chord.key {
            Key::Char(c) => c.to_string(),
            _ => String::new(),
        },
        _ => String::new(),
    };
    key_gap(
        &keys,
        &action.label,
        action.mnemonic_at,
        action.mnemonic_shift,
    )
}

pub fn fixture_gaps(value: &Value) -> Vec<String> {
    fn walk(value: &Value, path: &str, gaps: &mut Vec<String>) {
        match value {
            Value::Object(object) => {
                if object.contains_key("top_bar") && object.contains_key("status") {
                    match serde_json::from_value::<ChromeView>(value.clone()) {
                        Ok(chrome) => gaps.extend(
                            chrome_gaps(&chrome)
                                .into_iter()
                                .map(|gap| format!("{path}: {gap}")),
                        ),
                        Err(error) => gaps.push(format!("{path}: invalid chrome: {error}")),
                    }
                }
                if path.ends_with("/foot_actions") {
                    match serde_json::from_value::<radar_core::view::detail::FootActions>(value.clone()) {
                        Ok(foot) => {
                            for (index, item) in foot.actions.iter().chain(std::iter::once(&foot.menu)).enumerate() {
                                if let Some(gap) = action_gap(&item.action) {
                                    gaps.push(format!("{path}/control/{index}: {gap}"));
                                }
                            }
                        }
                        Err(error) => gaps.push(format!("{path}: invalid foot actions: {error}")),
                    }
                } else if let Some(actions) = object.get("actions").and_then(Value::as_array) {
                    for (index, action) in actions.iter().enumerate() {
                        match serde_json::from_value::<ActionSpec>(action.clone()) {
                            Ok(action) => {
                                if let Some(gap) = action_gap(&action) {
                                    gaps.push(format!("{path}/actions/{index}: {gap}"));
                                }
                            }
                            Err(error) => gaps
                                .push(format!("{path}/actions/{index}: invalid action: {error}")),
                        }
                    }
                }
                if object.get("block").and_then(Value::as_str) == Some("line") {
                    match serde_json::from_value::<Block>(value.clone()) {
                        Ok(Block::Line {
                            text,
                            key,
                            mnemonic_at,
                            mnemonic_shift,
                            ..
                        }) => {
                            if let Some(gap) = key_gap(
                                key.as_deref().unwrap_or(""),
                                &text.words(),
                                mnemonic_at,
                                mnemonic_shift,
                            ) {
                                gaps.push(format!("{path}: {gap}"));
                            }
                            if let Some(at) = mnemonic_at {
                                let mut offset = 0;
                                for segment in &text.0 {
                                    let end = offset + segment.text.chars().count();
                                    if (offset..end).contains(&at)
                                        && segment.icon.is_some()
                                        && !segment.chip
                                    {
                                        gaps.push(format!(
                                            "{path}: mnemonic points into replaceable icon fallback"
                                        ));
                                    }
                                    offset = end;
                                }
                            }
                        }
                        Ok(_) => unreachable!("line tag parsed another variant"),
                        Err(error) => gaps.push(format!("{path}: invalid rich line: {error}")),
                    }
                }
                for (key, child) in object {
                    walk(child, &format!("{path}/{key}"), gaps);
                }
            }
            Value::Array(array) => {
                for (index, child) in array.iter().enumerate() {
                    walk(child, &format!("{path}/{index}"), gaps);
                }
            }
            _ => {}
        }
    }
    let mut gaps = Vec::new();
    walk(value, "", &mut gaps);
    gaps
}

#[cfg(test)]
mod detail_footer_tests {
    use super::*;
    use radar_core::{Binding, view::detail::{FootAction, FootActions}};

    #[test]
    fn wrapped_footer_actions_and_menu_keep_the_mnemonic_guard() {
        let foot = FootActions::new(vec![FootAction::new(ActionSpec::named(
            "open", &Binding::new("o", "open"),
        ))]);
        let value = serde_json::json!({"foot_actions": foot});
        assert!(fixture_gaps(&value).is_empty());
        let mut bad = value.clone();
        bad["foot_actions"]["actions"][0]["action"].as_object_mut().unwrap().remove("mnemonic_at");
        assert!(fixture_gaps(&bad).iter().any(|gap| gap.contains("missing mnemonic")));
        let mut bad = value.clone();
        bad["foot_actions"]["menu"]["action"].as_object_mut().unwrap().remove("mnemonic_at");
        assert!(fixture_gaps(&bad).iter().any(|gap| gap.contains("missing mnemonic")));
        let mut bad = value;
        bad["foot_actions"]["actions"][0]["action"].as_object_mut().unwrap().remove("id");
        assert!(fixture_gaps(&bad).iter().any(|gap| gap.contains("invalid foot actions")));
    }
}
