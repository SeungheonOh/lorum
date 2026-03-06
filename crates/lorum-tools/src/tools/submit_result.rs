// These definitions are duplicated in lorum-runtime::subagent due to the
// circular dependency (lorum-tools → lorum-runtime). They serve as the
// reference implementation and are validated by tests below.
#![allow(dead_code)]

use lorum_ai_contract::ToolDefinition;
use serde_json::{json, Value};

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "submit_result".to_string(),
        description: "Submit the final result of this task. You MUST call this exactly once \
            before finishing. Use data for success or error for failure."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "result": {
                    "type": "object",
                    "description": "The result payload",
                    "properties": {
                        "data": {
                            "description": "The structured result data (for success)"
                        },
                        "error": {
                            "type": "string",
                            "description": "Error message (for failure)"
                        }
                    }
                }
            },
            "required": ["result"]
        }),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubmitResultPayload {
    pub data: Option<Value>,
    pub error: Option<String>,
}

pub fn parse_submit_result(args: &Value) -> Result<SubmitResultPayload, &'static str> {
    let result = args
        .get("result")
        .ok_or("result must be an object containing either data or error")?;

    if !result.is_object() {
        return Err("result must be an object containing either data or error");
    }

    let has_data = result.get("data").is_some();
    let has_error = result.get("error").is_some();

    if has_data && has_error {
        return Err("result cannot contain both data and error");
    }

    if !has_data && !has_error {
        return Err("result must contain either data or error");
    }

    if has_error {
        let error = result
            .get("error")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default();
        return Ok(SubmitResultPayload {
            data: None,
            error: Some(error),
        });
    }

    let data = result.get("data").cloned();
    if data.as_ref().is_some_and(|v| v.is_null()) {
        return Err("data is required when submit_result indicates success");
    }

    Ok(SubmitResultPayload {
        data,
        error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_success_with_data() {
        let args = json!({ "result": { "data": { "answer": 42 } } });
        let payload = parse_submit_result(&args).unwrap();
        assert_eq!(payload.data, Some(json!({ "answer": 42 })));
        assert_eq!(payload.error, None);
    }

    #[test]
    fn parse_success_with_string_data() {
        let args = json!({ "result": { "data": "just a string" } });
        let payload = parse_submit_result(&args).unwrap();
        assert_eq!(payload.data, Some(json!("just a string")));
        assert_eq!(payload.error, None);
    }

    #[test]
    fn parse_error_result() {
        let args = json!({ "result": { "error": "something went wrong" } });
        let payload = parse_submit_result(&args).unwrap();
        assert_eq!(payload.data, None);
        assert_eq!(payload.error, Some("something went wrong".to_string()));
    }

    #[test]
    fn rejects_both_data_and_error() {
        let args = json!({ "result": { "data": 1, "error": "oops" } });
        let err = parse_submit_result(&args).unwrap_err();
        assert_eq!(err, "result cannot contain both data and error");
    }

    #[test]
    fn rejects_neither_data_nor_error() {
        let args = json!({ "result": {} });
        let err = parse_submit_result(&args).unwrap_err();
        assert_eq!(err, "result must contain either data or error");
    }

    #[test]
    fn rejects_null_data() {
        let args = json!({ "result": { "data": null } });
        let err = parse_submit_result(&args).unwrap_err();
        assert_eq!(err, "data is required when submit_result indicates success");
    }

    #[test]
    fn rejects_missing_result() {
        let args = json!({});
        let err = parse_submit_result(&args).unwrap_err();
        assert_eq!(
            err,
            "result must be an object containing either data or error"
        );
    }

    #[test]
    fn rejects_non_object_result() {
        let args = json!({ "result": "not an object" });
        let err = parse_submit_result(&args).unwrap_err();
        assert_eq!(
            err,
            "result must be an object containing either data or error"
        );
    }

    #[test]
    fn definition_has_correct_name() {
        let def = definition();
        assert_eq!(def.name, "submit_result");
    }
}
