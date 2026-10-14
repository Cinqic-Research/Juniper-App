//! Token-ID client for Juniper 1 on llama.cpp's `llama-server`, ported from
//! the reference `j1data.runtime_client.TokenIdClient`.
//!
//! The runtime never sees text from this client. It receives an all-integer
//! prompt on `POST /completion` (used verbatim, no BOS) and returns sampled IDs
//! in the non-streaming result (`return_tokens`; streaming drops the IDs of
//! incomplete UTF-8 byte tokens). Host-only roles, the document separator, and
//! reserved IDs are banned with integer-keyed `logit_bias`. llama-server ends
//! generation at every end-of-generation ID (9, 10, 11 here), so an analysis
//! message closed by `<|end|>` is resubmitted with the host-appended header of
//! the next message.
//!
//! Only `/completion` exists here; the text endpoints (`/tokenize`,
//! `/detokenize`, `/apply-template`, chat and text completions) have no
//! caller. Requests go only to a literal loopback address, never through an
//! environment proxy, and redirects are refused, so the bearer key cannot
//! leave the Juniper-owned server.

use super::protocol::{self, Channel, ParsedTurn};
use super::tokenizer::{self as tk, J1Tokenizer};
use crate::commands::Cancellation;
use serde_json::{Value, json};
use std::net::SocketAddr;
#[cfg(test)]
use std::sync::Mutex;
use tokio::time::Duration;

pub const COMPLETION_PATH: &str = "/completion";
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// A forcing bias in the self-test; bans are applied after it and still win.
const FORCE: f32 = 100.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Sampling {
    pub temperature: f32,
    pub top_k: u32,
    pub top_p: f32,
    pub seed: u32,
}

impl Sampling {
    pub fn greedy() -> Self {
        Self {
            temperature: 0.0,
            top_k: 1,
            top_p: 1.0,
            seed: 0,
        }
    }

    /// Every sampler is set explicitly; llama-server's defaults (top-k 40,
    /// min-p, penalties, DRY, XTC) are not the reference's.
    fn write(&self, body: &mut serde_json::Map<String, Value>) {
        for (key, value) in [
            ("samplers", json!(["top_k", "top_p", "temperature"])),
            ("temperature", json!(self.temperature)),
            ("top_k", json!(self.top_k)),
            ("top_p", json!(self.top_p)),
            ("min_p", json!(0.0)),
            ("typical_p", json!(1.0)),
            ("repeat_penalty", json!(1.0)),
            ("presence_penalty", json!(0.0)),
            ("frequency_penalty", json!(0.0)),
            ("dry_multiplier", json!(0.0)),
            ("xtc_probability", json!(0.0)),
            ("seed", json!(self.seed)),
        ] {
            body.insert(key.into(), value);
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Completion {
    pub ids: Vec<u32>,
    pub stop_type: String,
    pub tokens_evaluated: Option<u64>,
    pub tokens_cached: Option<u64>,
}

/// Anything that completes an integer prompt. The production implementation
/// is `TokenIdClient`; tests wrap it to force specific IDs through the real
/// server.
pub trait CompletionSource: Sync {
    fn complete(
        &self,
        prompt: &[u32],
        n_predict: u32,
        sampling: &Sampling,
        extra_bias: &[(u32, f32)],
        cancellation: &Cancellation,
    ) -> impl std::future::Future<Output = Result<Completion, String>> + Send;
}

/// `http://<literal loopback IP>:<port>` and nothing else: no host names, no
/// user info, no path, query, or fragment, no implicit port.
pub fn loopback_base(url: &str) -> Result<String, String> {
    let refused = || {
        format!(
            "JUNIPER1_RUNTIME_URL_REFUSED: The Juniper 1 runtime address must be plain http://<loopback IP>:<port>, got {:?}.",
            url.chars().take(120).collect::<String>()
        )
    };
    let rest = url.strip_prefix("http://").ok_or_else(refused)?;
    let authority = rest.strip_suffix('/').unwrap_or(rest);
    if authority.is_empty()
        || authority.chars().any(|character| {
            matches!(character, '/' | '?' | '#' | '@' | '\\')
                || character.is_whitespace()
                || character.is_control()
        })
    {
        return Err(refused());
    }
    let address: SocketAddr = authority.parse().map_err(|_| refused())?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err(refused());
    }
    Ok(format!("http://{address}"))
}

pub struct TokenIdClient {
    base: String,
    api_key: Option<String>,
    vocab: u32,
    http: reqwest::Client,
    #[cfg(target_os = "linux")]
    owner: Option<(u32, u16)>,
    request_timeout: Duration,
    #[cfg(test)]
    paths_called: Mutex<Vec<String>>,
}

impl std::fmt::Debug for TokenIdClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TokenIdClient")
            .field("base", &self.base)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("vocab", &self.vocab)
            .finish()
    }
}

impl TokenIdClient {
    /// `owner` is the (PID, port) that must hold the loopback listener before
    /// the key is sent; Juniper sets it for the server it spawned.
    pub fn new(
        base_url: &str,
        vocab: u32,
        api_key: Option<String>,
        request_timeout: Duration,
        owner: Option<(u32, u16)>,
    ) -> Result<Self, String> {
        let base = loopback_base(base_url)?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|_| {
                "LOCAL_RUNTIME_FAILED: The runtime client could not be initialized.".to_owned()
            })?;
        #[cfg(not(target_os = "linux"))]
        let _ = owner;
        Ok(Self {
            base,
            api_key,
            vocab,
            http,
            #[cfg(target_os = "linux")]
            owner,
            request_timeout,
            #[cfg(test)]
            paths_called: Mutex::new(Vec::new()),
        })
    }

    #[cfg(test)]
    pub fn paths_called(&self) -> Vec<String> {
        self.paths_called
            .lock()
            .map(|paths| paths.clone())
            .unwrap_or_default()
    }

    fn ensure_owner(&self) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        if let Some((pid, port)) = self.owner
            && !crate::local_runtime::listener_belongs_to(pid, port)
        {
            return Err(
                "LOCAL_RUNTIME_UNAVAILABLE: The resident server no longer owns its local port."
                    .into(),
            );
        }
        Ok(())
    }

    fn body(
        &self,
        prompt: &[u32],
        n_predict: u32,
        sampling: &Sampling,
        extra_bias: &[(u32, f32)],
    ) -> Result<Value, String> {
        if prompt.is_empty() || prompt.iter().any(|id| *id >= self.vocab) {
            return Err("JUNIPER1_PROMPT_INVALID: The prompt must be a non-empty list of in-vocabulary token IDs.".into());
        }
        if n_predict < 1 {
            return Err("JUNIPER1_PROMPT_INVALID: n_predict must be positive.".into());
        }
        if extra_bias
            .iter()
            .any(|(id, bias)| *id >= self.vocab || !bias.is_finite())
        {
            return Err("JUNIPER1_PROMPT_INVALID: A logit bias is outside the vocabulary.".into());
        }
        // Bans come last so no earlier bias can re-enable a banned ID.
        let mut bias: Vec<Value> = extra_bias
            .iter()
            .map(|(id, value)| json!([id, value]))
            .collect();
        bias.extend(
            protocol::forbidden_in_generation()
                .into_iter()
                .map(|id| json!([id, false])),
        );
        let mut body = serde_json::Map::new();
        body.insert("prompt".into(), json!(prompt));
        body.insert("n_predict".into(), json!(n_predict));
        body.insert("stream".into(), json!(false));
        body.insert("return_tokens".into(), json!(true));
        body.insert("cache_prompt".into(), json!(true));
        body.insert("n_probs".into(), json!(0));
        body.insert("logit_bias".into(), Value::Array(bias));
        sampling.write(&mut body);
        Ok(Value::Object(body))
    }

    /// Validates a `/completion` result: an object whose `tokens` are
    /// in-vocabulary integers, no more than requested, for a prompt evaluated
    /// as sent.
    fn completion_from(
        &self,
        bytes: &[u8],
        prompt_len: usize,
        n_predict: u32,
    ) -> Result<Completion, String> {
        let invalid =
            |what: &str| format!("JUNIPER1_RUNTIME_RESPONSE_INVALID: The runtime returned {what}.");
        let value: Value = serde_json::from_slice(bytes).map_err(|_| invalid("malformed JSON"))?;
        let object = value.as_object().ok_or_else(|| invalid("a non-object"))?;
        let tokens = object
            .get("tokens")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("no generated token IDs"))?;
        let ids = tokens
            .iter()
            .map(|token| {
                token
                    .as_u64()
                    .filter(|id| *id < u64::from(self.vocab))
                    .map(|id| id as u32)
            })
            .collect::<Option<Vec<u32>>>()
            .ok_or_else(|| invalid("a token ID that is not an in-vocabulary integer"))?;
        if ids.len() > n_predict as usize {
            return Err(invalid("more tokens than were requested"));
        }
        let tokens_evaluated = match object.get("tokens_evaluated") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_u64()
                    .ok_or_else(|| invalid("a malformed prompt token count"))?,
            ),
        };
        if tokens_evaluated.is_some_and(|count| count > prompt_len as u64) {
            return Err(invalid(
                "a prompt longer than the one sent (implicit BOS or specials)",
            ));
        }
        let stop_type = object
            .get("stop_type")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("no stop type"))?
            .to_owned();
        Ok(Completion {
            ids,
            stop_type,
            tokens_evaluated,
            tokens_cached: object.get("tokens_cached").and_then(Value::as_u64),
        })
    }

    async fn post_completion(
        &self,
        body: &Value,
        cancellation: &Cancellation,
    ) -> Result<Vec<u8>, String> {
        if cancellation.is_cancelled() {
            return Err("REQUEST_CANCELLED: Generation cancelled.".into());
        }
        self.ensure_owner()?;
        #[cfg(test)]
        if let Ok(mut paths) = self.paths_called.lock() {
            paths.push(COMPLETION_PATH.into());
        }
        let mut call = self
            .http
            .post(format!("{}{COMPLETION_PATH}", self.base))
            .timeout(self.request_timeout)
            .json(body);
        if let Some(key) = &self.api_key {
            call = call.bearer_auth(key);
        }
        let exchange = async {
            let response = call.send().await.map_err(|error| {
                if error.is_timeout() {
                    "LOCAL_RUNTIME_TIMEOUT: The local model did not answer in time.".to_owned()
                } else {
                    "LOCAL_RUNTIME_UNAVAILABLE: The local model is not responding.".to_owned()
                }
            })?;
            let status = response.status();
            if status.is_redirection() {
                return Err("JUNIPER1_RUNTIME_REDIRECT_REFUSED: The local runtime tried to redirect the request.".to_owned());
            }
            if !status.is_success() {
                return Err(format!(
                    "LOCAL_RUNTIME_ERROR: The local runtime returned HTTP {}.",
                    status.as_u16()
                ));
            }
            if response
                .content_length()
                .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
            {
                return Err(
                    "JUNIPER1_RUNTIME_RESPONSE_INVALID: The runtime response is too large."
                        .to_owned(),
                );
            }
            let bytes = response.bytes().await.map_err(|error| {
                if error.is_timeout() {
                    "LOCAL_RUNTIME_TIMEOUT: The local model did not answer in time.".to_owned()
                } else {
                    "LOCAL_RUNTIME_UNAVAILABLE: The local model stopped responding.".to_owned()
                }
            })?;
            if bytes.len() > MAX_RESPONSE_BYTES {
                return Err(
                    "JUNIPER1_RUNTIME_RESPONSE_INVALID: The runtime response is too large."
                        .to_owned(),
                );
            }
            Ok(bytes.to_vec())
        };
        tokio::select! {
            result = exchange => result,
            _ = cancellation.wait() => Err("REQUEST_CANCELLED: Generation cancelled.".into()),
        }
    }

    /// Forces each relevant ID: 9, 10, and 11 must end generation (they are
    /// in the GGUF's end-of-generation set) and banned ID 1 must never be
    /// sampled. Proves the bans and integer IDs work on this server.
    pub async fn self_test(
        &self,
        tok: &J1Tokenizer,
        cancellation: &Cancellation,
    ) -> Result<Vec<(u32, String)>, String> {
        let mut prompt = vec![tk::USER];
        prompt.extend(tok.encode("hi")?);
        prompt.extend([tk::END, tk::ASSISTANT]);
        let mut report = Vec::new();
        for id in [tk::END, tk::CALL, tk::EOT, tk::SYSTEM] {
            let completion = self
                .complete(
                    &prompt,
                    2,
                    &Sampling::greedy(),
                    &[(id, FORCE)],
                    cancellation,
                )
                .await?;
            report.push((id, format!("{}:{:?}", completion.stop_type, completion.ids)));
            let passed = if id == tk::SYSTEM {
                !completion.ids.contains(&id)
            } else {
                completion.stop_type == "eos" && completion.ids.first() == Some(&id)
            };
            if !passed {
                return Err(format!(
                    "JUNIPER1_RUNTIME_SELF_TEST_FAILED: Control ID {id} did not behave as qualified on this runtime."
                ));
            }
        }
        Ok(report)
    }
}

impl CompletionSource for TokenIdClient {
    async fn complete(
        &self,
        prompt: &[u32],
        n_predict: u32,
        sampling: &Sampling,
        extra_bias: &[(u32, f32)],
        cancellation: &Cancellation,
    ) -> Result<Completion, String> {
        let body = self.body(prompt, n_predict, sampling, extra_bias)?;
        let bytes = self.post_completion(&body, cancellation).await?;
        self.completion_from(&bytes, prompt.len(), n_predict)
    }
}

#[derive(Clone, Debug)]
pub struct TurnGeneration {
    pub parsed: ParsedTurn,
    pub generated: Vec<u32>,
    /// The generation stopped because the turn budget or the context ran out.
    pub exhausted: bool,
}

/// The channel of the last channel token in `ids`.
fn open_channel(ids: &[u32]) -> Option<Channel> {
    ids.iter().rev().find_map(|id| match *id {
        tk::ANALYSIS => Some(Channel::Analysis),
        tk::FINAL => Some(Channel::Final),
        tk::TOOL_CALL => Some(Channel::ToolCall),
        _ => None,
    })
}

/// One assistant turn after a host `<|assistant|>`: generation is resubmitted
/// after `<|end|>` closes an analysis message, within `budget` generated IDs
/// in total and the server's context, then parsed. Nothing is accepted
/// unless the parser accepts it.
#[allow(clippy::too_many_arguments)]
pub async fn generate_turn<S: CompletionSource>(
    source: &S,
    tok: &J1Tokenizer,
    prompt: &[u32],
    budget: u32,
    context_size: u32,
    sampling: &Sampling,
    max_segments: u32,
    turn: u32,
    cancellation: &Cancellation,
) -> Result<TurnGeneration, String> {
    let mut generated: Vec<u32> = Vec::new();
    let mut segments = 0;
    let mut exhausted = false;
    while segments < max_segments {
        let remaining = budget.saturating_sub(generated.len() as u32);
        let room = (context_size as usize).saturating_sub(prompt.len() + generated.len()) as u32;
        let n_predict = remaining.min(room);
        if n_predict < 1 {
            exhausted = true;
            break;
        }
        let mut sequence = Vec::with_capacity(prompt.len() + generated.len());
        sequence.extend_from_slice(prompt);
        sequence.extend_from_slice(&generated);
        let completion = source
            .complete(&sequence, n_predict, sampling, &[], cancellation)
            .await?;
        segments += 1;
        generated.extend_from_slice(&completion.ids);
        let last = completion.ids.last().copied();
        if last.is_none() || matches!(last, Some(tk::CALL | tk::EOT)) {
            break;
        }
        if completion.stop_type != "eos" {
            exhausted = completion.stop_type == "limit";
            break;
        }
        if last == Some(tk::END) && open_channel(&generated) == Some(Channel::Analysis) {
            // The model-side header of the message after its analysis.
            generated.push(tk::ASSISTANT);
            continue;
        }
        break;
    }
    Ok(TurnGeneration {
        parsed: protocol::parse_generation(tok, &generated, turn),
        generated,
        exhausted,
    })
}

#[cfg(test)]
pub(crate) mod forcing {
    //! Forces scripted IDs through a real `CompletionSource` one at a time
    //! (logit bias +100, bans still apply), then lets it sample freely. Used
    //! to drive host paths, such as tool calls, that the model under test was
    //! never trained to produce; a forced call says nothing about the model.

    use super::*;

    pub struct Forced<'a, S> {
        pub inner: &'a S,
        script: Vec<u32>,
        position: Mutex<usize>,
    }

    impl<'a, S: CompletionSource> Forced<'a, S> {
        pub fn new(inner: &'a S, script: Vec<u32>) -> Self {
            Self {
                inner,
                script,
                position: Mutex::new(0),
            }
        }

        fn next(&self) -> Option<u32> {
            let position = *self.position.lock().expect("position");
            self.script.get(position).copied()
        }

        fn advance(&self) {
            *self.position.lock().expect("position") += 1;
        }
    }

    impl<S: CompletionSource> CompletionSource for Forced<'_, S> {
        async fn complete(
            &self,
            prompt: &[u32],
            n_predict: u32,
            sampling: &Sampling,
            extra_bias: &[(u32, f32)],
            cancellation: &Cancellation,
        ) -> Result<Completion, String> {
            let mut local: Vec<u32> = Vec::new();
            let mut last = None;
            while (local.len() as u32) < n_predict
                && let Some(id) = self.next()
            {
                let mut sequence = prompt.to_vec();
                sequence.extend_from_slice(&local);
                let mut bias = vec![(id, FORCE)];
                bias.extend_from_slice(extra_bias);
                let completion = self
                    .inner
                    .complete(&sequence, 1, sampling, &bias, cancellation)
                    .await?;
                if completion.ids != [id] {
                    return Err(format!(
                        "forced ID {id} was not sampled: {:?}",
                        completion.ids
                    ));
                }
                self.advance();
                local.push(id);
                let stop = completion.stop_type.clone();
                last = Some(completion);
                if stop == "eos" {
                    return Ok(Completion {
                        ids: local,
                        stop_type: stop,
                        ..last.expect("completion")
                    });
                }
            }
            if local.len() as u32 == n_predict {
                return Ok(Completion {
                    ids: local,
                    stop_type: "limit".into(),
                    ..last.expect("completion")
                });
            }
            let mut sequence = prompt.to_vec();
            sequence.extend_from_slice(&local);
            let mut free = self
                .inner
                .complete(
                    &sequence,
                    n_predict - local.len() as u32,
                    sampling,
                    extra_bias,
                    cancellation,
                )
                .await?;
            local.extend_from_slice(&free.ids);
            free.ids = local;
            Ok(free)
        }
    }
}
