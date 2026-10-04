// v2.22.35 - Share timer ordering, archive eligibility, and Android-compatible history summaries.
//! Windows-facing timer projection facade.
//!
//! The caller supplies one wall-clock sample per frame or refresh. The
//! projection is read-only: persist the original app-data document through the
//! normal mutation API, never serialize a `TimerProjection` back as app data.
//!
//! ```ignore
//! use gridtimer_native::desktop_timer::{project_timer_views, TimerViewPhase};
//!
//! let now_epoch_millis = system_time_epoch_millis();
//! let projection = project_timer_views(&app_data_json, now_epoch_millis)?;
//! for slot in projection.slots {
//!     draw_total(slot.accumulated_millis);
//!     draw_phase(
//!         slot.micro_break_phase,
//!         slot.micro_break_phase_progress_millis,
//!         slot.micro_break_phase_target_millis,
//!         slot.micro_break_phase_remaining_millis,
//!     );
//!     if slot.micro_break_phase == TimerViewPhase::Unknown {
//!         show_upgrade_required(&slot.source_micro_break_phase);
//!     }
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub use crate::app_data::{
    project_timer_views, project_timer_views_json, TimerProjection, TimerProjectionError,
    TimerProjector, TimerView, TimerViewPhase, MAX_MICRO_BREAK_DETAIL_SESSIONS,
};

use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};

const MAX_TRACKED_DURATION_MILLIS: i64 = 10 * 365 * 24 * 60 * 60 * 1_000;

/// Preserve the synchronized order, then append any newly introduced slots.
/// Unknown order entries and repeated ids never produce duplicate cards.
pub fn ordered_slot_ids(slot_ids: &[i32], slot_order: &[i32]) -> Vec<i32> {
    let available = slot_ids.iter().copied().collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    slot_order
        .iter()
        .chain(slot_ids)
        .copied()
        .filter(|id| available.contains(id) && seen.insert(*id))
        .collect()
}

pub fn total_accumulated_millis(projection: &TimerProjection) -> i64 {
    projection.slots.iter().fold(0_i64, |total, slot| {
        total.saturating_add(safe_duration(slot.accumulated_millis))
    })
}

/// Same blank-slot rule used by the Android/shared archive mutation.
pub fn is_blank_timer(slot: &TimerView) -> bool {
    slot.title.trim().is_empty()
        && slot.category_id.is_none()
        && slot.note.trim().is_empty()
        && slot.accumulated_millis == 0
        && !slot.is_running
        && slot.micro_break_phase == TimerViewPhase::Focus
        && slot.micro_break_cycle_index == 0
        && slot.micro_break_phase_progress_millis == 0
}

/// Matches the shared restore mutation: prefer the original empty slot and
/// otherwise choose the first empty slot in the source document's slot order.
pub fn restore_target_slot_id(projection: &TimerProjection, original_slot_id: i32) -> Option<i32> {
    projection
        .slots
        .iter()
        .find(|slot| slot.id == original_slot_id && is_blank_timer(slot))
        .or_else(|| projection.slots.iter().find(|slot| is_blank_timer(slot)))
        .map(|slot| slot.id)
}

/// Read-only records needed for the history UI; note contents are not parsed.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TimerHistorySession {
    pub id: String,
    pub slot_id: i32,
    pub slot_title: String,
    pub category_id: Option<String>,
    pub started_at_epoch_millis: i64,
    pub ended_at_epoch_millis: i64,
    pub duration_millis: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TimerHistoryArchive {
    pub id: String,
    pub original_slot_id: i32,
    pub title: String,
    pub category_id: Option<String>,
    pub note: String,
    pub accumulated_millis: i64,
    pub archived_at_epoch_millis: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct HistoryCategory {
    id: String,
    name: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct HistorySource {
    categories: Vec<HistoryCategory>,
    sessions: Vec<TimerHistorySession>,
    archived_tasks: Vec<TimerHistoryArchive>,
}

/// `None` includes every category. `Some("")` selects uncategorized records.
#[derive(Clone, Copy, Debug, Default)]
pub struct TimerHistoryFilter<'a> {
    pub query: &'a str,
    pub category_id: Option<&'a str>,
    pub slot_id: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TimerCategoryDuration {
    pub category_id: Option<String>,
    pub category_name: String,
    pub duration_millis: i64,
    pub session_count: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TimerHistorySummary {
    /// Most recent first. Ids refer to records in the original app document.
    pub session_ids: Vec<String>,
    pub archived_task_ids: Vec<String>,
    pub total_millis: i64,
    pub today_millis: i64,
    pub week_millis: i64,
    pub durations_by_category: Vec<TimerCategoryDuration>,
}

/// Cache this index when the app-data snapshot changes, rather than parsing
/// the complete document on every repaint. Date windows come from the UI's
/// local time zone, so DST days may be shorter or longer than 24 hours.
#[derive(Clone, Debug, Default)]
pub struct TimerHistoryIndex {
    pub sessions: Vec<TimerHistorySession>,
    pub archived_tasks: Vec<TimerHistoryArchive>,
    category_names: HashMap<String, String>,
    session_haystacks: Vec<String>,
    archive_haystacks: Vec<String>,
}

impl TimerHistoryIndex {
    pub fn parse(raw: &str) -> Result<Self, serde_json::Error> {
        let source: HistorySource = serde_json::from_str(raw)?;
        let category_names = source
            .categories
            .into_iter()
            .map(|category| (category.id, category.name))
            .collect::<HashMap<_, _>>();
        let mut sessions = source.sessions;
        let mut archived_tasks = source.archived_tasks;
        sessions.sort_by(|left, right| {
            right
                .ended_at_epoch_millis
                .cmp(&left.ended_at_epoch_millis)
                .then_with(|| {
                    right
                        .started_at_epoch_millis
                        .cmp(&left.started_at_epoch_millis)
                })
                .then_with(|| left.id.cmp(&right.id))
        });
        archived_tasks.sort_by(|left, right| {
            right
                .archived_at_epoch_millis
                .cmp(&left.archived_at_epoch_millis)
                .then_with(|| left.id.cmp(&right.id))
        });
        let session_haystacks = sessions
            .iter()
            .map(|session| {
                history_haystack(
                    &session.slot_title,
                    "",
                    category_name(&category_names, session.category_id.as_deref()),
                    session.slot_id,
                )
            })
            .collect();
        let archive_haystacks = archived_tasks
            .iter()
            .map(|task| {
                history_haystack(
                    &task.title,
                    &task.note,
                    category_name(&category_names, task.category_id.as_deref()),
                    task.original_slot_id,
                )
            })
            .collect();
        Ok(Self {
            sessions,
            archived_tasks,
            category_names,
            session_haystacks,
            archive_haystacks,
        })
    }

    /// Android assigns the complete recorded duration to its ending instant.
    /// A session ending at the next midnight belongs to the next day; it is
    /// never counted in both days. Archives are not added to session totals.
    /// Invalid or empty time windows have zero duration, but do not hide rows.
    pub fn summarize(
        &self,
        filter: &TimerHistoryFilter<'_>,
        today_window: (i64, i64),
        week_window: (i64, i64),
    ) -> TimerHistorySummary {
        let keywords = filter
            .query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>();
        let mut summary = TimerHistorySummary::default();
        let mut category_totals = BTreeMap::<Option<String>, TimerCategoryDuration>::new();
        for (session, haystack) in self.sessions.iter().zip(&self.session_haystacks) {
            if !matches_filter(filter, session.category_id.as_deref(), session.slot_id)
                || !keywords.iter().all(|keyword| haystack.contains(keyword))
            {
                continue;
            }
            summary.session_ids.push(session.id.clone());
            let duration = safe_duration(session.duration_millis);
            summary.total_millis = summary.total_millis.saturating_add(duration);
            if in_window(session.ended_at_epoch_millis, today_window) {
                summary.today_millis = summary.today_millis.saturating_add(duration);
            }
            if in_window(session.ended_at_epoch_millis, week_window) {
                summary.week_millis = summary.week_millis.saturating_add(duration);
            }
            let category = category_totals
                .entry(session.category_id.clone())
                .or_insert_with(|| TimerCategoryDuration {
                    category_id: session.category_id.clone(),
                    category_name: category_name(
                        &self.category_names,
                        session.category_id.as_deref(),
                    )
                    .to_string(),
                    ..Default::default()
                });
            category.duration_millis = category.duration_millis.saturating_add(duration);
            category.session_count = category.session_count.saturating_add(1);
        }
        for (task, haystack) in self.archived_tasks.iter().zip(&self.archive_haystacks) {
            if matches_filter(filter, task.category_id.as_deref(), task.original_slot_id)
                && keywords.iter().all(|keyword| haystack.contains(keyword))
            {
                summary.archived_task_ids.push(task.id.clone());
            }
        }
        summary.durations_by_category = category_totals.into_values().collect();
        summary.durations_by_category.sort_by(|left, right| {
            right
                .duration_millis
                .cmp(&left.duration_millis)
                .then_with(|| left.category_name.cmp(&right.category_name))
                .then_with(|| left.category_id.cmp(&right.category_id))
        });
        summary
    }
}

fn safe_duration(duration: i64) -> i64 {
    duration.clamp(0, MAX_TRACKED_DURATION_MILLIS)
}

fn category_name<'a>(names: &'a HashMap<String, String>, id: Option<&str>) -> &'a str {
    id.and_then(|id| names.get(id))
        .map(String::as_str)
        .unwrap_or("未分类")
}

fn history_haystack(title: &str, note: &str, category_name: &str, slot_id: i32) -> String {
    format!("{title}\n{note}\n{category_name}\n格子 {slot_id:02}\n格子{slot_id}\n{slot_id}")
        .to_lowercase()
}

fn matches_filter(
    filter: &TimerHistoryFilter<'_>,
    category_id: Option<&str>,
    slot_id: i32,
) -> bool {
    filter.slot_id.is_none_or(|selected| selected == slot_id)
        && filter
            .category_id
            .is_none_or(|selected| selected == category_id.unwrap_or(""))
}

fn in_window(ended_at: i64, (start, end): (i64, i64)) -> bool {
    end > start && ended_at >= start && ended_at < end
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_data;
    use serde_json::json;

    #[test]
    fn synchronized_order_ignores_unknown_ids_and_keeps_new_slots_visible() {
        assert_eq!(
            vec![3, 1, 2, 100],
            ordered_slot_ids(&[1, 2, 3, 100, 2], &[3, 3, 99, 1])
        );
        assert!(ordered_slot_ids(&[], &[1]).is_empty());
    }

    #[test]
    fn projection_excludes_rest_and_matches_pause_persistence() {
        let raw = app_data::default_app_data_json(1_000);
        let started = app_data::start_slot_app_data_json(&raw, 1, 1_000).unwrap();
        let projector = TimerProjector::parse(&started).unwrap();
        let focus_target = projector.project(1_000).slots[0].micro_break_phase_target_millis;
        let now = 1_000 + focus_target + 5_000;
        let projection = projector.project(now);
        let running = projection.slots.iter().find(|slot| slot.id == 1).unwrap();
        assert_eq!(TimerViewPhase::Break, running.micro_break_phase);
        assert_eq!(focus_target, total_accumulated_millis(&projection));
        let paused = app_data::pause_slots_app_data_json(&started, &[1], now).unwrap();
        assert_eq!(
            total_accumulated_millis(&projection),
            total_accumulated_millis(&project_timer_views(&paused, now).unwrap())
        );
    }

    #[test]
    fn settling_a_running_slot_records_focus_without_restarting_or_duplicating_it() {
        let raw = app_data::default_app_data_json(1_000);
        let started = app_data::start_slot_app_data_json(&raw, 1, 1_000).unwrap();
        let projection = project_timer_views(&started, 1_000).unwrap();
        let slot = projection.slots.iter().find(|slot| slot.id == 1).unwrap();
        let run_id = slot.active_run_id.clone();
        let now = 1_000 + slot.micro_break_phase_target_millis;
        let settled = app_data::start_slot_app_data_json(&started, 1, now).unwrap();
        let value: serde_json::Value = serde_json::from_str(&settled).unwrap();
        assert_eq!(1, value["sessions"].as_array().unwrap().len());
        let after = project_timer_views(&settled, now).unwrap();
        let active = after.slots.iter().find(|slot| slot.id == 1).unwrap();
        assert!(active.is_running);
        assert_eq!(TimerViewPhase::Break, active.micro_break_phase);
        assert_eq!(run_id, active.active_run_id);
        let repeated = app_data::start_slot_app_data_json(&settled, 1, now).unwrap();
        let repeated_value: serde_json::Value = serde_json::from_str(&repeated).unwrap();
        assert_eq!(value["sessions"], repeated_value["sessions"]);
    }

    #[test]
    fn archive_restore_target_matches_shared_mutation_and_refuses_overwrite() {
        let raw = app_data::default_app_data_json(1_000);
        let named = app_data::update_slot_title_app_data_json(&raw, 2, "阅读", 1_001).unwrap();
        let archived = app_data::archive_slot_app_data_json(&named, 2, "archived", 1_002).unwrap();
        let occupied =
            app_data::update_slot_title_app_data_json(&archived, 2, "写作", 1_003).unwrap();
        let projection = project_timer_views(&occupied, 1_004).unwrap();
        let target = restore_target_slot_id(&projection, 2).unwrap();
        assert_ne!(target, 2);
        let restored =
            app_data::restore_archived_task_app_data_json(&occupied, "archived", 1_004).unwrap();
        let result = project_timer_views(&restored, 1_004).unwrap();
        assert_eq!(
            "阅读",
            result
                .slots
                .iter()
                .find(|slot| slot.id == target)
                .unwrap()
                .title
        );
        assert_eq!(
            "写作",
            result.slots.iter().find(|slot| slot.id == 2).unwrap().title
        );
        let mut full = result;
        for slot in &mut full.slots {
            slot.category_id = Some("work".to_string());
        }
        assert_eq!(None, restore_target_slot_id(&full, 2));
    }

    #[test]
    fn cross_midnight_and_exact_boundaries_follow_android_end_time_rule() {
        let raw = json!({"sessions": [
            {"id":"cross", "startedAtEpochMillis": 90, "endedAtEpochMillis": 110, "durationMillis":20},
            {"id":"start", "endedAtEpochMillis":100, "durationMillis":1},
            {"id":"end", "endedAtEpochMillis":200, "durationMillis":3},
            {"id":"before", "endedAtEpochMillis":99, "durationMillis":4}
        ]}).to_string();
        let index = TimerHistoryIndex::parse(&raw).unwrap();
        let summary = index.summarize(&TimerHistoryFilter::default(), (100, 200), (0, 300));
        assert_eq!(vec!["end", "cross", "start", "before"], summary.session_ids);
        assert_eq!(21, summary.today_millis);
        assert_eq!(28, summary.week_millis);
        assert_eq!(28, summary.total_millis);
        let android = crate::timer_insights::sum_timer_session_durations_in_window(
            &[110, 100, 200, 99],
            &[20, 1, 3, 4],
            100,
            200,
        )
        .unwrap();
        assert_eq!(android, summary.today_millis);
    }

    #[test]
    fn category_slot_and_all_search_terms_filter_sessions_and_archives() {
        let raw = json!({
            "categories":[{"id":"study", "name":"学习"}],
            "sessions":[
                {"id":"a", "slotId":1, "slotTitle":"Rust 阅读", "categoryId":"study", "durationMillis":40},
                {"id":"b", "slotId":2, "slotTitle":"Rust 阅读", "categoryId":"study", "durationMillis":60},
                {"id":"c", "slotId":1, "slotTitle":"杂项", "durationMillis":10}
            ],
            "archivedTasks":[
                {"id":"x", "originalSlotId":1, "title":"Rust", "note":"阅读笔记", "categoryId":"study", "accumulatedMillis":40},
                {"id":"y", "originalSlotId":1, "title":"无分类", "accumulatedMillis":10}
            ]
        }).to_string();
        let index = TimerHistoryIndex::parse(&raw).unwrap();
        let filter = TimerHistoryFilter {
            query: " RUST  学习 阅读 ",
            category_id: Some("study"),
            slot_id: Some(1),
        };
        let summary = index.summarize(&filter, (0, 1), (0, 1));
        assert_eq!(vec!["a"], summary.session_ids);
        assert_eq!(vec!["x"], summary.archived_task_ids);
        assert_eq!(40, summary.total_millis);
        let uncategorized = index.summarize(
            &TimerHistoryFilter {
                category_id: Some(""),
                ..Default::default()
            },
            (0, 1),
            (0, 1),
        );
        assert_eq!(vec!["c"], uncategorized.session_ids);
        assert_eq!(vec!["y"], uncategorized.archived_task_ids);
        assert_eq!(
            "未分类",
            uncategorized.durations_by_category[0].category_name
        );
        let all = index.summarize(&TimerHistoryFilter::default(), (0, 1), (0, 1));
        assert_eq!(
            110, all.total_millis,
            "archives must not double count focus time"
        );
        assert_eq!(100, all.durations_by_category[0].duration_millis);
    }

    #[test]
    fn extreme_durations_and_invalid_windows_are_safe() {
        let raw = json!({"sessions":[
            {"id":"a", "durationMillis": i64::MAX, "endedAtEpochMillis": i64::MAX},
            {"id":"b", "durationMillis": i64::MIN, "endedAtEpochMillis": i64::MIN},
            {"id":"c", "durationMillis": i64::MAX, "endedAtEpochMillis": 0}
        ]})
        .to_string();
        let summary = TimerHistoryIndex::parse(&raw).unwrap().summarize(
            &TimerHistoryFilter::default(),
            (1, 0),
            (i64::MIN, i64::MAX),
        );
        assert_eq!(0, summary.today_millis);
        assert_eq!(MAX_TRACKED_DURATION_MILLIS, summary.week_millis);
        assert_eq!(2 * MAX_TRACKED_DURATION_MILLIS, summary.total_millis);
    }

    #[test]
    fn original_empty_slot_wins_and_partial_micro_break_is_not_empty() {
        let raw = app_data::default_app_data_json(0);
        let mut projection = project_timer_views(&raw, 0).unwrap();
        assert_eq!(Some(3), restore_target_slot_id(&projection, 3));
        let slot = projection
            .slots
            .iter_mut()
            .find(|slot| slot.id == 3)
            .unwrap();
        slot.micro_break_phase_progress_millis = 1;
        assert!(!is_blank_timer(slot));
        assert_ne!(Some(3), restore_target_slot_id(&projection, 3));
    }
}
