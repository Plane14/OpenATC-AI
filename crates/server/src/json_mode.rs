//! Grammar-constrained JSON for request classification and crew actions.
pub const GRAMMAR: &str = r#"
root ::= ws object ws
value ::= object | array | string | number | "true" | "false" | "null"
object ::= "{" ws (string ws ":" ws value ws ("," ws string ws ":" ws value ws)*)? "}"
array ::= "[" ws (value ws ("," ws value ws)*)? "]"
string ::= "\"" ([^"\\\x00-\x1F] | "\\" (["\\/bfnrt] | "u" [0-9a-fA-F] [0-9a-fA-F] [0-9a-fA-F] [0-9a-fA-F]))* "\""
number ::= "-"? ("0" | [1-9] [0-9]*) ("." [0-9]+)? ([eE] [+-]? [0-9]+)?
ws ::= [ \t\n\r]*
"#;

pub fn enabled(format: Option<&serde_json::Value>) -> Result<bool, String> {
    match format.and_then(|f| f.get("type")).and_then(|v| v.as_str()) {
        None if format.is_none() => Ok(false),
        Some("text") => Ok(false),
        Some("json_object") => Ok(true),
        _ => Err("response_format supports text or json_object".into()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn unsupported_formats_are_not_silently_ignored() {
        assert!(!super::enabled(None).unwrap());
        assert!(super::enabled(Some(&serde_json::json!({"type":"json_object"}))).unwrap());
        assert!(super::enabled(Some(&serde_json::json!({"type":"json_schema"}))).is_err());
    }
}
