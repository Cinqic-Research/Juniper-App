//! Checks that the two App bodies the harness had to restate (`host_tool_turn`
//! and `execute_host_tool`, which are async or tied to Tauri state) still hold
//! every step, code and message `app.rs` relies on, in the same order. The
//! harness refuses to run, and its tests fail, when they do not.

pub const PROVIDERS_RS: &str = include_str!("../../../src-tauri/src/providers.rs");
pub const TOOLS_RS: &str = include_str!("../../../src-tauri/src/tools.rs");
pub const BEHAVIOR_RS: &str = include_str!("../../../src-tauri/src/behavior.rs");
pub const DEFAULTS_TS: &str = include_str!("../../../src/lib/defaults.ts");

/// Ordered snippets of `host_tool_turn` (whitespace-collapsed).
const HOST_TOOL_TURN: &[&str] = &[
    "if !session.call_ids.insert(call.id.clone()) {",
    "\"DUPLICATE_CALL_ID\", \"This tool call reused an earlier call ID and was not run.\",",
    "if !tools::loop_allowed(round, index as u32 + 1) {",
    "results.push(denied(call, \"TOOL_LOOP_LIMIT\", \"Tool loop limit reached.\"));",
    "match tool_gate(request, &call.name, &session.grants) {",
    "ToolGate::NotEnabled => {",
    "\"TOOL_NOT_ENABLED\", \"This tool is not enabled for this request.\",",
    "ToolGate::Allowed => {}",
    "ToolGate::NeedsPermission(tool) => {",
    "if let Err(error) = tools::validate_call(&call.name, &call.arguments) {",
    "Some(json!({ \"code\": \"INVALID_TOOL_ARGUMENT\", \"message\": error.to_string() })),",
    "let preview = match permission_preview(request, &session.host_context, call) {",
    "let decision = request_permission(",
    "\"allow-once\" => {}",
    "\"allow-chat\" | \"allow-assistant\" if !requires_per_call_approval(tool) => { session.grants.insert(call.name.clone()); }",
    "\"allow-chat\" | \"allow-assistant\" => {}",
    "\"PERMISSION_DENIED\", \"The user denied this host capability.\",",
    "let result = execute_host_tool( request, &mut session.host_context, call, round, index as u32 + 1, );",
];

/// Ordered snippets of `request_permission` (the timeout path).
const REQUEST_PERMISSION: &[&str] = &[
    "response = timeout(Duration::from_secs(300), receiver) => match response {",
    "Err(_) => Err(ProviderError::new(\"PERMISSION_TIMEOUT\", \"The permission request timed out.\")),",
];

/// Ordered snippets of `execute_host_tool`.
const EXECUTE_HOST_TOOL: &[&str] = &[
    "if matches!( call.name.as_str(), \"calculator.evaluate\" | \"datetime.current\" | \"unit.convert\" ) { return tools::execute_call( &call.id, &call.name, &call.arguments, round, calls_this_round, ); }",
    "if let Err(error) = tools::validate_call(&call.name, &call.arguments) {",
    "\"code\": \"INVALID_TOOL_ARGUMENT\", \"message\": error.to_string()",
    "\"file.read\" => { let id = call.arguments[\"attachmentId\"].as_str().unwrap_or_default(); request .attachments .iter() .find(|attachment| attachment.id == id) .map(|attachment| { json!({ \"attachmentId\": attachment.id, \"name\": attachment.name, \"content\": attachment.content }) }) .ok_or(( \"ATTACHMENT_NOT_GRANTED\", \"That attachment was not granted to this request.\", )) }",
    "\"file.metadata\" => { let id = call.arguments[\"attachmentId\"].as_str().unwrap_or_default(); request .attachments .iter() .find(|attachment| attachment.id == id) .map(|attachment| { json!({ \"attachmentId\": attachment.id, \"name\": attachment.name, \"sizeBytes\": attachment.size_bytes.unwrap_or(attachment.content.len() as u64), \"contentType\": attachment.content_type.as_deref().unwrap_or(\"text/plain\") }) }) .ok_or(( \"ATTACHMENT_NOT_GRANTED\", \"That attachment was not granted to this request.\", )) }",
    "_ => Err((\"UNKNOWN_TOOL\", \"This host tool is not available.\")),",
    "Ok(payload) if payload.len() <= tools::MAX_PAYLOAD_BYTES => { tools::host_result(&call.id, &call.name, \"success\", Some(value), None) }",
    "\"code\": \"TOOL_RESULT_TOO_LARGE\", \"message\": \"The host tool result exceeded the runtime limit.\"",
    "Err((code, message)) => tools::host_result( &call.id, &call.name, \"error\", None, Some(json!({ \"code\": code, \"message\": message })), ),",
];

/// The tool message the App sends back to the model, in both provider loops.
const TOOL_MESSAGE: &str = "\"content\": behavior::neutralize_control_tokens(&result.to_string())";

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The body of the item whose header line starts with `header`, collapsed.
fn body(source: &str, header: &str) -> Result<String, String> {
    let start = source
        .find(header)
        .ok_or_else(|| format!("`{header}` not found"))?;
    // Bodies end at the next item at column 0.
    let rest = &source[start..];
    let end = rest[1..]
        .find("\nfn ")
        .into_iter()
        .chain(rest[1..].find("\nasync fn "))
        .chain(rest[1..].find("\npub fn "))
        .chain(rest[1..].find("\n// "))
        .min()
        .map_or(rest.len(), |index| index + 1);
    Ok(collapse(&rest[..end]))
}

fn in_order(name: &str, text: &str, snippets: &[&str]) -> Result<(), String> {
    let mut position = 0;
    for snippet in snippets {
        let snippet = collapse(snippet);
        match text[position..].find(&snippet) {
            Some(offset) => position += offset + snippet.len(),
            None => return Err(format!("{name}: missing or reordered: {snippet}")),
        }
    }
    Ok(())
}

/// Names of the checks that passed, or the first failure.
pub fn check() -> Result<Vec<&'static str>, String> {
    in_order(
        "host_tool_turn",
        &body(PROVIDERS_RS, "async fn host_tool_turn<R: Runtime>(")?,
        HOST_TOOL_TURN,
    )?;
    in_order(
        "request_permission",
        &body(PROVIDERS_RS, "async fn request_permission<R: Runtime>(")?,
        REQUEST_PERMISSION,
    )?;
    in_order(
        "execute_host_tool",
        &body(PROVIDERS_RS, "fn execute_host_tool(")?,
        EXECUTE_HOST_TOOL,
    )?;
    let providers = collapse(PROVIDERS_RS);
    if providers.matches(&collapse(TOOL_MESSAGE)).count() < 2 {
        return Err("tool messages are no longer the neutralized HostToolResult string".into());
    }
    Ok(vec![
        "host_tool_turn order and denial messages",
        "request_permission timeout",
        "execute_host_tool stateless and attachment arms",
        "tool message = neutralize_control_tokens(result.to_string())",
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bound_app_sources_still_match_the_harness() {
        assert_eq!(check().unwrap().len(), 4);
    }

    #[test]
    fn a_reordered_or_changed_step_is_detected() {
        let text = collapse("b(); a(); c();");
        assert!(in_order("x", &text, &["a();", "c();"]).is_ok());
        assert!(in_order("x", &text, &["c();", "a();"]).is_err());
        assert!(in_order("x", &text, &["d();"]).is_err());
    }
}
