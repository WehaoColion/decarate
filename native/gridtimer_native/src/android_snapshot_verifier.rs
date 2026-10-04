// v2.22.49 - Verify exact payload ranges in reusable Android read buffers.
use jni::objects::{JByteArray, JClass, JString, ReleaseMode};
use jni::sys::{jboolean, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use ring::digest::{Context, SHA256};
use serde::Deserialize;

#[derive(Deserialize)]
struct SchemaHeader {
    #[serde(rename = "schemaVersion")]
    version: Option<i64>,
}

fn hex_matches(digest: &[u8], expected: &str) -> bool {
    if expected.len() != digest.len() * 2 {
        return false;
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    digest
        .iter()
        .zip(expected.as_bytes().chunks_exact(2))
        .all(|(byte, pair)| {
            pair[0].to_ascii_lowercase() == HEX[(byte >> 4) as usize]
                && pair[1].to_ascii_lowercase() == HEX[(byte & 15) as usize]
        })
}

/// Android owns the only SQLite connection/transaction used for this read. The
/// prefix is the original envelope's small metadata part, including its final NUL.
/// Every payload and its envelope are verified, and the entire JSON is scanned for
/// the top-level version without materializing note/session objects.
pub fn verify_snapshot(
    raw: &[u8],
    prefix: &str,
    expected: &str,
    envelope: &str,
    supported: i32,
) -> bool {
    let fields: Vec<_> = prefix.split('\0').collect();
    if fields.len() != 9
        || fields[0] != "gridtimer-snapshot-envelope-v1"
        || !matches!(fields[1], "current_snapshot" | "snapshot_history")
        || !fields[8].is_empty()
    {
        return false;
    }
    let Ok(schema) = fields[4].parse::<i64>() else {
        return false;
    };
    if schema > i64::from(supported) {
        return false;
    }
    if !hex_matches(ring::digest::digest(&SHA256, raw).as_ref(), expected) {
        return false;
    }
    // Old history rows predate envelope seals. They are never selected as current
    // by this fast path: verify their payload and BOTH schema barriers, but do not
    // use their unsealed revision/item count as authority. An unsealed current or
    // a present-but-invalid envelope must still use the original recovery path.
    let legacy_history = envelope.is_empty() && fields[1] == "snapshot_history";
    if !legacy_history {
        let mut digest = Context::new(&SHA256);
        digest.update(prefix.as_bytes());
        digest.update(raw);
        if !hex_matches(digest.finish().as_ref(), envelope) {
            return false;
        }
    }
    let Ok(header) = serde_json::from_slice::<SchemaHeader>(raw) else {
        return false;
    };
    header.version.unwrap_or(i64::from(supported)) <= i64::from(supported)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_data_NativeSnapshotVerifier_nativeVerify(
    mut env: JNIEnv,
    _class: JClass,
    payload: JByteArray,
    length: i32,
    prefix: JString,
    expected: JString,
    envelope: JString,
    supported: i32,
) -> jboolean {
    let Ok(prefix) = env.get_string(&prefix).map(String::from) else {
        return JNI_FALSE;
    };
    let Ok(expected) = env.get_string(&expected).map(String::from) else {
        return JNI_FALSE;
    };
    let Ok(envelope) = env.get_string(&envelope).map(String::from) else {
        return JNI_FALSE;
    };
    // SAFETY: the caller exclusively owns this buffer until verification returns.
    // Its pool cannot recycle it while borrowed. The RAII guard releases/pins or
    // discards the VM copy before the buffer becomes reusable.
    let Ok(bytes) = (unsafe { env.get_array_elements(&payload, ReleaseMode::NoCopyBack) }) else {
        return JNI_FALSE;
    };
    let raw = unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<u8>(), bytes.len()) };
    if std::panic::catch_unwind(|| {
        verify_snapshot_range(raw, length, &prefix, &expected, &envelope, supported)
    })
    .unwrap_or(false)
    {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

fn verify_snapshot_range(
    buffer: &[u8],
    length: i32,
    prefix: &str,
    expected: &str,
    envelope: &str,
    supported: i32,
) -> bool {
    let Ok(length) = usize::try_from(length) else {
        return false;
    };
    buffer
        .get(..length)
        .is_some_and(|raw| verify_snapshot(raw, prefix, expected, envelope, supported))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
    fn seal(raw: &[u8], schema: i32) -> (String, String, String) {
        let prefix = format!(
            "gridtimer-snapshot-envelope-v1\0snapshot_history\x001\0a\0{schema}\x007\x002\x0099\0"
        );
        // Independent representation of the existing Android envelope protocol.
        let canonical = [prefix.as_bytes(), raw].concat();
        (
            prefix,
            hex(ring::digest::digest(&SHA256, raw).as_ref()),
            hex(ring::digest::digest(&SHA256, &canonical).as_ref()),
        )
    }

    #[test]
    fn reused_buffer_checks_exact_length_and_rejects_partial_or_invalid_ranges() {
        let raw = br#"{"schemaVersion":15,"notes":[]}"#;
        let (prefix, digest, envelope) = seal(raw, 15);
        let buffer = [raw.as_slice(), b"stale bytes from a longer snapshot"].concat();
        assert!(verify_snapshot_range(
            &buffer,
            raw.len() as i32,
            &prefix,
            &digest,
            &envelope,
            15
        ));
        for length in [
            -1,
            0,
            raw.len() as i32 - 1,
            buffer.len() as i32,
            buffer.len() as i32 + 1,
        ] {
            assert!(!verify_snapshot_range(
                &buffer, length, &prefix, &digest, &envelope, 15
            ));
        }
    }

    #[test]
    fn large_unicode_snapshot_keeps_exact_payload_and_envelope_protocol() {
        let raw = serde_json::json!({"notes":[{"content":"你好😀\u{0000}\\\"".repeat(20_000),"schemaVersion":999}],"schemaVersion":15}).to_string();
        let (prefix, digest, envelope) = seal(raw.as_bytes(), 15);
        assert!(verify_snapshot(
            raw.as_bytes(),
            &prefix,
            &digest,
            &envelope,
            15
        ));
        assert!(verify_snapshot(
            raw.as_bytes(),
            &prefix,
            &digest.to_uppercase(),
            &envelope.to_uppercase(),
            15
        ));
        let mut corrupt = raw.into_bytes();
        corrupt[40] ^= 1;
        assert!(!verify_snapshot(&corrupt, &prefix, &digest, &envelope, 15));
    }

    #[test]
    fn changed_payload_metadata_or_unsealed_envelope_rejects_the_receipt() {
        let raw = br#"{"schemaVersion":15,"notes":[]}"#;
        let (prefix, digest, envelope) = seal(raw, 15);
        for (from, to) in [
            ("snapshot_history", "current_snapshot"),
            ("\x001\0", "\x003\0"),
            ("\0a\0", "\0b\0"),
            ("\x0015\0", "\x0014\0"),
            ("\x007\0", "\x008\0"),
            ("\x002\0", "\x003\0"),
            ("\x0099\0", "\x00100\0"),
        ] {
            let changed = prefix.replace(from, to);
            assert_ne!(prefix, changed);
            assert!(!verify_snapshot(raw, &changed, &digest, &envelope, 15));
        }
        assert!(!verify_snapshot(raw, &prefix, "bad", &envelope, 15));
        assert!(!verify_snapshot(raw, &prefix, &digest, "bad", 15));
    }

    #[test]
    fn legacy_history_checks_payload_and_both_versions_but_cannot_seal_current() {
        let raw = br#"{"schemaVersion":15}"#;
        let (prefix, digest, _) = seal(raw, 15);
        assert!(verify_snapshot(raw, &prefix, &digest, "", 15));
        assert!(!verify_snapshot(raw, &prefix, "bad", "", 15));
        assert!(!verify_snapshot(
            raw,
            &prefix.replace("snapshot_history", "current_snapshot"),
            &digest,
            "",
            15
        ));
        let (future_column, digest, _) = seal(raw, 16);
        assert!(!verify_snapshot(raw, &future_column, &digest, "", 15));
        let future = br#"{"schemaVersion":16}"#;
        let (prefix, digest, _) = seal(future, 15);
        assert!(!verify_snapshot(future, &prefix, &digest, "", 15));
    }

    #[test]
    fn raw_future_schema_cannot_hide_behind_a_supported_sealed_column() {
        for (schema, raw) in [
            (15, r#"{"notes":[{"schemaVersion":15}],"schemaVersion":16}"#),
            (16, r#"{"schemaVersion":15}"#),
            (15, r#"{"schemaVersion":9223372036854775807}"#),
            (15, r#"{"schemaVersion":15,"schemaVersion":16}"#),
            (15, "malformed"),
            (15, r#"{"schemaVersion":15} trailing"#),
        ] {
            let (prefix, digest, envelope) = seal(raw.as_bytes(), schema);
            assert!(
                !verify_snapshot(raw.as_bytes(), &prefix, &digest, &envelope, 15),
                "{raw}"
            );
        }
    }

    #[test]
    fn legacy_defaults_and_nested_versions_do_not_imply_future_schema() {
        for raw in [
            "{}",
            r#"{"schemaVersion":null}"#,
            r#"{"notes":[{"schemaVersion":999}],"schemaVersion":15}"#,
        ] {
            let (prefix, digest, envelope) = seal(raw.as_bytes(), 15);
            assert!(verify_snapshot(
                raw.as_bytes(),
                &prefix,
                &digest,
                &envelope,
                15
            ));
        }
    }
}
