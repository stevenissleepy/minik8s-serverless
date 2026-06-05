use axum::body::Bytes;
use axum::http::StatusCode;
use serde_json::Value;

pub(crate) type HttpResult<T> = Result<T, (StatusCode, String)>;

pub(crate) fn decode_json_body(body: Bytes) -> HttpResult<Value> {
    if body.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&body)
        .map_err(|error| bad_request(format!("invalid JSON body: {error}")))
}

pub(crate) fn bad_request(message: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, format!("{}\n", message.into()))
}

pub(crate) fn api_error(message: impl Into<String>) -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("{}\n", message.into()),
    )
}
