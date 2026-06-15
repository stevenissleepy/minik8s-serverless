use crate::FunctionContext;
use serde_json::{Map, Value};

pub fn handle(event: Value, ctx: &FunctionContext) -> Value {
    let ticket_id = string_field(&event, "ticket_id", "unknown");
    let action = string_field(&event, "action", "unknown");
    let mut result = Map::new();
    result.insert("ticket_id".to_string(), Value::String(ticket_id.clone()));
    result.insert("status".to_string(), Value::String("notified".to_string()));
    result.insert("action".to_string(), Value::String(action.clone()));
    result.insert(
        "category".to_string(),
        event.get("category").cloned().unwrap_or(Value::Null),
    );
    result.insert(
        "risk".to_string(),
        event.get("risk").cloned().unwrap_or(Value::Null),
    );
    result.insert(
        "message".to_string(),
        Value::String(format!("ticket {ticket_id} handled by {action}")),
    );
    result.insert("notified_by".to_string(), Value::String(ctx.name.clone()));
    Value::Object(result)
}

fn string_field(value: &Value, field: &str, default: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_string()
}
