// v1.1.0.7 Windows - Bind each legal send confirmation to one frozen scan and configuration.

pub const PREPARE_ACTION_LABEL: &str = "整理资料并继续";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendBinding {
    pub workspace: String,
    pub revision: u64,
    pub scan_fingerprint: String,
    pub configuration_fingerprint: String,
}

#[derive(Clone, Copy, Debug)]
pub struct Readiness {
    pub has_evidence: bool,
    pub configured: bool,
    pub snapshot_current: bool,
    pub workspace_ready: bool,
    pub preparing: bool,
    pub running: bool,
}

impl Readiness {
    pub fn can_prepare(self) -> bool {
        self.workspace_ready && !self.preparing && !self.running
    }

    pub fn can_send(self) -> bool {
        self.has_evidence
            && self.configured
            && self.snapshot_current
            && self.workspace_ready
            && !self.preparing
            && !self.running
    }
}

pub struct SendConsent {
    binding: SendBinding,
    consumed: bool,
}

impl SendConsent {
    pub fn new(binding: SendBinding) -> Self {
        Self {
            binding,
            consumed: false,
        }
    }

    pub fn matches(&self, current: &SendBinding) -> bool {
        !self.consumed && self.binding == *current
    }

    /// Consume before any worker or transport starts, including rejected attempts.
    pub fn consume(&mut self, current: &SendBinding, readiness: Readiness) -> bool {
        if self.consumed {
            return false;
        }
        self.consumed = true;
        readiness.can_send() && self.binding == *current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> SendBinding {
        SendBinding {
            workspace: "synthetic-workspace".into(),
            revision: 7,
            scan_fingerprint: "frozen-scan".into(),
            configuration_fingerprint: "endpoint-model-key-fingerprint".into(),
        }
    }

    fn ready() -> Readiness {
        Readiness {
            has_evidence: true,
            configured: true,
            snapshot_current: true,
            workspace_ready: true,
            preparing: false,
            running: false,
        }
    }

    #[test]
    fn authorization_is_consumed_once_even_if_send_is_clicked_again() {
        let current = binding();
        let mut consent = SendConsent::new(current.clone());
        assert!(consent.consume(&current, ready()));
        assert!(!consent.consume(&current, ready()));
        assert!(!consent.matches(&current));
    }

    #[test]
    fn changed_workspace_revision_scan_or_configuration_requires_new_confirmation() {
        let original = binding();
        for field in 0..4 {
            let mut changed = original.clone();
            match field {
                0 => changed.workspace.push_str("-other"),
                1 => changed.revision += 1,
                2 => changed.scan_fingerprint.push_str("-new"),
                _ => changed.configuration_fingerprint.push_str("-new"),
            }
            let mut consent = SendConsent::new(original.clone());
            assert!(!consent.matches(&changed));
            assert!(!consent.consume(&changed, ready()));
            assert!(!consent.consume(&original, ready()));
            let mut reconfirmed = SendConsent::new(changed.clone());
            assert!(reconfirmed.consume(&changed, ready()));
        }
    }

    #[test]
    fn empty_unconfigured_changed_and_blocked_workspaces_never_send() {
        let current = binding();
        for state in [
            Readiness {
                has_evidence: false,
                ..ready()
            },
            Readiness {
                configured: false,
                ..ready()
            },
            Readiness {
                snapshot_current: false,
                ..ready()
            },
            Readiness {
                workspace_ready: false,
                ..ready()
            },
        ] {
            let mut consent = SendConsent::new(current.clone());
            assert!(!consent.consume(&current, state));
            assert!(!consent.consume(&current, ready()));
        }
    }

    #[test]
    fn preparing_and_inflight_requests_do_not_accept_a_second_send() {
        let current = binding();
        for state in [
            Readiness {
                preparing: true,
                ..ready()
            },
            Readiness {
                running: true,
                ..ready()
            },
        ] {
            let mut consent = SendConsent::new(current.clone());
            assert!(!consent.consume(&current, state));
        }
    }

    #[test]
    fn preparing_and_inflight_requests_cannot_replace_the_cancellation_owner() {
        assert!(ready().can_prepare());
        for state in [
            Readiness {
                workspace_ready: false,
                ..ready()
            },
            Readiness {
                preparing: true,
                ..ready()
            },
            Readiness {
                running: true,
                ..ready()
            },
        ] {
            assert!(!state.can_prepare());
        }
    }
}
