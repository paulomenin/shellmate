use serde_json::Value;

/// The model ids in the reply of a model-list call (`GET .../models`).
///
/// Reads the usual shape `{"data": [{"id": "..."}]}`, and also a bare array, or `{"models": [...]}`, of
/// objects with an `id`. A reply in any other shape gives an empty list. That means "cannot say", not "the
/// server has no models": callers must not treat a missing name as proof that a model does not exist.
pub fn parse_model_ids(body: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    let list = match &v {
        Value::Array(a) => Some(a),
        other => other["data"].as_array().or(other["models"].as_array()),
    };
    list.map(|items| {
        items
            .iter()
            .filter_map(|m| m["id"].as_str())
            .map(str::to_owned)
            .collect()
    })
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_usual_shapes_are_read() {
        assert_eq!(
            parse_model_ids(r#"{"object":"list","data":[{"id":"a"},{"id":"b/c","x":1}]}"#),
            ["a", "b/c"]
        );
        assert_eq!(parse_model_ids(r#"[{"id":"a"}]"#), ["a"]);
        assert_eq!(parse_model_ids(r#"{"models":[{"id":"a"}]}"#), ["a"]);
    }

    #[test]
    fn an_unreadable_reply_is_an_empty_list_not_an_error() {
        for body in [
            "",
            "not json",
            "{}",
            r#"{"data":"x"}"#,
            r#"{"data":[1,{"name":"n"}]}"#,
        ] {
            assert!(parse_model_ids(body).is_empty(), "{body:?}");
        }
    }
}
