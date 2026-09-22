use serde_json::Value;
pub fn explicit_provider(v: &Value) -> Option<(String, String, Option<String>)> {
    let ty = v
        .pointer("/payload/type")
        .or_else(|| v.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let explicit = matches!(
        ty,
        "model/rerouted" | "server_model" | "response.server_model" | "server_model_reported"
    );
    if !explicit {
        return None;
    }
    let p = v.get("payload").unwrap_or(v);
    let model = [
        "server_model",
        "provider_model",
        "served_model",
        "model",
        "to_model",
    ]
    .iter()
    .find_map(|k| p.get(k).and_then(Value::as_str))?;
    let reason = p
        .get("reason")
        .or_else(|| p.get("routing_reason"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    Some((model.to_owned(), ty.to_owned(), reason))
}
