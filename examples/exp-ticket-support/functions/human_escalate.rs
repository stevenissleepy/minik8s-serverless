use crate::FunctionContext;
use serde_json::{Map, Value};

pub fn handle(event: Value, ctx: &FunctionContext) -> Value {
    let risk = int_field(&event, "risk");
    let user_level = string_field(&event, "user_level", "");
    let mut result = object_or_empty(event);
    result.insert(
        "action".to_string(),
        Value::String("human-escalate".to_string()),
    );
    result.insert(
        "priority".to_string(),
        Value::String(if risk >= 90 { "p1" } else { "p2" }.to_string()),
    );
    result.insert(
        "queue".to_string(),
        Value::String(
            if user_level == "vip" {
                "vip-support"
            } else {
                "support"
            }
            .to_string(),
        ),
    );
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

fn int_field(value: &Value, field: &str) -> i64 {
    match value.get(field) {
        Some(Value::Number(number)) => number.as_i64().unwrap_or(0),
        Some(Value::String(text)) => text.parse::<i64>().unwrap_or(0),
        _ => 0,
    }
}

fn object_or_empty(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(object) => object,
        _ => Map::new(),
    }
}
