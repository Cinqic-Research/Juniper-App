//! Refuses Juniper 1 GGUF files on every text-mode model path (Juniper 1 finding A-02).
//!
//! llama.cpp's text tokenizer for the exported Juniper 1 GGUF does not reproduce the canonical
//! `j1tok` tokenizer, and with special-token parsing it turns literal `<|system|>` text into
//! control IDs. Juniper 1 may only run behind a host that tokenizes with `j1tok` and sends token
//! IDs. Until that backend exists, any GGUF that identifies itself as Juniper 1 is refused before
//! it can reach `llama-server`'s chat/text endpoints or an Ollama import.
//!
//! A file is treated as Juniper 1 when its metadata holds any `juniper.*` key (the exporter writes
//! `juniper.io_contract = "token-ids-only"`) or a `general.name` beginning with `juniper-1`
//! (exports made before the marker existed). The reader only walks the key/value header, skips
//! arrays and strings without allocating them, bounds every length, and fails closed on a
//! malformed header.
//!
//! The same reader gives the token-ID backend (`crate::juniper1`) the metadata it must verify
//! before it starts a Juniper 1 model; that backend never goes through this text-path refusal.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufReader, Read, Seek, SeekFrom};

const MAGIC: &[u8; 4] = b"GGUF";
const MAX_KEY_BYTES: u64 = 1 << 16;
const MAX_KV_COUNT: u64 = 1 << 20;
const MAX_ARRAY_DEPTH: usize = 16;
const MAX_INSPECTED_STRING: u64 = 1 << 12;

#[derive(Debug, PartialEq, Eq)]
pub enum Identity {
    Other,
    JuniperOne { io_contract: Option<String> },
}

fn read_u32<R: Read>(r: &mut R) -> io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u64<R: Read>(r: &mut R) -> io::Result<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_owned())
}

fn skip<R: Read + Seek>(r: &mut R, n: u64) -> io::Result<()> {
    if n <= 1 << 16 {
        // Short skips read through the buffer (seeking would discard it on every token string).
        let copied = io::copy(&mut r.by_ref().take(n), &mut io::sink())?;
        return if copied == n {
            Ok(())
        } else {
            Err(io::ErrorKind::UnexpectedEof.into())
        };
    }
    let n = i64::try_from(n).map_err(|_| invalid("length overflow"))?;
    r.seek(SeekFrom::Current(n)).map(|_| ())
}

/// Reads a GGUF string; returns its text only when it is short enough to inspect.
fn read_string<R: Read + Seek>(r: &mut R) -> io::Result<Option<String>> {
    let len = read_u64(r)?;
    if len > MAX_INSPECTED_STRING {
        skip(r, len)?;
        return Ok(None);
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)?;
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

fn scalar_size(kind: u32) -> Option<u64> {
    match kind {
        0 | 1 | 7 => Some(1),
        2 | 3 => Some(2),
        4..=6 => Some(4),
        10..=12 => Some(8),
        _ => None,
    }
}

fn skip_value<R: Read + Seek>(r: &mut R, kind: u32) -> io::Result<()> {
    skip_value_at_depth(r, kind, 0)
}

fn skip_value_at_depth<R: Read + Seek>(r: &mut R, kind: u32, depth: usize) -> io::Result<()> {
    if depth > MAX_ARRAY_DEPTH {
        return Err(invalid("GGUF arrays are nested too deeply"));
    }
    if let Some(size) = scalar_size(kind) {
        return skip(r, size);
    }
    match kind {
        8 => read_string(r).map(|_| ()),
        9 => {
            let elem = read_u32(r)?;
            let count = read_u64(r)?;
            if let Some(size) = scalar_size(elem) {
                return skip(
                    r,
                    size.checked_mul(count)
                        .ok_or_else(|| invalid("array overflow"))?,
                );
            }
            if elem != 8 && elem != 9 {
                return Err(invalid("unsupported GGUF array element type"));
            }
            for _ in 0..count {
                skip_value_at_depth(r, elem, depth + 1)?; // strings, or nested arrays
            }
            Ok(())
        }
        _ => Err(invalid("unknown GGUF value type")),
    }
}

/// "juniper-1 …", "Juniper 1", "juniper1-…" (but not "juniper12…"): older exports carried no marker.
fn names_juniper_one(name: &str) -> bool {
    let compact: String = name
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    compact.starts_with("juniper1") && !compact[8..].starts_with(|c: char| c.is_ascii_digit())
}

/// The metadata a model check needs, read from the key/value header. Only short strings and
/// scalars under `general.`, `juniper.`, `tokenizer.ggml.`, and `*.context_length` are kept;
/// arrays are kept as element counts.
#[derive(Debug, Default, PartialEq)]
#[cfg_attr(target_os = "android", allow(dead_code))]
pub struct Header {
    pub strings: BTreeMap<String, String>,
    pub integers: BTreeMap<String, u64>,
    pub bools: BTreeMap<String, bool>,
    pub array_lengths: BTreeMap<String, u64>,
    pub juniper_keys: BTreeSet<String>,
}

impl Header {
    pub fn string(&self, key: &str) -> Option<&str> {
        self.strings.get(key).map(String::as_str)
    }
}

fn kept(key: &str) -> bool {
    key.starts_with("general.")
        || key.starts_with("juniper.")
        || key.starts_with("tokenizer.ggml.")
        || key.ends_with(".context_length")
}

fn read_scalar<R: Read>(r: &mut R, kind: u32) -> io::Result<Option<u64>> {
    Ok(match kind {
        0 => {
            let mut b = [0u8; 1];
            r.read_exact(&mut b)?;
            Some(u64::from(b[0]))
        }
        2 => {
            let mut b = [0u8; 2];
            r.read_exact(&mut b)?;
            Some(u64::from(u16::from_le_bytes(b)))
        }
        4 => Some(u64::from(read_u32(r)?)),
        5 => u64::try_from(read_u32(r)? as i32).ok(),
        10 => Some(read_u64(r)?),
        11 => u64::try_from(read_u64(r)? as i64).ok(),
        _ => None,
    })
}

/// Reads the key/value header. The reader is left at an unspecified position.
pub fn read_header<R: Read + Seek>(r: &mut R) -> io::Result<Header> {
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(invalid("not a GGUF file"));
    }
    let version = read_u32(r)?;
    if !(2..=3).contains(&version) {
        return Err(invalid("unsupported GGUF version"));
    }
    let _tensors = read_u64(r)?;
    let kv_count = read_u64(r)?;
    if kv_count > MAX_KV_COUNT {
        return Err(invalid("implausible GGUF metadata count"));
    }
    let mut header = Header::default();
    for _ in 0..kv_count {
        let key_len = read_u64(r)?;
        if key_len == 0 || key_len > MAX_KEY_BYTES {
            return Err(invalid("implausible GGUF key length"));
        }
        let mut key = vec![0u8; key_len as usize];
        r.read_exact(&mut key)?;
        let key = String::from_utf8_lossy(&key).into_owned();
        let kind = read_u32(r)?;
        if key.starts_with("juniper.") {
            header.juniper_keys.insert(key.clone());
        }
        if !kept(&key) {
            skip_value(r, kind)?;
            continue;
        }
        match kind {
            8 => {
                let value = read_string(r)?.unwrap_or_default();
                header.strings.insert(key, value);
            }
            7 => {
                let mut b = [0u8; 1];
                r.read_exact(&mut b)?;
                header.bools.insert(key, b[0] != 0);
            }
            9 => {
                let elem = read_u32(r)?;
                let count = read_u64(r)?;
                header.array_lengths.insert(key, count);
                if let Some(size) = scalar_size(elem) {
                    skip(
                        r,
                        size.checked_mul(count)
                            .ok_or_else(|| invalid("array overflow"))?,
                    )?;
                } else if elem == 8 || elem == 9 {
                    for _ in 0..count {
                        skip_value(r, elem)?;
                    }
                } else {
                    return Err(invalid("unsupported GGUF array element type"));
                }
            }
            0 | 2 | 4 | 5 | 10 | 11 => {
                if let Some(value) = read_scalar(r, kind)? {
                    header.integers.insert(key, value);
                }
            }
            _ => skip_value(r, kind)?,
        }
    }
    Ok(header)
}

/// Classifies a GGUF stream by its metadata. The reader is left at an unspecified position.
pub fn identify<R: Read + Seek>(r: &mut R) -> io::Result<Identity> {
    let header = read_header(r)?;
    let juniper = !header.juniper_keys.is_empty()
        || header.string("general.name").is_some_and(names_juniper_one);
    Ok(if juniper {
        Identity::JuniperOne {
            io_contract: header.strings.get("juniper.io_contract").cloned(),
        }
    } else {
        Identity::Other
    })
}

/// Fails unless the file is a well-formed GGUF that is not Juniper 1 (fail closed).
pub fn refuse_juniper_on_text_path<R: Read + Seek>(r: &mut R) -> Result<(), String> {
    let start = r
        .stream_position()
        .map_err(|_| "The selected model file could not be read.".to_owned())?;
    let verdict = identify(&mut BufReader::with_capacity(1 << 16, &mut *r));
    r.seek(SeekFrom::Start(start))
        .map_err(|_| "The selected model file could not be read.".to_owned())?;
    match verdict {
        Ok(Identity::Other) => Ok(()),
        Ok(Identity::JuniperOne { .. }) => Err(
            "Juniper 1 models need Juniper's own tokenizer and token-ID runtime; they cannot be loaded through a text-mode path."
                .to_owned(),
        ),
        Err(_) => Err("The selected file is not a readable GGUF model.".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn s(out: &mut Vec<u8>, text: &str) {
        out.extend((text.len() as u64).to_le_bytes());
        out.extend(text.as_bytes());
    }

    fn gguf(kvs: &[(&str, &str)], token_array: usize) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(MAGIC);
        out.extend(3u32.to_le_bytes());
        out.extend(0u64.to_le_bytes());
        out.extend(((kvs.len() + 2) as u64).to_le_bytes());
        s(&mut out, "general.architecture");
        out.extend(8u32.to_le_bytes());
        s(&mut out, "qwen3");
        s(&mut out, "tokenizer.ggml.tokens");
        out.extend(9u32.to_le_bytes());
        out.extend(8u32.to_le_bytes());
        out.extend((token_array as u64).to_le_bytes());
        for i in 0..token_array {
            s(&mut out, &format!("<|t{i}|>"));
        }
        for (k, v) in kvs {
            s(&mut out, k);
            out.extend(8u32.to_le_bytes());
            s(&mut out, v);
        }
        out
    }

    #[test]
    fn marked_juniper_export_is_refused_and_position_restored() {
        let bytes = gguf(&[("juniper.io_contract", "token-ids-only")], 64);
        let mut cur = Cursor::new(bytes);
        assert_eq!(
            identify(&mut cur).unwrap(),
            Identity::JuniperOne {
                io_contract: Some("token-ids-only".into())
            }
        );
        cur.set_position(0);
        assert!(refuse_juniper_on_text_path(&mut cur).is_err());
        assert_eq!(cur.position(), 0);
    }

    #[test]
    fn unmarked_legacy_juniper_export_is_refused_by_name() {
        let bytes = gguf(&[("general.name", "juniper-1 (experimental export)")], 8);
        assert!(refuse_juniper_on_text_path(&mut Cursor::new(bytes)).is_err());
    }

    #[test]
    fn legacy_name_variants() {
        for name in [
            "juniper-1 (experimental export)",
            "Juniper 1",
            "juniper1-base",
            "JUNIPER_1",
        ] {
            assert!(names_juniper_one(name), "{name}");
        }
        for name in ["juniper12b", "gpt-oss-20b", "juniper-app"] {
            assert!(!names_juniper_one(name), "{name}");
        }
    }

    #[test]
    fn nested_string_arrays_are_skipped() {
        let mut out = Vec::new();
        out.extend(MAGIC);
        out.extend(3u32.to_le_bytes());
        out.extend(0u64.to_le_bytes());
        out.extend(1u64.to_le_bytes());
        s(&mut out, "x.nested");
        out.extend(9u32.to_le_bytes()); // array
        out.extend(9u32.to_le_bytes()); // of arrays
        out.extend(1u64.to_le_bytes());
        out.extend(8u32.to_le_bytes()); // of strings
        out.extend(2u64.to_le_bytes());
        s(&mut out, "a");
        s(&mut out, "b");
        assert_eq!(identify(&mut Cursor::new(out)).unwrap(), Identity::Other);
    }

    #[test]
    fn deeply_nested_metadata_is_refused_on_a_small_stack() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                for key in ["x.nested", "general.nested"] {
                    let mut bytes = Vec::new();
                    bytes.extend(MAGIC);
                    bytes.extend(3u32.to_le_bytes());
                    bytes.extend(0u64.to_le_bytes());
                    bytes.extend(1u64.to_le_bytes());
                    s(&mut bytes, key);
                    bytes.extend(9u32.to_le_bytes());
                    for _ in 0..4096 {
                        bytes.extend(9u32.to_le_bytes());
                        bytes.extend(1u64.to_le_bytes());
                    }
                    bytes.extend(8u32.to_le_bytes());
                    bytes.extend(0u64.to_le_bytes());
                    let mut cursor = Cursor::new(bytes);
                    let error = read_header(&mut cursor).unwrap_err();
                    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                    assert!(error.to_string().contains("nested too deeply"));
                    cursor.set_position(0);
                    assert!(refuse_juniper_on_text_path(&mut cursor).is_err());
                    assert_eq!(cursor.position(), 0);
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn other_models_pass() {
        let bytes = gguf(&[("general.name", "gpt-oss-20b")], 24576);
        assert_eq!(
            identify(&mut Cursor::new(bytes.clone())).unwrap(),
            Identity::Other
        );
        assert!(refuse_juniper_on_text_path(&mut Cursor::new(bytes)).is_ok());
    }

    /// Android has no token-ID backend: its JNI renders a chat template and
    /// tokenizes text with special parsing. Juniper 1 must stay refused on the
    /// route that reaches it (`android_runtime.rs` is compiled only for Android,
    /// so this checks its source), and no Juniper 1 artifact is resolvable there.
    #[test]
    fn android_native_route_refuses_juniper_one_before_loading() {
        let source = include_str!("android_runtime.rs");
        let stream = &source[source
            .find("pub async fn stream_chat")
            .expect("Android stream_chat")..];
        let guard = stream
            .find("crate::gguf_guard::refuse_juniper_on_text_path(&mut model_file)?")
            .expect("the Android route calls the guard");
        let load = stream
            .find("wait_until_ready(")
            .expect("the Android route loads the model");
        let generate = stream.find("\"startGenerate\"").expect("generation call");
        assert!(guard < load && guard < generate);
        let jni =
            include_str!("../plugins/juniper-local/android/src/main/cpp/juniper_llama_jni.cpp");
        assert!(
            jni.contains("common_tokenize(context, chat_params.prompt, true, true)"),
            "the JNI changed; re-audit whether it can serve token IDs before relaxing this"
        );
        for id in ["juniper1-base-v1-f16", "juniper1-sft-pilot-v1-f16"] {
            assert!(crate::catalog::find(id).is_err(), "{id}");
        }
        let marked = gguf(&[("juniper.io_contract", "token-ids-only")], 4);
        assert!(refuse_juniper_on_text_path(&mut Cursor::new(marked)).is_err());
    }

    #[test]
    fn header_reader_keeps_typed_metadata() {
        let mut out = gguf(
            &[
                ("juniper.io_contract", "token-ids-only"),
                ("juniper.protocol", "juniper-chat-v1"),
            ],
            3,
        );
        // Append a u32 context length and a bool, and fix the count.
        let count = u64::from_le_bytes(out[16..24].try_into().unwrap()) + 2;
        out[16..24].copy_from_slice(&count.to_le_bytes());
        s(&mut out, "qwen3.context_length");
        out.extend(4u32.to_le_bytes());
        out.extend(2048u32.to_le_bytes());
        s(&mut out, "tokenizer.ggml.add_bos_token");
        out.extend(7u32.to_le_bytes());
        out.push(0);
        let header = read_header(&mut Cursor::new(out)).expect("header");
        assert_eq!(header.string("juniper.protocol"), Some("juniper-chat-v1"));
        assert_eq!(header.string("general.architecture"), Some("qwen3"));
        assert_eq!(header.integers.get("qwen3.context_length"), Some(&2048));
        assert_eq!(
            header.bools.get("tokenizer.ggml.add_bos_token"),
            Some(&false)
        );
        assert_eq!(header.array_lengths.get("tokenizer.ggml.tokens"), Some(&3));
        assert_eq!(header.juniper_keys.len(), 2);
    }

    #[test]
    fn malformed_or_truncated_headers_fail_closed() {
        let good = gguf(&[("general.name", "gpt-oss-20b")], 4);
        for cut in [0, 3, 10, 30, good.len() - 1] {
            assert!(refuse_juniper_on_text_path(&mut Cursor::new(good[..cut].to_vec())).is_err());
        }
        let mut wrong_magic = good.clone();
        wrong_magic[0] = b'X';
        assert!(refuse_juniper_on_text_path(&mut Cursor::new(wrong_magic)).is_err());
        let mut huge = good;
        huge[16..24].copy_from_slice(&(u64::MAX).to_le_bytes());
        assert!(refuse_juniper_on_text_path(&mut Cursor::new(huge)).is_err());
    }
}
