//! SSH agent protocol primitives: wire format, fingerprinting, and message filtering.
//!
//! This is a direct extraction of the protocol layer from `cloister-ssh-filter/src/lib.rs`.

use std::io::{self, Read, Write};

use base64ct::{Base64Unpadded, Encoding};
use sha2::{Digest, Sha256};

// SSH agent protocol constants
pub const SSH_AGENTC_REQUEST_IDENTITIES: u8 = 11;
pub const SSH_AGENT_IDENTITIES_ANSWER: u8 = 12;
pub const SSH_AGENTC_SIGN_REQUEST: u8 = 13;
pub const FAILURE_BYTE: u8 = 5;
pub const SSH_AGENT_FAILURE: &[u8] = &[0, 0, 0, 1, FAILURE_BYTE];

/// Maximum message size (256 KB) to prevent memory exhaustion from malformed frames.
pub const MAX_MSG_LEN: u32 = 256 * 1024;

// --- Wire format helpers ---

/// Read a framed SSH agent message: u32 big-endian length + payload.
pub fn read_message(r: &mut dyn Read) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_MSG_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message too large: {len}"),
        ));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

/// Write a framed SSH agent message: u32 big-endian length + payload.
pub fn write_message(w: &mut dyn Write, msg: &[u8]) -> io::Result<()> {
    let len: u32 = msg
        .len()
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "message too large"))?;
    if len > MAX_MSG_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message too large: {len}"),
        ));
    }
    w.write_all(&len.to_be_bytes())?;
    w.write_all(msg)?;
    w.flush()
}

/// Parse a big-endian u32 at `offset` in `data`. Returns (value, new_offset).
pub fn read_u32(data: &[u8], offset: usize) -> Option<(u32, usize)> {
    if offset + 4 > data.len() {
        return None;
    }
    let val = u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]);
    Some((val, offset + 4))
}

/// Parse an SSH string (u32 length + bytes) at `offset`. Returns (&[u8], new_offset).
pub fn read_string(data: &[u8], offset: usize) -> Option<(&[u8], usize)> {
    let (len, off) = read_u32(data, offset)?;
    let end = off.checked_add(len as usize)?;
    if end > data.len() {
        return None;
    }
    Some((&data[off..end], end))
}

/// Compute SSH fingerprint: SHA256:<unpadded-base64 of SHA-256 hash of key blob>.
pub fn fingerprint(key_blob: &[u8]) -> String {
    let hash = Sha256::digest(key_blob);
    let encoded = Base64Unpadded::encode_string(&hash);
    format!("SHA256:{encoded}")
}

/// Encode an SSH string (u32 length prefix + bytes) and append to `out`.
pub fn write_string(out: &mut Vec<u8>, data: &[u8]) {
    let len: u32 = data.len().try_into().expect("SSH string exceeds u32::MAX");
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(data);
}

// --- Message filtering ---

/// Build an empty IDENTITIES_ANSWER (zero keys).
fn empty_identities() -> Vec<u8> {
    vec![SSH_AGENT_IDENTITIES_ANSWER, 0, 0, 0, 0]
}

/// Filter an IDENTITIES_ANSWER to only include keys whose fingerprint is in the allowlist.
pub fn filter_identities(response: &[u8], allowed: &[String]) -> Vec<u8> {
    if response.is_empty() || response[0] != SSH_AGENT_IDENTITIES_ANSWER {
        return vec![FAILURE_BYTE];
    }

    let (nkeys, mut offset) = match read_u32(response, 1) {
        Some(v) => v,
        None => return empty_identities(),
    };

    let mut kept: Vec<(&[u8], &[u8])> = Vec::new();
    for _ in 0..nkeys {
        let (key_blob, off2) = match read_string(response, offset) {
            Some(v) => v,
            None => return empty_identities(),
        };
        let (comment, off3) = match read_string(response, off2) {
            Some(v) => v,
            None => return empty_identities(),
        };
        offset = off3;

        let fp = fingerprint(key_blob);
        if allowed.contains(&fp) {
            kept.push((key_blob, comment));
        }
    }

    let mut out = Vec::new();
    out.push(SSH_AGENT_IDENTITIES_ANSWER);
    let nkeys: u32 = kept.len().try_into().expect("key count exceeds u32::MAX");
    out.extend_from_slice(&nkeys.to_be_bytes());
    for (key_blob, comment) in kept {
        write_string(&mut out, key_blob);
        write_string(&mut out, comment);
    }
    out
}

/// Check whether a sign request message targets a key in the allowlist.
pub fn is_sign_allowed(msg: &[u8], allowed: &[String]) -> bool {
    if msg.is_empty() || msg[0] != SSH_AGENTC_SIGN_REQUEST {
        return false;
    }
    match read_string(msg, 1) {
        Some((key_blob, _)) => {
            let fp = fingerprint(key_blob);
            allowed.contains(&fp)
        }
        None => false,
    }
}

// --- Test helpers ---

/// Build a mock IDENTITIES_ANSWER with the given (key_blob, comment) pairs.
pub fn build_identities_answer(keys: &[(&[u8], &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(SSH_AGENT_IDENTITIES_ANSWER);
    let nkeys: u32 = keys.len().try_into().expect("key count exceeds u32::MAX");
    out.extend_from_slice(&nkeys.to_be_bytes());
    for (blob, comment) in keys {
        write_string(&mut out, blob);
        write_string(&mut out, comment);
    }
    out
}

/// Build a mock SSH_AGENTC_SIGN_REQUEST message.
pub fn build_sign_request(key_blob: &[u8], data: &[u8]) -> Vec<u8> {
    let mut msg = Vec::new();
    msg.push(SSH_AGENTC_SIGN_REQUEST);
    write_string(&mut msg, key_blob);
    write_string(&mut msg, data);
    msg.extend_from_slice(&0u32.to_be_bytes()); // flags
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_write_message_roundtrip() {
        let payload = b"hello agent";
        let mut buf = Vec::new();
        write_message(&mut buf, payload).unwrap();

        let mut cursor = io::Cursor::new(buf);
        let result = read_message(&mut cursor).unwrap();
        assert_eq!(result, payload);
    }

    #[test]
    fn read_message_rejects_oversized() {
        let len = (MAX_MSG_LEN + 1).to_be_bytes();
        let mut cursor = io::Cursor::new(len.to_vec());
        let result = read_message(&mut cursor);
        assert!(result.is_err());
    }

    #[test]
    fn write_message_rejects_oversized() {
        let big = vec![0u8; (MAX_MSG_LEN + 1) as usize];
        let mut buf = Vec::new();
        let result = write_message(&mut buf, &big);
        assert!(result.is_err());
    }

    #[test]
    fn read_u32_basic() {
        let data = [0, 0, 0, 42, 0xFF];
        let (val, off) = read_u32(&data, 0).unwrap();
        assert_eq!(val, 42);
        assert_eq!(off, 4);
    }

    #[test]
    fn read_u32_out_of_bounds() {
        let data = [0, 0];
        assert!(read_u32(&data, 0).is_none());
    }

    #[test]
    fn read_string_basic() {
        let mut data = Vec::new();
        data.extend_from_slice(&3u32.to_be_bytes());
        data.extend_from_slice(b"abc");
        data.push(0xFF);
        let (s, off) = read_string(&data, 0).unwrap();
        assert_eq!(s, b"abc");
        assert_eq!(off, 7);
    }

    #[test]
    fn read_string_out_of_bounds() {
        let mut data = Vec::new();
        data.extend_from_slice(&100u32.to_be_bytes());
        data.extend_from_slice(b"short");
        assert!(read_string(&data, 0).is_none());
    }

    #[test]
    fn fingerprint_known_value() {
        let fp = fingerprint(b"");
        assert!(fp.starts_with("SHA256:"), "got: {fp}");
        assert_eq!(fp, "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU");
    }

    #[test]
    fn fingerprint_deterministic() {
        let fp1 = fingerprint(b"test key blob");
        let fp2 = fingerprint(b"test key blob");
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn fingerprint_different_inputs() {
        let fp1 = fingerprint(b"key1");
        let fp2 = fingerprint(b"key2");
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn filter_identities_keeps_allowed() {
        let key1 = b"allowed-key";
        let key2 = b"blocked-key";
        let fp1 = fingerprint(key1);

        let response = build_identities_answer(&[(key1, b"comment1"), (key2, b"comment2")]);
        let filtered = filter_identities(&response, &[fp1]);

        assert_eq!(filtered[0], SSH_AGENT_IDENTITIES_ANSWER);
        let (nkeys, offset) = read_u32(&filtered, 1).unwrap();
        assert_eq!(nkeys, 1);
        let (blob, _) = read_string(&filtered, offset).unwrap();
        assert_eq!(blob, key1);
    }

    #[test]
    fn filter_identities_blocks_all() {
        let response = build_identities_answer(&[(b"key1", b"c1"), (b"key2", b"c2")]);
        let filtered = filter_identities(&response, &["SHA256:nonexistent".to_string()]);

        assert_eq!(filtered[0], SSH_AGENT_IDENTITIES_ANSWER);
        let (nkeys, _) = read_u32(&filtered, 1).unwrap();
        assert_eq!(nkeys, 0);
    }

    #[test]
    fn filter_identities_empty_response() {
        let response = build_identities_answer(&[]);
        let filtered = filter_identities(&response, &["SHA256:any".to_string()]);

        assert_eq!(filtered[0], SSH_AGENT_IDENTITIES_ANSWER);
        let (nkeys, _) = read_u32(&filtered, 1).unwrap();
        assert_eq!(nkeys, 0);
    }

    #[test]
    fn filter_identities_preserves_comment() {
        let key = b"mykey";
        let comment = b"user@host";
        let fp = fingerprint(key);

        let response = build_identities_answer(&[(key, comment)]);
        let filtered = filter_identities(&response, &[fp]);

        let (_, offset) = read_u32(&filtered, 1).unwrap();
        let (_, offset) = read_string(&filtered, offset).unwrap();
        let (got_comment, _) = read_string(&filtered, offset).unwrap();
        assert_eq!(got_comment, comment);
    }

    #[test]
    fn filter_identities_non_answer_returns_failure() {
        let garbage = vec![99, 1, 2, 3];
        let filtered = filter_identities(&garbage, &[]);
        assert_eq!(filtered, vec![FAILURE_BYTE]);
    }

    #[test]
    fn is_sign_allowed_with_allowed_key() {
        let key = b"allowed-key";
        let fp = fingerprint(key);
        let msg = build_sign_request(key, b"data to sign");
        assert!(is_sign_allowed(&msg, &[fp]));
    }

    #[test]
    fn is_sign_allowed_with_blocked_key() {
        let key = b"blocked-key";
        let msg = build_sign_request(key, b"data to sign");
        assert!(!is_sign_allowed(&msg, &["SHA256:nonexistent".to_string()]));
    }

    #[test]
    fn is_sign_allowed_empty_message() {
        assert!(!is_sign_allowed(&[], &[]));
    }

    #[test]
    fn is_sign_allowed_wrong_message_type() {
        let key = b"some-key";
        let fp = fingerprint(key);
        let mut msg = build_sign_request(key, b"data");
        msg[0] = SSH_AGENTC_REQUEST_IDENTITIES;
        assert!(!is_sign_allowed(&msg, &[fp]));
    }
}
