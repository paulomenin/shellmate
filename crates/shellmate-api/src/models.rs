use crate::ToolSupport;
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

/// What a model-list reply says about native tool calling for `model`.
///
/// Servers that say anything put it on the model's entry, in one of two shapes: `supported_parameters`
/// (OpenRouter) lists `"tools"` among the parameters the model accepts, and `capabilities` (LM Studio,
/// Ollama) lists `"tool_use"`, `"tools"` or `"function_calling"`. An entry that has one of those lists says
/// yes or no by it. An entry without one, a model that is not listed and a reply in another shape are
/// [`ToolSupport::Unknown`]: not knowing is never read as no.
pub fn parse_model_tool_support(body: &str, model: &str) -> ToolSupport {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return ToolSupport::Unknown;
    };
    let list = match &v {
        Value::Array(a) => Some(a),
        other => other["data"].as_array().or(other["models"].as_array()),
    };
    let Some(entry) = list.and_then(|l| l.iter().find(|m| m["id"].as_str() == Some(model))) else {
        return ToolSupport::Unknown;
    };
    let names = |key: &str| -> Option<Vec<&str>> {
        entry[key]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
    };
    if let Some(params) = names("supported_parameters") {
        return if params.contains(&"tools") {
            ToolSupport::Yes
        } else {
            ToolSupport::No
        };
    }
    if let Some(caps) = names("capabilities") {
        return if caps
            .iter()
            .any(|c| matches!(*c, "tools" | "tool_use" | "function_calling"))
        {
            ToolSupport::Yes
        } else {
            ToolSupport::No
        };
    }
    ToolSupport::Unknown
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

    #[test]
    fn what_a_listing_says_about_tools_is_read_for_the_named_model() {
        use ToolSupport::*;
        let body = r#"{"data":[
            {"id":"a/yes","supported_parameters":["temperature","tools","tool_choice"]},
            {"id":"a/no","supported_parameters":["temperature"]},
            {"id":"l/yes","capabilities":["completion","tool_use"]},
            {"id":"l/also","capabilities":["tools"]},
            {"id":"l/no","capabilities":["completion"]},
            {"id":"plain"}
        ]}"#;
        for (model, want) in [
            ("a/yes", Yes),
            ("a/no", No),
            ("l/yes", Yes),
            ("l/also", Yes),
            ("l/no", No),
            // no word on tools, or not listed: not known
            ("plain", Unknown),
            ("missing", Unknown),
        ] {
            assert_eq!(parse_model_tool_support(body, model), want, "{model}");
        }
        // another shape, or no reply at all: not known
        for body in [
            "",
            "not json",
            "{}",
            r#"{"data":"x"}"#,
            r#"[{"id":"plain"}]"#,
        ] {
            assert_eq!(parse_model_tool_support(body, "plain"), Unknown, "{body:?}");
        }
        // a bare array works as for the ids
        assert_eq!(
            parse_model_tool_support(r#"[{"id":"m","capabilities":["tools"]}]"#, "m"),
            Yes
        );
    }
}
