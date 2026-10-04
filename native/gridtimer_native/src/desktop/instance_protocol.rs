// v1.1.0.3 - Version-aware desktop activation only requests a safe handoff to a newer release.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenEffect {
    Activate,
    Upgrade,
}

#[derive(Clone, Debug)]
pub struct OpenReply {
    pub bytes: Vec<u8>,
    pub effect: OpenEffect,
    pub consumed: &'static [u8],
}

pub const LEGACY_REQUEST: &[u8] = b"GRIDTIMER_ACTIVATE_V1\n";
pub const LEGACY_ACK: &[u8] = b"OK_V1\n";
pub const LEGACY_CONSUMED: &[u8] = b"ACK_CONSUMED_V1\n";
pub const VERSION_CONSUMED: &[u8] = b"ACK_CONSUMED_V2\n";
pub const CANCEL_REQUEST: &[u8] = b"GRIDTIMER_CANCEL_OPEN_V2\n";
pub const CANCEL_ACK: &[u8] = b"OK_CANCEL_V2\n";
pub fn upgrade_request_is_fresh(elapsed: std::time::Duration, requester_running: bool) -> bool {
    requester_running && elapsed <= std::time::Duration::from_secs(40)
}

pub fn release_version(value: &str) -> Option<[u32; 4]> {
    let mut numbers = [0; 4];
    let components = value.split('.').collect::<Vec<_>>();
    if !(3..=4).contains(&components.len()) {
        return None;
    }
    for (index, component) in components.iter().enumerate() {
        if component.is_empty() || !component.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        numbers[index] = component.parse().ok()?;
    }
    Some(numbers)
}

pub fn version_request(version: &str) -> Option<Vec<u8>> {
    release_version(version)?;
    Some(format!("GRIDTIMER_OPEN_V2 {version}\n").into_bytes())
}

pub fn reply_to_open(message: &[u8], running: &str) -> Option<OpenReply> {
    if message == LEGACY_REQUEST {
        return Some(OpenReply {
            bytes: LEGACY_ACK.to_vec(),
            effect: OpenEffect::Activate,
            consumed: LEGACY_CONSUMED,
        });
    }
    let requested = std::str::from_utf8(message)
        .ok()?
        .strip_prefix("GRIDTIMER_OPEN_V2 ")?
        .strip_suffix('\n')?;
    let effect = if release_version(requested)? > release_version(running)? {
        OpenEffect::Upgrade
    } else {
        OpenEffect::Activate
    };
    let action = match effect {
        OpenEffect::Activate => "ACTIVATE",
        OpenEffect::Upgrade => "UPGRADE",
    };
    Some(OpenReply {
        bytes: format!("GRIDTIMER_RUNNING_V2 {running} {action}\n").into_bytes(),
        effect,
        consumed: VERSION_CONSUMED,
    })
}

pub fn committed_open_effect(reply: &OpenReply, consumed: &[u8]) -> Option<OpenEffect> {
    (consumed == reply.consumed).then_some(reply.effect)
}

pub fn parse_running_reply(reply: &[u8], requested: &str) -> Option<(String, OpenEffect)> {
    let line = std::str::from_utf8(reply)
        .ok()?
        .strip_prefix("GRIDTIMER_RUNNING_V2 ")?
        .strip_suffix('\n')?;
    let (running, action) = line.split_once(' ')?;
    let expected = if release_version(requested)? > release_version(running)? {
        OpenEffect::Upgrade
    } else {
        OpenEffect::Activate
    };
    let reported = match action {
        "ACTIVATE" => OpenEffect::Activate,
        "UPGRADE" => OpenEffect::Upgrade,
        _ => return None,
    };
    (expected == reported).then(|| (running.to_string(), reported))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_strictly_newer_release_can_request_safe_upgrade() {
        for wanted in ["1.1.0.2", "1.1.0.3", "1.1.0"] {
            let reply = reply_to_open(&version_request(wanted).unwrap(), "1.1.0.3").unwrap();
            assert_eq!(reply.effect, OpenEffect::Activate);
            assert_eq!(
                parse_running_reply(&reply.bytes, wanted).unwrap().1,
                OpenEffect::Activate
            );
        }
        for wanted in ["1.1.0.4", "1.1.0.10", "1.2.0.0"] {
            let reply = reply_to_open(&version_request(wanted).unwrap(), "1.1.0.3").unwrap();
            assert_eq!(reply.effect, OpenEffect::Upgrade);
            assert_eq!(
                parse_running_reply(&reply.bytes, wanted).unwrap().1,
                OpenEffect::Upgrade
            );
        }
    }
    #[test]
    fn unacknowledged_or_wrong_protocol_request_cannot_close_running_client() {
        let reply = reply_to_open(&version_request("1.1.0.4").unwrap(), "1.1.0.3").unwrap();
        for consumed in [&b""[..], LEGACY_CONSUMED, &b"ACK_CONSUMED_V2\nextra"[..]] {
            assert_eq!(committed_open_effect(&reply, consumed), None);
        }
        assert_eq!(
            committed_open_effect(&reply, VERSION_CONSUMED),
            Some(OpenEffect::Upgrade)
        );
    }
    #[test]
    fn expired_or_abandoned_request_cannot_start_a_late_shutdown() {
        assert!(upgrade_request_is_fresh(
            std::time::Duration::from_secs(1),
            true
        ));
        assert!(!upgrade_request_is_fresh(
            std::time::Duration::from_secs(41),
            true
        ));
        assert!(!upgrade_request_is_fresh(
            std::time::Duration::from_secs(1),
            false
        ));
    }
    #[test]
    fn legacy_activation_remains_activation_and_malformed_versions_are_refused() {
        let legacy = reply_to_open(LEGACY_REQUEST, "1.1.0.3").unwrap();
        assert_eq!(legacy.effect, OpenEffect::Activate);
        assert_eq!(legacy.bytes, LEGACY_ACK);
        for version in [
            "",
            "1.1",
            "1.1.0.3.1",
            "1.1..3",
            "1.1.0.3\n",
            "1.1.0.-1",
            "1.1.0.99999999999999999",
        ] {
            assert!(version_request(version).is_none());
        }
        assert!(reply_to_open(b"GRIDTIMER_OPEN_V2 1.1.0.4\nextra", "1.1.0.3").is_none());
        assert!(
            parse_running_reply(b"GRIDTIMER_RUNNING_V2 1.1.0.3 UPGRADE\n", "1.1.0.3").is_none()
        );
    }
}
