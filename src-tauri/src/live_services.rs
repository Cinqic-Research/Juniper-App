//! Live checks against real services (ignored; needs the network and npx).
//! Web: a Wikipedia search and page read through Juniper's own clients, and
//! refusals for metadata and private addresses, including through a public
//! redirect. MCP: the reference `server-everything` over stdio, connected,
//! listed, and called through Juniper's client.
//!
//!   JUNIPER_LIVE_EVIDENCE=/path/services.jsonl \
//!   cargo test --lib live_services -- --ignored --nocapture

use crate::agent::{mcp, store, web};
use crate::commands::Cancellation;
use serde_json::{Value, json};
use std::io::Write;

fn record(value: &Value) {
    println!("{value}");
    if let Ok(path) = std::env::var("JUNIPER_LIVE_EVIDENCE") {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("evidence");
        writeln!(file, "{value}").expect("write");
    }
}

#[test]
#[ignore = "uses the network; run deliberately"]
fn live_services() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let at = chrono::Utc::now().to_rfc3339();

    let provider = web::Provider::Wikipedia {
        language: "en".into(),
    };
    let results = runtime.block_on(web::search(&provider, "Juniperus communis"));
    let search = match &results {
        Ok(items) => json!({
            "ok": !items.is_empty() && items.iter().all(|item| item.url.starts_with("https://en.wikipedia.org/wiki/")),
            "count": items.len(),
            "first": items.first().map(|item| json!({ "title": item.title, "url": item.url, "snippet": item.snippet.chars().take(160).collect::<String>() })),
        }),
        Err((code, message)) => json!({ "ok": false, "code": code, "message": message }),
    };
    let page = results
        .ok()
        .and_then(|items| items.into_iter().next())
        .map(|item| {
            runtime.block_on(web::fetch(
                web::checked_url(&item.url).unwrap(),
                &Cancellation::default(),
            ))
        });
    let fetch = match page {
        Some(Ok(page)) => json!({
            "ok": !page.text.is_empty() && page.final_url.host_str() == Some("en.wikipedia.org"),
            "title": page.title,
            "finalUrl": page.final_url.to_string(),
            "characters": page.text.chars().count(),
            "truncated": page.truncated,
            "scriptFree": !page.text.contains("<script"),
        }),
        Some(Err((code, message))) => json!({ "ok": false, "code": code, "message": message }),
        None => json!({ "ok": false, "code": "NO_RESULT" }),
    };
    let refusals: Vec<Value> = [
        "http://169.254.169.254/latest/meta-data/",
        "http://localhost:8080/",
        "http://10.0.0.1/",
        // A public redirector pointing at loopback: refused at the redirect.
        "https://httpbin.org/redirect-to?url=http%3A%2F%2F127.0.0.1%3A9%2F",
    ]
    .iter()
    .map(|url| {
        let outcome = match web::checked_url(url) {
            Err((code, _)) => Err(code),
            Ok(parsed) => runtime
                .block_on(web::fetch(parsed, &Cancellation::default()))
                .map(|page| page.final_url.to_string())
                .map_err(|(code, _)| code),
        };
        json!({ "url": url, "refused": outcome.is_err(), "code": outcome.err() })
    })
    .collect();
    record(
        &json!({ "check": "web", "at": at, "provider": "wikipedia-en", "search": search, "fetch": fetch, "refusals": refusals }),
    );

    let database =
        std::env::temp_dir().join(format!("juniper-live-mcp-{}.db", uuid::Uuid::new_v4()));
    crate::storage::initialize(&database).unwrap();
    let connection = store::open(&database).unwrap();
    let plugin = mcp::register(
        &connection,
        mcp::PluginConfig {
            name: "server-everything".into(),
            kind: "mcp-stdio".into(),
            command: Some("/usr/bin/npx".into()),
            args: vec![
                "-y".into(),
                "@modelcontextprotocol/server-everything".into(),
            ],
            env: vec![],
            url: None,
            credential_ref: None,
        },
    )
    .unwrap();
    let tested = runtime.block_on(mcp::test_connection(database.clone(), plugin.id.clone()));
    let mcp_record = match tested {
        Ok(plugin) => {
            let echo = plugin
                .state
                .tools
                .iter()
                .find(|tool| tool.name == "echo")
                .cloned();
            let call = echo.map(|_| {
                runtime.block_on(mcp::call_for_tests(
                    &plugin,
                    "echo",
                    json!({ "message": "hello from Juniper" }),
                ))
            });
            json!({
                "status": plugin.state.status,
                "message": plugin.state.message,
                "era": plugin.state.era,
                "protocolVersion": plugin.state.protocol_version,
                "serverInfo": plugin.state.server_info,
                "tools": plugin.state.tools.iter().map(|tool| tool.name.clone()).collect::<Vec<_>>(),
                "rejected": plugin.state.rejected_tools,
                "echo": call.map(|result| match result {
                    Ok(value) => json!({ "ok": value.to_string().contains("hello from Juniper"), "content": value["content"] }),
                    Err((code, message)) => json!({ "ok": false, "code": code, "message": message }),
                }),
            })
        }
        Err(error) => json!({ "status": "error", "message": error }),
    };
    mcp::disconnect(&plugin.id);
    record(
        &json!({ "check": "mcp-server-everything", "at": chrono::Utc::now().to_rfc3339(), "result": mcp_record }),
    );
    std::fs::remove_file(database).ok();
}
