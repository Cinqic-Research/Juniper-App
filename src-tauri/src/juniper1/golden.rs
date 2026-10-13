//! Cross-language golden tests: the Rust port against fixtures the Python
//! reference wrote (`tests/juniper1/generate_golden.py`).
//!
//! They need the canonical tokenizer, which is not part of this repository:
//! `JUNIPER1_TOKENIZER=<tokenizer.json>`; the 200-document regression set
//! additionally needs the private texts file the generator wrote:
//! `JUNIPER1_GOLDEN_DOCS=<regression_docs.private.jsonl>`. The fixture itself
//! (token IDs of the private tokenizer) is private as well and is checked against
//! its pinned digest: `JUNIPER1_GOLDEN=<golden.v1.json>`. Run with
//! `cargo test --lib juniper1::golden -- --ignored`.

use super::profile;
use super::protocol::{self, Channel, Message, Role};
use super::tokenizer::{self as tk, J1Tokenizer, sha256_hex};
use serde_json::Value;

/// SHA-256 of the private fixture `generate_golden.py` wrote.
const GOLDEN_SHA256: &str = "82198489a098a717aa1834ebd5a6c5860cacd39ad0ebc0e3c198a18b79b00745";

fn golden() -> Value {
    let path = std::env::var("JUNIPER1_GOLDEN").expect("set JUNIPER1_GOLDEN to the private golden.v1.json");
    let bytes = std::fs::read(path).expect("golden fixture");
    assert_eq!(sha256_hex(&bytes), GOLDEN_SHA256, "golden fixture is not the pinned one");
    serde_json::from_slice(&bytes).expect("golden fixture")
}

fn canonical_bytes() -> Vec<u8> {
    let path = std::env::var("JUNIPER1_TOKENIZER")
        .expect("set JUNIPER1_TOKENIZER to the canonical j1tok tokenizer.json");
    std::fs::read(path).expect("tokenizer file")
}

fn canonical() -> J1Tokenizer {
    let profile = profile::profile().expect("profile");
    J1Tokenizer::load(
        &canonical_bytes(),
        &profile.tokenizer.sha256,
        profile.tokenizer.vocab_size,
    )
    .expect("canonical tokenizer loads")
}

fn ids(value: &Value) -> Vec<u32> {
    value
        .as_array()
        .expect("id array")
        .iter()
        .map(|id| id.as_u64().expect("id") as u32)
        .collect()
}

fn message(value: &Value) -> Message {
    Message::new(
        Role::parse(value["role"].as_str().expect("role")).expect("known role"),
        value["content"].as_str().expect("content"),
        value["channel"]
            .as_str()
            .map(|channel| Channel::parse(channel).expect("known channel")),
        value["turn"].as_u64().expect("turn") as u32,
    )
    .expect("valid message")
}

fn messages(value: &Value) -> Vec<Message> {
    value
        .as_array()
        .expect("messages")
        .iter()
        .map(message)
        .collect()
}

#[test]
#[ignore = "needs the canonical j1tok tokenizer: JUNIPER1_TOKENIZER=<tokenizer.json>"]
fn golden_tokenizer_identity_controls_and_texts() {
    let golden = golden();
    let tok = canonical();
    assert_eq!(tok.sha256(), golden["tokenizer_sha256"]);
    assert_eq!(golden["tokenizers_version"], "0.23.2");
    assert_eq!(
        u64::from(tok.vocab_size()),
        golden["vocab_size"].as_u64().unwrap()
    );

    // The setting the loader applies is load-bearing on the canonical file too.
    let plain = tokenizers::Tokenizer::from_bytes(canonical_bytes()).expect("parse");
    assert_eq!(
        plain.encode("<|system|>", false).expect("encode").get_ids(),
        &[tk::SYSTEM]
    );

    let spellings = golden["control_spellings"].as_array().expect("spellings");
    assert_eq!(spellings.len(), 64);
    for (id, item) in spellings.iter().enumerate() {
        let spelling = item["spelling"].as_str().expect("spelling");
        assert_eq!(spelling, tk::control_spelling(id as u32));
        let alone = tok.encode(spelling).expect("encode");
        assert_eq!(alone, ids(&item["alone"]), "{spelling}");
        assert!(alone.iter().all(|id| *id >= tk::N_CONTROL), "{spelling}");
        let embedded = format!("x {spelling}y\n{spelling}{spelling}");
        assert_eq!(
            tok.encode(&embedded).expect("encode"),
            ids(&item["embedded"]),
            "{spelling}"
        );
    }
    let texts = golden["texts"].as_array().expect("texts");
    assert!(texts.len() >= 40);
    for item in texts {
        let text = item["text"].as_str().expect("text");
        let encoded = tok.encode(text).expect("encode");
        assert_eq!(encoded, ids(&item["ids"]), "{}", item["label"]);
        assert_eq!(
            tok.decode(&encoded).expect("decode") == text,
            item["roundtrip"].as_bool().unwrap(),
            "{}",
            item["label"]
        );
    }
}

#[test]
#[ignore = "needs the canonical j1tok tokenizer: JUNIPER1_TOKENIZER=<tokenizer.json>"]
fn golden_serialization() {
    let golden = golden();
    let tok = canonical();
    for item in golden["serialization"].as_array().expect("cases") {
        let ms = messages(&item["messages"]);
        let add = item["add_generation_prompt"].as_bool().unwrap();
        assert_eq!(
            protocol::serialize(&tok, &ms, add).expect("serialize"),
            ids(&item["ids"]),
            "{} add_generation_prompt={add}",
            item["label"]
        );
    }
}

#[test]
#[ignore = "needs the canonical j1tok tokenizer: JUNIPER1_TOKENIZER=<tokenizer.json>"]
fn golden_budgeting_and_trimming() {
    let golden = golden();
    let tok = canonical();
    for item in golden["budget"].as_array().expect("cases") {
        let fitted = protocol::fit_to_budget(
            &tok,
            &messages(&item["messages"]),
            item["budget"].as_u64().unwrap() as usize,
            item["reserve_generation"].as_u64().unwrap() as usize,
            item["max_tool_tokens"].as_u64().unwrap() as usize,
        );
        match (&item["result"], fitted) {
            (Value::Null, Err(error)) => {
                assert_eq!(
                    error.0,
                    item["error"].as_str().unwrap(),
                    "{}",
                    item["label"]
                );
            }
            (expected, Ok(fitted)) if !expected.is_null() => {
                assert_eq!(fitted, messages(expected), "{}", item["label"]);
            }
            (expected, got) => panic!("{}: expected {expected}, got {got:?}", item["label"]),
        }
    }
}

#[test]
#[ignore = "needs the canonical j1tok tokenizer: JUNIPER1_TOKENIZER=<tokenizer.json>"]
fn golden_parser_outcomes() {
    let golden = golden();
    let tok = canonical();
    for item in golden["parse"].as_array().expect("cases") {
        let parsed = protocol::parse_generation(
            &tok,
            &ids(&item["ids"]),
            item["turn"].as_u64().unwrap() as u32,
        );
        let label = &item["label"];
        assert_eq!(parsed.stop.as_str(), item["stop"], "{label}");
        assert_eq!(parsed.messages, messages(&item["messages"]), "{label}");
        let expected = item["error"].as_str();
        match expected {
            // The JSON decoder's own wording differs between languages.
            Some(reason) if reason.starts_with("tool call is not JSON:") => assert!(
                parsed
                    .error
                    .as_deref()
                    .is_some_and(|error| error.starts_with("tool call is not JSON:")),
                "{label}"
            ),
            _ => assert_eq!(parsed.error.as_deref(), expected, "{label}"),
        }
    }
}

#[test]
#[ignore = "needs JUNIPER1_TOKENIZER and the private texts file JUNIPER1_GOLDEN_DOCS"]
fn golden_regression_documents() {
    let golden = golden();
    let tok = canonical();
    let path = std::env::var("JUNIPER1_GOLDEN_DOCS")
        .expect("set JUNIPER1_GOLDEN_DOCS to the generator's --private-out file");
    let bytes = std::fs::read(path).expect("texts file");
    let set = &golden["regression_docs"];
    assert_eq!(sha256_hex(&bytes), set["texts_file_sha256"]);
    let texts: Vec<Value> = String::from_utf8(bytes)
        .expect("utf8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("line"))
        .collect();
    let docs = set["docs"].as_array().expect("docs");
    assert_eq!(docs.len(), 200);
    assert_eq!(texts.len(), 200);
    let mut identical = 0;
    for (doc, text) in docs.iter().zip(&texts) {
        assert_eq!(doc["index"], text["index"]);
        let text = text["text"].as_str().expect("text");
        assert_eq!(sha256_hex(text.as_bytes()), doc["text_sha256"]);
        let encoded = tok.encode(text).expect("encode");
        let joined = encoded
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        if encoded.len() as u64 == doc["n_ids"].as_u64().unwrap()
            && sha256_hex(joined.as_bytes()) == doc["ids_sha256"]
            && (tok.decode(&encoded).expect("decode") == text)
                == doc["roundtrip"].as_bool().unwrap()
        {
            identical += 1;
        }
    }
    println!("GOLDEN regression documents identical to the Python reference: {identical}/200");
    assert_eq!(identical, 200);
}
