//! Juniper 1's canonical tokenizer (`j1tok`), loaded exactly as the reference
//! `j1data.jtok.load` loads it.
//!
//! ID layout: 0-63 control tokens (twelve named, the rest `<|reserved_NN|>`),
//! 64-319 the 256 byte-fallback tokens, then learned merges. IDs are never
//! reordered or reassigned.
//!
//! `tokenizer.json` does not persist `encode_special_tokens`, so a plain load
//! would encode literal `<|system|>` text as control ID 1. The loader enables
//! it and checks that it took effect; control IDs then enter a sequence only
//! when the protocol serializer inserts them by ID.

use sha2::{Digest, Sha256};
use tokenizers::Tokenizer;

pub const N_CONTROL: u32 = 64;
pub const FIRST_BYTE_ID: u32 = N_CONTROL;
pub const FIRST_LEARNED_ID: u32 = N_CONTROL + 256;
/// A `tokenizer.json` larger than this is not a Juniper tokenizer.
pub const MAX_TOKENIZER_BYTES: u64 = 16 * 1024 * 1024;

/// Named control tokens; their position is their ID.
pub const NAMED_CONTROL: [&str; 12] = [
    "<|endoftext|>",
    "<|system|>",
    "<|developer|>",
    "<|user|>",
    "<|assistant|>",
    "<|tool|>",
    "<|analysis|>",
    "<|final|>",
    "<|tool_call|>",
    "<|end|>",
    "<|call|>",
    "<|eot|>",
];

pub const ENDOFTEXT: u32 = 0;
pub const SYSTEM: u32 = 1;
pub const DEVELOPER: u32 = 2;
pub const USER: u32 = 3;
pub const ASSISTANT: u32 = 4;
pub const TOOL: u32 = 5;
pub const ANALYSIS: u32 = 6;
pub const FINAL: u32 = 7;
pub const TOOL_CALL: u32 = 8;
pub const END: u32 = 9;
pub const CALL: u32 = 10;
pub const EOT: u32 = 11;

/// The spelling of control ID `id` (`id < 64`).
pub fn control_spelling(id: u32) -> String {
    NAMED_CONTROL
        .get(id as usize)
        .map(|name| (*name).to_owned())
        .unwrap_or_else(|| format!("<|reserved_{id:02}|>"))
}

/// All 64 control spellings in ID order.
pub fn control_spellings() -> Vec<String> {
    (0..N_CONTROL).map(control_spelling).collect()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// The canonical tokenizer with `encode_special_tokens` on. There is no way to
/// obtain one with the setting off.
pub struct J1Tokenizer {
    inner: Tokenizer,
    sha256: String,
    vocab_size: u32,
}

impl std::fmt::Debug for J1Tokenizer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("J1Tokenizer")
            .field("sha256", &self.sha256)
            .field("vocab_size", &self.vocab_size)
            .finish()
    }
}

impl J1Tokenizer {
    /// Loads `tokenizer.json` bytes whose SHA-256 must equal `expected_sha256`
    /// and whose vocabulary must have `expected_vocab` IDs.
    pub fn load(bytes: &[u8], expected_sha256: &str, expected_vocab: u32) -> Result<Self, String> {
        if bytes.len() as u64 > MAX_TOKENIZER_BYTES {
            return Err(
                "JUNIPER1_TOKENIZER_INVALID: The Juniper 1 tokenizer file is implausibly large."
                    .into(),
            );
        }
        let sha256 = sha256_hex(bytes);
        if sha256 != expected_sha256 {
            return Err("JUNIPER1_TOKENIZER_MISMATCH: The Juniper 1 tokenizer is not the qualified j1tok file.".into());
        }
        let mut inner = Tokenizer::from_bytes(bytes).map_err(|_| {
            "JUNIPER1_TOKENIZER_INVALID: The Juniper 1 tokenizer could not be parsed.".to_owned()
        })?;
        inner.set_encode_special_tokens(true);
        if !inner.get_encode_special_tokens() {
            return Err("JUNIPER1_TOKENIZER_INVALID: Special-token encoding could not be disabled for text.".into());
        }
        let vocab = inner.get_vocab(true);
        if (0..N_CONTROL).any(|id| vocab.get(&control_spelling(id)) != Some(&id))
            || (0..=255u32)
                .any(|byte| vocab.get(&format!("<0x{byte:02X}>")) != Some(&(FIRST_BYTE_ID + byte)))
        {
            return Err(
                "JUNIPER1_TOKENIZER_INVALID: Control or byte-fallback token IDs do not match j1.tokenizer.v1."
                    .into(),
            );
        }
        let vocab_size = u32::try_from(inner.get_vocab_size(true)).unwrap_or(u32::MAX);
        if vocab_size != expected_vocab {
            return Err(
                "JUNIPER1_TOKENIZER_INVALID: The tokenizer vocabulary size is not the qualified one."
                    .into(),
            );
        }
        Ok(Self {
            inner,
            sha256,
            vocab_size,
        })
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn vocab_size(&self) -> u32 {
        self.vocab_size
    }

    /// Text to IDs without added specials; literal control spellings stay
    /// ordinary bytes.
    pub fn encode(&self, text: &str) -> Result<Vec<u32>, String> {
        self.inner
            .encode_char_offsets(text, false)
            .map(|encoding| encoding.get_ids().to_vec())
            .map_err(|_| "JUNIPER1_TOKENIZER_FAILED: Text could not be tokenized.".to_owned())
    }

    pub fn decode(&self, ids: &[u32]) -> Result<String, String> {
        self.inner
            .decode(ids, false)
            .map_err(|_| "JUNIPER1_TOKENIZER_FAILED: Token IDs could not be decoded.".to_owned())
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! A small tokenizer with j1tok's structure (64 controls, 256 byte-fallback
    //! tokens, the same pre-tokenizer and decoder, a handful of merges) so the
    //! protocol and client run in CI without the canonical file. It is not
    //! j1tok and nothing about canonical IDs is inferred from it.

    use super::*;
    use serde_json::{Value, json};

    pub const PRE_TOKENIZER_REGEX: &str = r"(?i:'s|'t|'re|'ve|'m|'ll|'d)|[^\r\n\p{L}\p{N}]?\p{L}+|\p{N}| ?[^\s\p{L}\p{N}]+[\r\n]*|\s*[\r\n]+|\s+(?!\S)|\s+";

    pub fn json() -> Vec<u8> {
        let mut vocab = serde_json::Map::new();
        for id in 0..N_CONTROL {
            vocab.insert(control_spelling(id), json!(id));
        }
        for byte in 0..=255u32 {
            vocab.insert(format!("<0x{byte:02X}>"), json!(FIRST_BYTE_ID + byte));
        }
        let mut next = FIRST_LEARNED_ID;
        let mut add = |token: &str, vocab: &mut serde_json::Map<String, Value>| {
            if !vocab.contains_key(token) {
                vocab.insert(token.to_owned(), json!(next));
                next += 1;
            }
        };
        for character in (0x20u8..0x7f).map(char::from) {
            add(&character.to_string(), &mut vocab);
        }
        add("\n", &mut vocab);
        add("\t", &mut vocab);
        let merges = [
            (" ", "t"),
            ("h", "e"),
            (" t", "he"),
            ("e", "r"),
            (" ", " "),
            ("  ", "  "),
            ("o", "k"),
            ("h", "i"),
            ("<", "|"),
            ("|", ">"),
        ];
        for (left, right) in merges {
            add(&format!("{left}{right}"), &mut vocab);
        }
        let added: Vec<Value> = (0..N_CONTROL)
            .map(|id| {
                json!({
                    "id": id, "content": control_spelling(id), "single_word": false,
                    "lstrip": false, "rstrip": false, "normalized": false, "special": true
                })
            })
            .collect();
        serde_json::to_vec(&json!({
            "version": "1.0",
            "truncation": null,
            "padding": null,
            "added_tokens": added,
            "normalizer": null,
            "pre_tokenizer": {
                "type": "Split",
                "pattern": { "Regex": PRE_TOKENIZER_REGEX },
                "behavior": "Isolated",
                "invert": false
            },
            "post_processor": null,
            "decoder": { "type": "Sequence", "decoders": [{ "type": "ByteFallback" }, { "type": "Fuse" }] },
            "model": {
                "type": "BPE", "dropout": null, "unk_token": null,
                "continuing_subword_prefix": null, "end_of_word_suffix": null,
                "fuse_unk": false, "byte_fallback": true, "ignore_merges": false,
                "vocab": vocab,
                "merges": merges.iter().map(|(left, right)| json!([left, right])).collect::<Vec<_>>()
            }
        }))
        .expect("test tokenizer serializes")
    }

    pub fn tokenizer() -> J1Tokenizer {
        let bytes = json();
        let vocab = Tokenizer::from_bytes(&bytes)
            .expect("test tokenizer parses")
            .get_vocab_size(true) as u32;
        J1Tokenizer::load(&bytes, &sha256_hex(&bytes), vocab).expect("test tokenizer loads")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_control_spellings_never_become_control_ids() {
        let tok = testing::tokenizer();
        for spelling in control_spellings() {
            let ids = tok.encode(&format!("a {spelling} b")).expect("encode");
            assert!(ids.iter().all(|id| *id >= N_CONTROL), "{spelling}: {ids:?}");
            assert_eq!(tok.decode(&ids).expect("decode"), format!("a {spelling} b"));
        }
    }

    #[test]
    fn the_encode_special_tokens_setting_is_load_bearing() {
        // Without the setter a plain load maps the literal spelling to the control ID.
        let bytes = testing::json();
        let plain = Tokenizer::from_bytes(&bytes).expect("parse");
        assert!(!plain.get_encode_special_tokens());
        assert_eq!(
            plain.encode("<|system|>", false).expect("encode").get_ids(),
            &[SYSTEM]
        );
        let tok = testing::tokenizer();
        assert_ne!(tok.encode("<|system|>").expect("encode"), vec![SYSTEM]);
    }

    #[test]
    fn hash_vocabulary_and_control_layout_are_enforced() {
        let bytes = testing::json();
        let vocab = Tokenizer::from_bytes(&bytes)
            .expect("parse")
            .get_vocab_size(true) as u32;
        let sha = sha256_hex(&bytes);
        assert!(
            J1Tokenizer::load(&bytes, &"0".repeat(64), vocab)
                .unwrap_err()
                .starts_with("JUNIPER1_TOKENIZER_MISMATCH")
        );
        assert!(
            J1Tokenizer::load(&bytes, &sha, vocab + 1)
                .unwrap_err()
                .starts_with("JUNIPER1_TOKENIZER_INVALID")
        );
        let garbage = b"{\"not\": \"a tokenizer\"}";
        assert!(
            J1Tokenizer::load(garbage, &sha256_hex(garbage), vocab)
                .unwrap_err()
                .starts_with("JUNIPER1_TOKENIZER_INVALID")
        );
        // Swapping two control spellings changes their IDs and is refused.
        let swapped = String::from_utf8(bytes.clone())
            .expect("utf8")
            .replace("<|user|>", "<|tmp|>")
            .replace("<|tool|>", "<|user|>")
            .replace("<|tmp|>", "<|tool|>");
        assert!(
            J1Tokenizer::load(swapped.as_bytes(), &sha256_hex(swapped.as_bytes()), vocab)
                .unwrap_err()
                .starts_with("JUNIPER1_TOKENIZER_INVALID")
        );
    }

    #[test]
    fn byte_fallback_round_trips_text_outside_the_vocabulary() {
        let tok = testing::tokenizer();
        for text in [
            "東京 🙂",
            "e\u{301}",
            "nul\u{0}x",
            "\u{2028}\u{ffff}",
            "\t\tx  \r\n",
        ] {
            let ids = tok.encode(text).expect("encode");
            assert_eq!(tok.decode(&ids).expect("decode"), text);
        }
        assert!(
            tok.encode("東")
                .expect("encode")
                .iter()
                .all(|id| (FIRST_BYTE_ID..FIRST_LEARNED_ID).contains(id))
        );
    }

    #[test]
    fn control_spellings_are_in_id_order() {
        let spellings = control_spellings();
        assert_eq!(spellings.len(), 64);
        assert_eq!(spellings[11], "<|eot|>");
        assert_eq!(spellings[12], "<|reserved_12|>");
        assert_eq!(spellings[63], "<|reserved_63|>");
    }
}
