use crate::FunctionContext;
use serde_json::{Map, Value};

pub fn handle(event: Value, ctx: &FunctionContext) -> Value {
    let text = event
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let category = if contains_any(&text, &["refund", "money", "charge"]) {
        "refund"
    } else if contains_any(&text, &["password", "login", "error", "bug"]) {
        "technical"
    } else if contains_any(&text, &["angry", "complaint", "terrible"]) {
        "complaint"
    } else {
        "general"
    };

    let mut result = object_or_empty(event);
    result.insert("category".to_string(), Value::String(category.to_string()));
    result.insert("classified_by".to_string(), Value::String(ctx.name.clone()));
    Value::Object(result)
}

fn contains_any(text: &str, words: &[&str]) -> bool {
    words.iter().any(|word| text.contains(word))
}

fn object_or_empty(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(object) => object,
        _ => Map::new(),
    }
}
