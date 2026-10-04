// v2.22.49.8 - Authenticate compact history bytes without reloading full JSON at launch.
use crate::android_snapshot_verifier::verify_snapshot;
use base64::{engine::general_purpose::STANDARD, Engine};
use flate2::{write::ZlibEncoder, Compression, Decompress, FlushDecompress, Status};
use jni::objects::{JByteArray, JClass, JString, ReleaseMode};
use jni::sys::{jboolean, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use ring::hmac;
use std::io::Write;

const MARKER: &[u8] = b"!gridtimer-history-v1:";
const DOMAIN: &[u8] = b"gridtimer-history-authenticated-content-v1\0";
const MAX_RAW: usize = 128 * 1024 * 1024;
const MAX_COMPRESSED: usize = MAX_RAW + 1024 * 1024;
const MAX_PREFIX: usize = 16 * 1024;
const HEADER_LEN: usize = 20;
const TAG_LEN: usize = 32;
const MAX_BINARY: usize = HEADER_LEN + MAX_PREFIX + 128 + MAX_COMPRESSED + TAG_LEN;
const MAX_ENCODED: usize = MAX_BINARY.div_ceil(3) * 4;

pub fn is_packed(raw: &[u8]) -> bool {
    raw.starts_with(MARKER)
}

fn history_prefix(prefix: &str, supported: i32) -> bool {
    let fields: Vec<_> = prefix.split('\0').collect();
    prefix.len() <= MAX_PREFIX
        && supported >= 0
        && fields.len() == 9
        && fields[0] == "gridtimer-snapshot-envelope-v1"
        && fields[1] == "snapshot_history"
        && fields[8].is_empty()
        && fields[4]
            .parse::<i64>()
            .is_ok_and(|schema| schema <= i64::from(supported))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.as_bytes().iter().all(u8::is_ascii_hexdigit)
}

fn tag(bytes: &[u8], key: &[u8]) -> hmac::Tag {
    let mut context = hmac::Context::with_key(&hmac::Key::new(hmac::HMAC_SHA256, key));
    context.update(DOMAIN);
    context.update(bytes);
    context.sign()
}

struct Packed {
    bytes: Vec<u8>,
    raw_len: usize,
    sealed_supported: i32,
    prefix_end: usize,
    digest_end: usize,
    envelope_end: usize,
    payload_end: usize,
}

impl Packed {
    fn parse(encoded: &[u8]) -> Option<Self> {
        let encoded = encoded.strip_prefix(MARKER)?;
        if encoded.len() > MAX_ENCODED {
            return None;
        }
        let bytes = STANDARD.decode(encoded).ok()?;
        if bytes.len() < HEADER_LEN + TAG_LEN || bytes.len() > MAX_BINARY {
            return None;
        }
        let raw_len = u32::from_le_bytes(bytes[0..4].try_into().ok()?) as usize;
        let sealed_supported = i32::from_le_bytes(bytes[4..8].try_into().ok()?);
        let prefix_len = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
        let digest_len = u16::from_le_bytes(bytes[12..14].try_into().ok()?) as usize;
        let envelope_len = u16::from_le_bytes(bytes[14..16].try_into().ok()?) as usize;
        let payload_len = u32::from_le_bytes(bytes[16..20].try_into().ok()?) as usize;
        if raw_len == 0
            || raw_len > MAX_RAW
            || sealed_supported < 0
            || prefix_len == 0
            || prefix_len > MAX_PREFIX
            || digest_len != 64
            || !matches!(envelope_len, 0 | 64)
            || payload_len == 0
            || payload_len > MAX_COMPRESSED
        {
            return None;
        }
        let prefix_end = HEADER_LEN + prefix_len;
        let digest_end = prefix_end + digest_len;
        let envelope_end = digest_end + envelope_len;
        let payload_end = envelope_end + payload_len;
        if payload_end + TAG_LEN != bytes.len() {
            return None;
        }
        let packed = Self {
            bytes,
            raw_len,
            sealed_supported,
            prefix_end,
            digest_end,
            envelope_end,
            payload_end,
        };
        if !history_prefix(packed.prefix()?, sealed_supported)
            || !valid_digest(packed.expected()?)
            || !(packed.envelope()?.is_empty() || valid_digest(packed.envelope()?))
        {
            return None;
        }
        Some(packed)
    }

    fn prefix(&self) -> Option<&str> {
        std::str::from_utf8(&self.bytes[HEADER_LEN..self.prefix_end]).ok()
    }

    fn expected(&self) -> Option<&str> {
        std::str::from_utf8(&self.bytes[self.prefix_end..self.digest_end]).ok()
    }

    fn envelope(&self) -> Option<&str> {
        std::str::from_utf8(&self.bytes[self.digest_end..self.envelope_end]).ok()
    }

    fn authenticated(&self, key: &[u8]) -> bool {
        if key.len() != 32 {
            return false;
        }
        // ring performs the authentication comparison in constant time. The domain
        // is part of the authenticated message, including on the verification side.
        let mut message = Vec::with_capacity(DOMAIN.len() + self.payload_end);
        message.extend_from_slice(DOMAIN);
        message.extend_from_slice(&self.bytes[..self.payload_end]);
        hmac::verify(
            &hmac::Key::new(hmac::HMAC_SHA256, key),
            &message,
            &self.bytes[self.payload_end..],
        )
        .is_ok()
    }

    fn decode_verified(&self, supported: i32) -> Option<String> {
        if self.sealed_supported > supported || supported < 0 {
            return None;
        }
        let payload = &self.bytes[self.envelope_end..self.payload_end];
        let mut decoder = Decompress::new(true);
        let mut raw = vec![0; self.raw_len + 1];
        let status = decoder
            .decompress(payload, &mut raw, FlushDecompress::Finish)
            .ok()?;
        if status != Status::StreamEnd
            || decoder.total_out() != self.raw_len as u64
            || decoder.total_in() != payload.len() as u64
        {
            return None;
        }
        raw.truncate(self.raw_len);
        if !verify_snapshot(
            &raw,
            self.prefix()?,
            self.expected()?,
            self.envelope()?,
            supported,
        ) {
            return None;
        }
        String::from_utf8(raw).ok()
    }
}

/// Only verified history is packed. Current rows remain plain JSON. A successful
/// seal binds the exact original bytes, all row metadata and schema support floor.
pub fn seal(
    raw: &[u8],
    prefix: &str,
    expected: &str,
    envelope: &str,
    supported: i32,
    key: &[u8],
) -> Option<String> {
    if key.len() != 32
        || raw.is_empty()
        || raw.len() > MAX_RAW
        || !history_prefix(prefix, supported)
        || !valid_digest(expected)
        || !(envelope.is_empty() || valid_digest(envelope))
        || !verify_snapshot(raw, prefix, expected, envelope, supported)
    {
        return None;
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(raw).ok()?;
    let compressed = encoder.finish().ok()?;
    if compressed.len() > MAX_COMPRESSED {
        return None;
    }
    let mut bytes = Vec::with_capacity(
        HEADER_LEN + prefix.len() + expected.len() + envelope.len() + compressed.len() + TAG_LEN,
    );
    bytes.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&supported.to_le_bytes());
    bytes.extend_from_slice(&(prefix.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(expected.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&(envelope.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
    bytes.extend_from_slice(prefix.as_bytes());
    bytes.extend_from_slice(expected.as_bytes());
    bytes.extend_from_slice(envelope.as_bytes());
    bytes.extend_from_slice(&compressed);
    bytes.extend_from_slice(tag(&bytes, key).as_ref());
    let encoded = format!(
        "{}{}",
        std::str::from_utf8(MARKER).ok()?,
        STANDARD.encode(bytes)
    );
    // Never issue a reusable receipt until the codec round trip has been checked.
    let packed = Packed::parse(encoded.as_bytes())?;
    let restored = packed.decode_verified(supported)?;
    (restored.as_bytes() == raw).then_some(encoded)
}

/// Every startup authenticates actual compressed bytes. A missing key merely
/// disables this shortcut; it never strands a snapshot that passes original checks.
pub fn verify(
    packed: &[u8],
    prefix: &str,
    expected: &str,
    envelope: &str,
    supported: i32,
    key: &[u8],
) -> bool {
    let Some(packed) = Packed::parse(packed) else {
        return false;
    };
    if packed.sealed_supported > supported
        || !history_prefix(prefix, supported)
        || packed.prefix() != Some(prefix)
        || packed.expected() != Some(expected)
        || packed.envelope() != Some(envelope)
    {
        return false;
    }
    packed.authenticated(key) || packed.decode_verified(supported).is_some()
}

/// Recovery always validates the decompressed original, even with a valid receipt.
/// This preserves original payload/envelope checks and independently checks codec IO.
pub fn decode(packed: &str, _key: &[u8], supported: i32) -> Option<String> {
    Packed::parse(packed.as_bytes())?.decode_verified(supported)
}

fn verify_range(
    buffer: &[u8],
    length: i32,
    prefix: &str,
    expected: &str,
    envelope: &str,
    supported: i32,
    key: &[u8],
) -> bool {
    usize::try_from(length)
        .ok()
        .and_then(|length| buffer.get(..length))
        .is_some_and(|packed| verify(packed, prefix, expected, envelope, supported, key))
}

fn clear_jni_exception(env: &mut JNIEnv<'_>) {
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_data_NativeSnapshotVerifier_nativeSeal(
    mut env: JNIEnv,
    _class: JClass,
    raw: JString,
    prefix: JString,
    expected: JString,
    envelope: JString,
    supported: i32,
    key: JByteArray,
) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let raw = env.get_string(&raw).map(String::from).ok()?;
        let prefix = env.get_string(&prefix).map(String::from).ok()?;
        let expected = env.get_string(&expected).map(String::from).ok()?;
        let envelope = env.get_string(&envelope).map(String::from).ok()?;
        let key = env.convert_byte_array(&key).ok()?;
        let packed = seal(
            raw.as_bytes(),
            &prefix,
            &expected,
            &envelope,
            supported,
            &key,
        )?;
        env.new_string(packed).ok().map(JString::into_raw)
    }))
    .ok()
    .flatten()
    .unwrap_or(std::ptr::null_mut());
    clear_jni_exception(&mut env);
    result
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_data_NativeSnapshotVerifier_nativeDecode(
    mut env: JNIEnv,
    _class: JClass,
    packed: JString,
    supported: i32,
    key: JByteArray,
) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let packed = env.get_string(&packed).map(String::from).ok()?;
        let key = env.convert_byte_array(&key).ok()?;
        env.new_string(decode(&packed, &key, supported)?)
            .ok()
            .map(JString::into_raw)
    }))
    .ok()
    .flatten()
    .unwrap_or(std::ptr::null_mut());
    clear_jni_exception(&mut env);
    result
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_data_NativeSnapshotVerifier_nativeVerifyPacked(
    mut env: JNIEnv,
    _class: JClass,
    payload: JByteArray,
    length: i32,
    prefix: JString,
    expected: JString,
    envelope: JString,
    supported: i32,
    key: JByteArray,
) -> jboolean {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let prefix = env.get_string(&prefix).map(String::from).ok()?;
        let expected = env.get_string(&expected).map(String::from).ok()?;
        let envelope = env.get_string(&envelope).map(String::from).ok()?;
        let key = env.convert_byte_array(&key).ok()?;
        // SAFETY: callers keep this buffer exclusively owned until return. The
        // array guard releases the VM copy before their pool can recycle it.
        let bytes = unsafe { env.get_array_elements(&payload, ReleaseMode::NoCopyBack) }.ok()?;
        let buffer = unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast(), bytes.len()) };
        Some(verify_range(
            buffer, length, &prefix, &expected, &envelope, supported, &key,
        ))
    }))
    .ok()
    .flatten()
    .unwrap_or(false);
    clear_jni_exception(&mut env);
    if result {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::digest::{digest, SHA256};

    const KEY: [u8; 32] = [7; 32];

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn metadata(raw: &[u8], schema: i32) -> (String, String, String) {
        let prefix = format!(
            "gridtimer-snapshot-envelope-v1\0snapshot_history\x001\0owner\0{schema}\x007\x002\x0099\0"
        );
        let expected = hex(digest(&SHA256, raw).as_ref());
        let envelope = hex(digest(&SHA256, &[prefix.as_bytes(), raw].concat()).as_ref());
        (prefix, expected, envelope)
    }

    fn encoded(bytes: &[u8]) -> String {
        format!(
            "{}{}",
            std::str::from_utf8(MARKER).unwrap(),
            STANDARD.encode(bytes)
        )
    }

    fn fixture(raw: &[u8]) -> (String, String, String, String) {
        let (prefix, expected, envelope) = metadata(raw, 15);
        let packed = seal(raw, &prefix, &expected, &envelope, 15, &KEY).unwrap();
        (packed, prefix, expected, envelope)
    }

    #[test]
    fn packed_history_roundtrips_unicode_and_recovers_when_key_is_lost() {
        let raw = serde_json::json!({"schemaVersion":15,"notes":[{"content":"你好😀\u{0000}".repeat(4096)}]}).to_string();
        let (packed, prefix, expected, envelope) = fixture(raw.as_bytes());
        assert!(is_packed(packed.as_bytes()));
        assert!(packed.len() < raw.len() / 4);
        for key in [KEY.as_slice(), [].as_slice(), [9; 32].as_slice()] {
            assert!(verify(
                packed.as_bytes(),
                &prefix,
                &expected,
                &envelope,
                15,
                key
            ));
            assert_eq!(decode(&packed, key, 15).as_deref(), Some(raw.as_str()));
        }
        assert!(seal(raw.as_bytes(), &prefix, &expected, &envelope, 15, &[]).is_none());
    }

    #[test]
    fn changed_row_identity_or_metadata_never_uses_a_content_receipt() {
        let (packed, prefix, expected, envelope) = fixture(br#"{"schemaVersion":15}"#);
        for (old, new) in [
            ("snapshot_history", "current_snapshot"),
            ("\x001\0", "\x003\0"),
            ("owner", "other"),
            ("\x0015\0", "\x0014\0"),
            ("\x007\0", "\x008\0"),
            ("\x002\0", "\x003\0"),
            ("\x0099\0", "\x00100\0"),
        ] {
            assert!(!verify(
                packed.as_bytes(),
                &prefix.replace(old, new),
                &expected,
                &envelope,
                15,
                &KEY
            ));
        }
        assert!(!verify(
            packed.as_bytes(),
            &prefix,
            &"0".repeat(64),
            &envelope,
            15,
            &KEY
        ));
        assert!(!verify(
            packed.as_bytes(),
            &prefix,
            &expected,
            &"0".repeat(64),
            15,
            &KEY
        ));
        let raw = br#"{"schemaVersion":15}"#;
        assert!(seal(
            raw,
            &prefix.replace("snapshot_history", "current_snapshot"),
            &expected,
            &envelope,
            15,
            &KEY
        )
        .is_none());
    }

    #[test]
    fn future_schema_and_support_floor_are_preserved_for_legacy_defaults() {
        for raw in [
            b"{}".as_slice(),
            br#"{"schemaVersion":null}"#,
            br#"{"schemaVersion":15}"#,
        ] {
            let (packed, prefix, expected, envelope) = fixture(raw);
            for key in [KEY.as_slice(), [].as_slice()] {
                assert!(!verify(
                    packed.as_bytes(),
                    &prefix,
                    &expected,
                    &envelope,
                    14,
                    key
                ));
                assert!(decode(&packed, key, 14).is_none());
                assert!(verify(
                    packed.as_bytes(),
                    &prefix,
                    &expected,
                    &envelope,
                    16,
                    key
                ));
            }
        }
        for (raw, declared) in [
            (r#"{"schemaVersion":16}"#, 15),
            (r#"{"schemaVersion":15}"#, 16),
            (r#"{"schemaVersion":15,"schemaVersion":16}"#, 15),
            (r#"{"schemaVersion":9223372036854775807}"#, 15),
            ("malformed", 15),
        ] {
            let (prefix, expected, envelope) = metadata(raw.as_bytes(), declared);
            assert!(seal(raw.as_bytes(), &prefix, &expected, &envelope, 15, &KEY).is_none());
        }
    }

    #[test]
    fn bad_payloads_truncation_and_unbounded_lengths_are_rejected() {
        let (packed, prefix, expected, envelope) = fixture(br#"{"schemaVersion":15,"notes":[]}"#);
        let original = Packed::parse(packed.as_bytes()).unwrap();
        for offset in [
            0,
            4,
            8,
            12,
            14,
            16,
            HEADER_LEN,
            original.prefix_end,
            original.digest_end,
            original.envelope_end,
            original.payload_end - 1,
        ] {
            let mut bytes = original.bytes.clone();
            bytes[offset] ^= 1;
            let changed = encoded(&bytes);
            assert!(
                !verify(changed.as_bytes(), &prefix, &expected, &envelope, 15, &KEY),
                "offset {offset}"
            );
            assert!(decode(&changed, &[], 15).is_none(), "offset {offset}");
        }
        for remove in [1, TAG_LEN, TAG_LEN + 1] {
            assert!(decode(
                &encoded(&original.bytes[..original.bytes.len() - remove]),
                &KEY,
                15
            )
            .is_none());
        }
        let mut bytes = original.bytes.clone();
        bytes[..4].copy_from_slice(&((MAX_RAW + 1) as u32).to_le_bytes());
        assert!(decode(&encoded(&bytes), &KEY, 15).is_none());
        bytes.extend_from_slice(b"trailing");
        assert!(decode(&encoded(&bytes), &KEY, 15).is_none());
    }

    #[test]
    fn tampered_authentication_tag_only_falls_back_to_full_original_validation() {
        let (packed, prefix, expected, envelope) = fixture(br#"{"schemaVersion":15}"#);
        let original = Packed::parse(packed.as_bytes()).unwrap();
        let mut bytes = original.bytes.clone();
        *bytes.last_mut().unwrap() ^= 1;
        let changed = encoded(&bytes);
        assert!(!Packed::parse(changed.as_bytes())
            .unwrap()
            .authenticated(&KEY));
        assert!(verify(
            changed.as_bytes(),
            &prefix,
            &expected,
            &envelope,
            15,
            &KEY
        ));
        bytes[original.envelope_end] ^= 1;
        let changed = encoded(&bytes);
        assert!(!verify(
            changed.as_bytes(),
            &prefix,
            &expected,
            &envelope,
            15,
            &KEY
        ));
    }

    #[test]
    fn compressed_trailing_stream_and_declared_zipbomb_size_fail_even_without_key() {
        let (packed, prefix, expected, envelope) = fixture(br#"{"schemaVersion":15}"#);
        let original = Packed::parse(packed.as_bytes()).unwrap();
        let mut bytes = original.bytes.clone();
        bytes.splice(original.payload_end..original.payload_end, [1, 2, 3]);
        let payload_len = (original.payload_end - original.envelope_end + 3) as u32;
        bytes[16..20].copy_from_slice(&payload_len.to_le_bytes());
        assert!(!verify(
            encoded(&bytes).as_bytes(),
            &prefix,
            &expected,
            &envelope,
            15,
            &[]
        ));
        let mut bytes = original.bytes.clone();
        bytes[..4].copy_from_slice(&1u32.to_le_bytes());
        assert!(decode(&encoded(&bytes), &[], 15).is_none());
        // A missing zlib checksum can still yield all original JSON bytes. It
        // remains an incomplete compressed stream and cannot become a receipt.
        let mut bytes = original.bytes.clone();
        bytes.remove(original.payload_end - 1);
        let shortened_len = (original.payload_end - original.envelope_end - 1) as u32;
        bytes[16..20].copy_from_slice(&shortened_len.to_le_bytes());
        assert!(decode(&encoded(&bytes), &[], 15).is_none());
    }

    #[test]
    fn reused_buffers_validate_only_the_requested_range() {
        let (packed, prefix, expected, envelope) = fixture(br#"{"schemaVersion":15}"#);
        let buffer = [packed.as_bytes(), b"previous larger payload"].concat();
        assert!(verify_range(
            &buffer,
            packed.len() as i32,
            &prefix,
            &expected,
            &envelope,
            15,
            &KEY
        ));
        for length in [
            -1,
            0,
            packed.len() as i32 - 1,
            buffer.len() as i32,
            buffer.len() as i32 + 1,
        ] {
            assert!(!verify_range(
                &buffer, length, &prefix, &expected, &envelope, 15, &KEY
            ));
        }
    }

    #[test]
    fn legacy_history_without_envelope_still_requires_exact_payload_digest() {
        let raw = br#"{"schemaVersion":15}"#;
        let (prefix, expected, _) = metadata(raw, 15);
        let packed = seal(raw, &prefix, &expected, "", 15, &KEY).unwrap();
        assert!(verify(packed.as_bytes(), &prefix, &expected, "", 15, &KEY));
        assert_eq!(
            decode(&packed, &[], 15).as_deref(),
            std::str::from_utf8(raw).ok()
        );
        assert!(seal(raw, &prefix, &"0".repeat(64), "", 15, &KEY).is_none());
    }
}
