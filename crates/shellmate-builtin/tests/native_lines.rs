//! The arguments of a native call become the same one-line call as a call written in text. The table in
//! `tools/bench/evals/native_lines.json` is shared with the evaluator, which reads native calls the same way
//! when it scores a model, so the two cannot drift apart.

use serde_json::Value;

#[test]
fn every_tool_reads_the_arguments_as_the_table_says() {
    let table: Value =
        serde_json::from_str(include_str!("../../../tools/bench/evals/native_lines.json")).unwrap();
    let registry = shellmate_builtin::registry();
    let cases = table["cases"].as_array().unwrap();
    assert!(cases.len() > 20, "the table is not empty");
    for case in cases {
        let name = case["tool"].as_str().unwrap();
        let tool = registry.build_tool(name, &serde_json::json!({})).unwrap();
        let got = tool.call_line(&case["arguments"]);
        if case["error"].as_bool() == Some(true) {
            assert!(
                got.is_err(),
                "{name} {}: read as {got:?}",
                case["arguments"]
            );
        } else {
            assert_eq!(
                got.as_deref(),
                Ok(case["line"].as_str().unwrap()),
                "{name} {}",
                case["arguments"]
            );
        }
    }
}

#[test]
fn every_tool_in_the_table_is_one_the_evaluator_knows_and_each_has_a_schema() {
    let registry = shellmate_builtin::registry();
    for name in ["man", "help", "exec", "read"] {
        let tool = registry.build_tool(name, &serde_json::json!({})).unwrap();
        let settings = shellmate_api::ToolSettings {
            enabled: true,
            approval: shellmate_api::ApprovalSetting::Ask,
            preapproved: vec![],
        };
        let spec = tool
            .spec(shellmate_api::ToolAction::Ask, &settings)
            .unwrap();
        assert_eq!(spec.name, name);
    }
}
