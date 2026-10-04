// v1.1.0.7 Windows - Bind explicit AI consent to one immutable request and retain cancelled workers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QueryMode {
    #[default]
    Direct,
    Knowledge,
}

impl QueryMode {
    pub fn wire(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Knowledge => "knowledge",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryBinding {
    pub workspace: String,
    pub configuration: String,
    pub payload: String,
}

pub struct QueryConsent {
    binding: QueryBinding,
    consumed: bool,
}

impl QueryConsent {
    pub fn preview(binding: QueryBinding) -> Self {
        Self {
            binding,
            consumed: false,
        }
    }
}

#[derive(Default)]
pub struct QueryBoundary {
    next_id: u64,
    active: Option<(u64, QueryBinding)>,
    cancelled: bool,
}

impl QueryBoundary {
    pub fn busy(&self) -> bool {
        self.active.is_some()
    }

    pub fn begin(
        &mut self,
        consent: &mut QueryConsent,
        binding: &QueryBinding,
        ready: bool,
    ) -> Option<u64> {
        if self.busy() || !ready || consent.consumed || consent.binding != *binding {
            return None;
        }
        consent.consumed = true;
        self.next_id = self.next_id.saturating_add(1).max(1);
        self.active = Some((self.next_id, binding.clone()));
        self.cancelled = false;
        Some(self.next_id)
    }

    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    /// An old completion never frees another request. Cancellation keeps the
    /// worker owned until the matching completion or disconnect is received.
    pub fn finish(&mut self, id: u64, workspace: &str, configuration: &str) -> bool {
        let Some((active_id, binding)) = &self.active else {
            return false;
        };
        if *active_id != id {
            return false;
        }
        let accepted = !self.cancelled
            && binding.workspace == workspace
            && binding.configuration == configuration;
        self.active = None;
        accepted
    }

    pub fn active_id(&self) -> Option<u64> {
        self.active.as_ref().map(|(id, _)| *id)
    }

    pub fn active_scope_matches(&self, workspace: &str, configuration: &str) -> bool {
        self.active.as_ref().is_none_or(|(_, binding)| {
            binding.workspace == workspace && binding.configuration == configuration
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding() -> QueryBinding {
        QueryBinding {
            workspace: "workspace-a".into(),
            configuration: "configured-endpoint-model-key".into(),
            payload: "explicit-mode-question-sources".into(),
        }
    }

    #[test]
    fn authorization_is_bound_and_can_only_start_once() {
        let original = binding();
        let mut consent = QueryConsent::preview(original.clone());
        let mut boundary = QueryBoundary::default();
        for changed in [
            QueryBinding {
                workspace: "workspace-b".into(),
                ..original.clone()
            },
            QueryBinding {
                configuration: "different-key-or-service".into(),
                ..original.clone()
            },
            QueryBinding {
                payload: "different-question-or-sources".into(),
                ..original.clone()
            },
        ] {
            assert!(boundary.begin(&mut consent, &changed, true).is_none());
            assert!(!boundary.busy());
        }
        let id = boundary.begin(&mut consent, &original, true).unwrap();
        assert!(boundary.finish(id, &original.workspace, &original.configuration));
        assert!(boundary.begin(&mut consent, &original, true).is_none());
    }

    #[test]
    fn incomplete_or_unwritable_state_never_consumes_consent() {
        let original = binding();
        let mut boundary = QueryBoundary::default();
        let mut consent = QueryConsent::preview(original.clone());
        assert!(boundary.begin(&mut consent, &original, false).is_none());
        assert!(!boundary.busy());
        assert!(boundary.begin(&mut consent, &original, true).is_some());
    }

    #[test]
    fn cancelled_request_holds_ownership_until_worker_finishes() {
        let original = binding();
        let mut boundary = QueryBoundary::default();
        let id = boundary
            .begin(
                &mut QueryConsent::preview(original.clone()),
                &original,
                true,
            )
            .unwrap();
        boundary.cancel();
        assert!(boundary.busy());
        let mut next_consent = QueryConsent::preview(original.clone());
        assert!(boundary.begin(&mut next_consent, &original, true).is_none());
        assert!(!boundary.finish(id, &original.workspace, &original.configuration));
        assert!(!boundary.busy());
        let next = boundary.begin(&mut next_consent, &original, true).unwrap();
        assert!(!boundary.finish(id, &original.workspace, &original.configuration));
        assert_eq!(boundary.active_id(), Some(next));
    }

    #[test]
    fn changed_account_or_configuration_cannot_accept_completed_answer() {
        let original = binding();
        for (workspace, config) in [
            ("workspace-b", original.configuration.as_str()),
            (original.workspace.as_str(), "new-model"),
        ] {
            let mut boundary = QueryBoundary::default();
            let id = boundary
                .begin(
                    &mut QueryConsent::preview(original.clone()),
                    &original,
                    true,
                )
                .unwrap();
            assert!(!boundary.finish(id, workspace, config));
            assert!(!boundary.busy());
        }
    }
}
