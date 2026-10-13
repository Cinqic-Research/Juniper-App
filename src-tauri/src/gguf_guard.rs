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

use std::io::{self, Read, Seek, SeekFrom};

const MAGIC: &[u8; 4] = b"GGUF";
const MAX_KEY_BYTES: u64 = 1 << 16;
const MAX_KV_COUNT: u64 = 1 << 20;
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

fn skip<R: Seek>(r: &mut R, n: u64) -> io::Result<()> {
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
            if elem != 8 {
                return Err(invalid("unsupported GGUF array element type"));
            }
            for _ in 0..count {
                let len = read_u64(r)?;
                skip(r, len)?;
            }
            Ok(())
        }
        _ => Err(invalid("unknown GGUF value type")),
    }
}

/// Classifies a GGUF stream by its metadata. The reader is left at an unspecified position.
pub fn identify<R: Read + Seek>(r: &mut R) -> io::Result<Identity> {
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
    let mut juniper = false;
    let mut io_contract = None;
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
            juniper = true;
        }
        if kind == 8 && (key == "juniper.io_contract" || key == "general.name") {
            let value = read_string(r)?.unwrap_or_default();
            if key == "general.name" && value.to_ascii_lowercase().starts_with("juniper-1") {
                juniper = true;
            }
            if key == "juniper.io_contract" {
                io_contract = Some(value);
            }
            continue;
        }
        skip_value(r, kind)?;
    }
    Ok(if juniper {
        Identity::JuniperOne { io_contract }
    } else {
        Identity::Other
    })
}

/// Fails unless the file is a well-formed GGUF that is not Juniper 1 (fail closed).
pub fn refuse_juniper_on_text_path<R: Read + Seek>(r: &mut R) -> Result<(), String> {
    let start = r
        .stream_position()
        .map_err(|_| "The selected model file could not be read.".to_owned())?;
    let verdict = identify(r);
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
    fn other_models_pass() {
        let bytes = gguf(&[("general.name", "gpt-oss-20b")], 24576);
        assert_eq!(
            identify(&mut Cursor::new(bytes.clone())).unwrap(),
            Identity::Other
        );
        assert!(refuse_juniper_on_text_path(&mut Cursor::new(bytes)).is_ok());
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
