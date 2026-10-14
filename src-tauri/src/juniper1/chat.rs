//! A Juniper 1 chat request over the token-ID client, inside Juniper's host loop.
//!
//! The host composes the conversation as `juniper-chat-v1` messages: its own
//! instruction layers as `<|system|>`, the user's assistant profile as
//! `<|developer|>`, memories and attachments as framed `<|user|>` data, then the
//! dialogue. History carries only accepted user-facing answers; analysis is
//! never stored, emitted, or re-sent after its turn. A `<|tool_call|>` is a
//! request that goes through the same tool gate, permission prompt, limits,
//! and host execution as every other backend, and its result returns as a
//! host-authored `<|tool|>` message. Nothing reaches the interface unless the
//! parser accepted the turn.

use super::client::{self, CompletionSource, Sampling, TokenIdClient};
use super::profile::{TokenIdArtifact, TokenIdProfile};
use super::protocol::{self, Channel, Message, Role, Stop};
use super::tokenizer::{self as tk, J1Tokenizer};
use crate::behavior;
use crate::commands::{AppState, Cancellation, record_runtime_log};
use crate::domain::{
    ChatRequest, ChatStreamEvent, HostToolContext, NormalizedToolCall, RuntimeIdentity,
};
use crate::providers::{self, Finished, ProviderError};
use crate::tools;
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Emitter, Runtime};

/// Everything a request needs for a verified, running Juniper 1 server.
pub struct TokenIdRuntime {
    pub profile: TokenIdProfile,
    pub artifact: TokenIdArtifact,
    pub tokenizer: Arc<J1Tokenizer>,
    pub client: TokenIdClient,
}

#[derive(Default)]
struct Counters {
    input: u64,
    output: u64,
}

fn from_runtime(error: String) -> ProviderError {
    let (code, message) = error.split_once(':').map_or(
        ("LOCAL_RUNTIME_FAILED", error.as_str()),
        |(code, message)| (code, message.trim()),
    );
    ProviderError::new(code, message)
}

fn invalid_request(message: &str) -> ProviderError {
    ProviderError::new("INVALID_REQUEST", message)
}

fn protocol_failure(_: protocol::ProtocolError) -> ProviderError {
    invalid_request("The conversation could not be expressed in Juniper 1's protocol.")
}

/// The conversation as `juniper-chat-v1` messages and the current turn index.
fn conversation(
    request: &ChatRequest,
    runtime: &TokenIdRuntime,
    host_context: &HostToolContext,
) -> Result<(Vec<Message>, u32), ProviderError> {
    let offered = providers::offered_tools(request);
    let profile = request
        .messages
        .first()
        .filter(|message| message.role == "system")
        .map(|message| message.content.as_str())
        .unwrap_or_default();
    let lineage = behavior::Lineage {
        description: runtime.artifact.lineage(&runtime.profile),
        trained_by_cinqic: true,
    };
    let layers = behavior::instruction_layers(request, &lineage, &offered, profile)
        .map_err(|error| ProviderError::new("CONSTITUTION_INVALID", error))?;
    let mut messages =
        vec![Message::new(Role::System, layers.host, None, 0).map_err(protocol_failure)?];
    if let Some(profile) = layers.profile {
        messages.push(Message::new(Role::Developer, profile, None, 0).map_err(protocol_failure)?);
    }
    let mut turn = 0;
    let mut dialogue = Vec::new();
    for message in request
        .messages
        .iter()
        .filter(|message| message.role != "system")
    {
        dialogue.push(
            match message.role.as_str() {
                "user" => {
                    turn += 1;
                    Message::new(Role::User, message.content.as_str(), None, turn)
                }
                "assistant" => Message::new(
                    Role::Assistant,
                    message.content.as_str(),
                    Some(Channel::Final),
                    turn,
                ),
                _ => {
                    return Err(invalid_request(
                        "The chat request contains a message role the host does not accept.",
                    ));
                }
            }
            .map_err(protocol_failure)?,
        );
    }
    if dialogue.last().map(|message| message.role) != Some(Role::User) {
        return Err(invalid_request(
            "A Juniper 1 request must end with the user's message.",
        ));
    }
    let memories = providers::context_memories(request, host_context);
    if let Some(text) = behavior::memory_text_verbatim(&request.assistant_name, &memories) {
        messages.push(Message::new(Role::User, text, None, turn).map_err(protocol_failure)?);
    }
    if let Some(text) = behavior::attachments_text_verbatim(&request.attachments) {
        let last_user = dialogue.len() - 1;
        dialogue.insert(
            last_user,
            Message::new(Role::User, text, None, turn).map_err(protocol_failure)?,
        );
    }
    messages.extend(dialogue);
    Ok((messages, turn))
}

/// An accepted answer is non-empty plain text. The parser already refused
/// control IDs; literal control spellings and the host result protocol are
/// refused too, so neither protocol markup nor a forged host result is shown
/// even as text.
fn accept_answer(text: &str) -> Result<(), ProviderError> {
    if text.contains(tools::PROTOCOL_VERSION)
        || tk::control_spellings()
            .iter()
            .any(|spelling| text.contains(spelling.as_str()))
    {
        return Err(ProviderError::new(
            "MODEL_OUTPUT_INVALID",
            "The model's answer contained raw protocol text and was not accepted.",
        ));
    }
    if text.trim().is_empty() {
        return Err(ProviderError::new(
            "EMPTY_ANSWER",
            "The model finished without writing an answer.",
        ));
    }
    Ok(())
}

fn tool_call(message: &Message) -> Result<NormalizedToolCall, ProviderError> {
    let malformed = || {
        ProviderError::new(
            "MALFORMED_TOOL_CALL",
            "The model returned an invalid or oversized tool call.",
        )
    };
    let object = protocol::validate_call(&message.content).map_err(|_| malformed())?;
    let name = object["name"].as_str().unwrap_or_default().to_owned();
    let arguments = object["arguments"].clone();
    if name.len() > 256
        || serde_json::to_vec(&arguments)
            .map_or(true, |bytes| bytes.len() > tools::MAX_PAYLOAD_BYTES)
    {
        return Err(malformed());
    }
    Ok(NormalizedToolCall {
        // The host assigns call IDs; the protocol gives the model none to forge.
        id: format!("call-{}", uuid::Uuid::new_v4().simple()),
        name,
        arguments,
    })
}

// One auditable loop: the request, the event sink, cancellation, host state,
// the verified runtime, the completion source, and the token counters.
#[allow(clippy::too_many_arguments)]
async fn run<R: Runtime, S: CompletionSource>(
    request: &ChatRequest,
    app: &AppHandle<R>,
    cancellation: &Cancellation,
    state: &AppState,
    runtime: &TokenIdRuntime,
    source: &S,
    counters: &mut Counters,
) -> Result<(), ProviderError> {
    providers::check_attachment_limits(request)?;
    let tok = runtime.tokenizer.as_ref();
    let topic = format!("juniper://chat/{}", request.request_id);
    let mut session = providers::ToolSession::new(providers::scoped_host_context(request));
    let (conversation, turn) = conversation(request, runtime, &session.host_context)?;
    let generation = &runtime.profile.generation;
    let context = runtime.profile.server.ctx_size;
    let budget = request
        .generation
        .max_output
        .unwrap_or(generation.default_max_tokens)
        .clamp(1, generation.max_tokens_ceiling);
    let sampling = Sampling {
        temperature: generation.temperature,
        top_k: generation.top_k,
        top_p: generation.top_p,
        seed: uuid::Uuid::new_v4().as_u128() as u32,
    };
    // Assistant and tool messages of the current turn, analysis included.
    let mut transcript: Vec<Message> = Vec::new();
    let mut reasoning_announced = false;
    for round in 0..tools::MAX_TOOL_ROUNDS {
        let mut messages = conversation.clone();
        messages.extend(transcript.iter().cloned());
        let fitted = protocol::fit_to_budget(
            tok,
            &messages,
            context as usize,
            budget as usize,
            generation.max_tool_result_tokens as usize,
        )
        .map_err(|_| {
            ProviderError::new(
                "CONTEXT_OVERFLOW",
                format!(
                    "This message, Juniper's instructions, and {budget} tokens for the answer do not fit in the model's {context}-token context. Shorten the message or remove attachments."
                ),
            )
        })?;
        let prompt = protocol::serialize(tok, &fitted, true).map_err(protocol_failure)?;
        let generated = client::generate_turn(
            source,
            tok,
            &prompt,
            budget,
            context,
            &sampling,
            generation.max_segments,
            turn,
            cancellation,
        )
        .await
        .map_err(from_runtime)?;
        counters.input += prompt.len() as u64;
        counters.output += generated.generated.len() as u64;
        let parsed = generated.parsed;
        if !reasoning_announced
            && parsed
                .messages
                .iter()
                .any(|message| message.channel == Some(Channel::Analysis))
        {
            reasoning_announced = true;
            providers::emit_reasoning_activity(request, app, &topic);
        }
        match parsed.stop {
            Stop::Eot => {
                let answer = parsed
                    .messages
                    .last()
                    .map(|message| message.content.as_str())
                    .unwrap_or_default();
                accept_answer(answer)?;
                let mut event = ChatStreamEvent::for_request(&request.request_id);
                event.delta = Some(answer.to_owned());
                let _ = app.emit(&topic, event);
                return Ok(());
            }
            Stop::Call => {
                let call = parsed
                    .messages
                    .last()
                    .ok_or_else(|| {
                        ProviderError::new("MALFORMED_TOOL_CALL", "The tool call was empty.")
                    })
                    .and_then(tool_call)?;
                let calls = std::slice::from_ref(&call);
                let (_, results) = providers::host_tool_turn(
                    request,
                    calls,
                    round,
                    cancellation,
                    app,
                    &topic,
                    state,
                    &mut session,
                )
                .await?;
                providers::emit_tool_turn(request, app, &topic, calls, &results);
                transcript.extend(parsed.messages);
                for result in results {
                    let content = serde_json::to_string(&result).map_err(|_| {
                        ProviderError::new(
                            "TOOL_RESULT_INVALID",
                            "A host result could not be encoded.",
                        )
                    })?;
                    transcript.push(
                        Message::new(Role::Tool, content, None, turn).map_err(protocol_failure)?,
                    );
                }
            }
            Stop::Malformed => {
                let code = if generated.exhausted {
                    "GENERATION_TRUNCATED"
                } else {
                    "MODEL_OUTPUT_INVALID"
                };
                record_runtime_log(
                    state,
                    "juniper1.output_rejected",
                    Some(code),
                    Some(&request.provider.kind),
                    Some(&request.model.model_id),
                );
                return Err(if generated.exhausted {
                    ProviderError::new(
                        "GENERATION_TRUNCATED",
                        "The answer reached the output limit before the model finished it and was not shown.",
                    )
                } else {
                    ProviderError::new(
                        "MODEL_OUTPUT_INVALID",
                        "The model's reply did not follow Juniper 1's conversation protocol and was not shown.",
                    )
                });
            }
        }
    }
    Err(ProviderError::new(
        "TOOL_LOOP_LIMIT",
        "The model exceeded Juniper's bounded tool loop.",
    ))
}

/// Scripted IDs to force through the real server for a request, by request
/// ID. Test builds only; see `client::forcing`.
#[cfg(test)]
pub(crate) static FORCED_SCRIPTS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, Vec<u32>>>,
> = std::sync::LazyLock::new(Default::default);

/// Runs one Juniper 1 request and always ends it with a single `done` event.
pub async fn stream_chat<R: Runtime>(
    app: &AppHandle<R>,
    request: &ChatRequest,
    cancellation: &Cancellation,
    state: &AppState,
    runtime: &TokenIdRuntime,
    identity: RuntimeIdentity,
) {
    let started = Instant::now();
    record_runtime_log(
        state,
        "request.started",
        None,
        Some(&request.provider.kind),
        Some(&request.model.model_id),
    );
    let mut counters = Counters::default();
    #[cfg(test)]
    let script = FORCED_SCRIPTS
        .lock()
        .ok()
        .and_then(|mut scripts| scripts.remove(&request.request_id));
    #[cfg(test)]
    let result = match script {
        Some(script) => {
            let forced = client::forcing::Forced::new(&runtime.client, script);
            run(
                request,
                app,
                cancellation,
                state,
                runtime,
                &forced,
                &mut counters,
            )
            .await
        }
        None => {
            run(
                request,
                app,
                cancellation,
                state,
                runtime,
                &runtime.client,
                &mut counters,
            )
            .await
        }
    };
    #[cfg(not(test))]
    let result = run(
        request,
        app,
        cancellation,
        state,
        runtime,
        &runtime.client,
        &mut counters,
    )
    .await;
    providers::finish_request(
        request,
        app,
        state,
        result,
        Finished {
            cancelled: cancellation.is_cancelled(),
            started,
            backend: &runtime.profile.id,
            reasoning_effort: None,
            runtime: Some(identity),
            tokens: Some((counters.input, counters.output)),
        },
    );
}
