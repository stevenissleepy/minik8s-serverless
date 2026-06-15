use crate::FunctionContext;
use serde_json::{Map, Value};

pub fn handle(event: Value, ctx: &FunctionContext) -> Value {
    let category = string_field(&event, "category", "general");
    let reply = match category.as_str() {
        "refund" => "Your refund request has been received.",
        "technical" => "We sent password and troubleshooting steps.",
        "complaint" => "We recorded your complaint and will follow up.",
        _ => "We received your ticket.",
    };
    let mut result = object_or_empty(event);
    result.insert(
        "action".to_string(),
        Value::String("auto-reply".to_string()),
    );
    result.insert("reply".to_string(), Value::String(reply.to_string()));
    result.insert("handled_by".to_string(), Value::String(ctx.name.clone()));
    Value::Object(result)
}

fn string_field(value: &Value, field: &str, default: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_string()
}

fn object_or_empty(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(object) => object,
        _ => Map::new(),
    }
}
