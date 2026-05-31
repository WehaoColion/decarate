use crate::app_data::sanitize_app_data_json;

const MICRO_BREAK_FOCUS_MIN_MILLIS: i64 = 3 * 60 * 1_000;
const MICRO_BREAK_FOCUS_MAX_MILLIS: i64 = 5 * 60 * 1_000;
const MICRO_BREAK_FOCUS_STEP_MILLIS: i64 = 15_000;
const MICRO_BREAK_FOCUS_VARIANT_COUNT: i64 =
    ((MICRO_BREAK_FOCUS_MAX_MILLIS - MICRO_BREAK_FOCUS_MIN_MILLIS) / MICRO_BREAK_FOCUS_STEP_MILLIS)
        + 1;
const MICRO_BREAK_REST_MILLIS: i64 = 15_000;
const MAX_TRACKED_DURATION_MILLIS: i64 = 10 * 365 * 24 * 60 * 60 * 1_000;
const MICRO_BREAK_PHASE_FOCUS: i32 = 0;
const MICRO_BREAK_PHASE_BREAK: i32 = 1;
const HOME_FOCUS_KIND_RUNNING: i32 = 0;
const HOME_FOCUS_KIND_RESUME: i32 = 1;
const HOME_FOCUS_KIND_START_FRESH: i32 = 2;

pub fn select_persisted_app_data_json(
    primary_json: &str,
    backup_json: &str,
    now: i64,
) -> Option<String> {
    [primary_json, backup_json]
        .into_iter()
        .find_map(|candidate| sanitize_candidate_app_data(candidate, now))
}

fn sanitize_candidate_app_data(candidate: &str, now: i64) -> Option<String> {
    let trimmed = candidate.trim();
    if trimmed.is_empty() {
        return None;
    }
    sanitize_app_data_json(trimmed, now)
}

pub fn sum_timer_session_durations_in_window(
    ended_at_epoch_millis: &[i64],
    duration_millis: &[i64],
    window_start_epoch_millis: i64,
    window_end_epoch_millis: i64,
) -> Option<i64> {
    if ended_at_epoch_millis.len() != duration_millis.len()
        || window_end_epoch_millis < window_start_epoch_millis
    {
        return None;
    }

    Some(
        ended_at_epoch_millis
            .iter()
            .zip(duration_millis.iter())
            .filter(|(ended_at, _)| {
                **ended_at >= window_start_epoch_millis && **ended_at < window_end_epoch_millis
            })
            .fold(0_i64, |total, (_, duration)| {
                total.saturating_add(sanitize_tracked_duration(*duration))
            }),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerSessionWindowSummary {
    pub total_duration_millis: i64,
    pub session_count: i32,
    pub busiest_slot_id: i32,
    pub busiest_slot_duration_millis: i64,
    pub latest_session_index: i32,
    pub peak_session_index: i32,
}

impl TimerSessionWindowSummary {
    pub fn empty() -> Self {
        Self {
            total_duration_millis: 0,
            session_count: 0,
            busiest_slot_id: -1,
            busiest_slot_duration_millis: 0,
            latest_session_index: -1,
            peak_session_index: -1,
        }
    }

    pub fn as_i64_array(self) -> [i64; 6] {
        [
            self.total_duration_millis,
            self.session_count as i64,
            self.busiest_slot_id as i64,
            self.busiest_slot_duration_millis,
            self.latest_session_index as i64,
            self.peak_session_index as i64,
        ]
    }
}

pub fn summarize_timer_sessions_in_window(
    slot_ids: &[i32],
    ended_at_epoch_millis: &[i64],
    duration_millis: &[i64],
    window_start_epoch_millis: i64,
    window_end_epoch_millis: i64,
) -> Option<TimerSessionWindowSummary> {
    if slot_ids.len() != ended_at_epoch_millis.len()
        || slot_ids.len() != duration_millis.len()
        || window_end_epoch_millis < window_start_epoch_millis
    {
        return None;
    }

    let mut summary = TimerSessionWindowSummary::empty();
    let mut per_slot_totals: Vec<(i32, i64)> = Vec::new();
    let mut latest_ended_at = i64::MIN;
    let mut peak_duration = i64::MIN;

    for index in 0..slot_ids.len() {
        let ended_at = ended_at_epoch_millis[index];
        if ended_at < window_start_epoch_millis || ended_at >= window_end_epoch_millis {
            continue;
        }

        let duration = sanitize_tracked_duration(duration_millis[index]);
        summary.total_duration_millis = summary.total_duration_millis.saturating_add(duration);
        summary.session_count = summary.session_count.saturating_add(1);

        if ended_at > latest_ended_at {
            latest_ended_at = ended_at;
            summary.latest_session_index = index as i32;
        }
        if duration > peak_duration {
            peak_duration = duration;
            summary.peak_session_index = index as i32;
        }

        add_slot_duration(&mut per_slot_totals, slot_ids[index], duration);
    }

    for (slot_id, total) in per_slot_totals {
        if summary.busiest_slot_id < 0 || total > summary.busiest_slot_duration_millis {
            summary.busiest_slot_id = slot_id;
            summary.busiest_slot_duration_millis = total;
        }
    }

    Some(summary)
}

fn add_slot_duration(per_slot_totals: &mut Vec<(i32, i64)>, slot_id: i32, duration: i64) {
    if let Some((_, total)) = per_slot_totals
        .iter_mut()
        .find(|(existing_slot_id, _)| *existing_slot_id == slot_id)
    {
        *total = total.saturating_add(duration);
    } else {
        per_slot_totals.push((slot_id, duration));
    }
}

pub fn next_micro_break_delay_millis(
    slot_ids: &[i32],
    running_since_epoch_millis: &[i64],
    phase_codes: &[i32],
    cycle_indices: &[i32],
    phase_progress_millis: &[i64],
    now: i64,
) -> Option<i64> {
    if slot_ids.len() != running_since_epoch_millis.len()
        || slot_ids.len() != phase_codes.len()
        || slot_ids.len() != cycle_indices.len()
        || slot_ids.len() != phase_progress_millis.len()
    {
        return None;
    }

    slot_ids
        .iter()
        .enumerate()
        .filter_map(|(index, slot_id)| {
            let running_since = running_since_epoch_millis[index];
            if running_since < 0 {
                return None;
            }
            let phase = normalize_phase_code(phase_codes[index]);
            let target = phase_target_millis(*slot_id, cycle_indices[index], phase);
            let active_elapsed = safe_elapsed_since(Some(running_since), now);
            let progress = phase_progress_millis[index]
                .saturating_add(active_elapsed)
                .clamp(0, target);
            Some((target - progress).max(0))
        })
        .min()
}

pub fn home_rhythm_narrative(
    active_count: i32,
    today_session_count: i32,
    today_total: i64,
    latest_moment_label: &str,
    latest_slot_title: &str,
    latest_duration: i64,
    peak_duration: i64,
    focus_kind_code: i32,
    focus_slot_title: &str,
    busiest_slot_title: &str,
    busiest_duration: i64,
) -> String {
    if active_count > 0 && !focus_slot_title.trim().is_empty() {
        let settled_text = if today_session_count > 0 {
            format!(
                "今天已经完成了 {} 次记录，总计{}。",
                today_session_count,
                format_compact_duration_label(today_total)
            )
        } else {
            "今天还没有完成记录。".to_string()
        };
        return format!(
            "{} 眼下最合适的下一步，是继续 {}。沿着同一条线往前走，节奏最不容易断。",
            settled_text,
            focus_slot_title.trim()
        );
    }

    if !busiest_slot_title.trim().is_empty() && busiest_duration > 0 {
        let latest_text = if latest_moment_label.trim().is_empty() {
            String::new()
        } else {
            format!("最近一次结束在{}。", latest_moment_label.trim())
        };
        return format!(
            "今天最投入的是{}，累计{}。{}",
            busiest_slot_title.trim(),
            format_compact_duration_label(busiest_duration),
            latest_text
        );
    }

    if peak_duration >= 0 {
        return format!(
            "到目前为止，最长的一次专注持续了{}，说明节奏已经慢慢热起来了。",
            format_compact_duration_label(peak_duration)
        );
    }

    if latest_duration >= 0 && !latest_moment_label.trim().is_empty() {
        return format!(
            "最近一次结束在{}。{} 记录了{}。",
            latest_moment_label.trim(),
            latest_slot_title.trim(),
            format_compact_duration_label(latest_duration)
        );
    }

    if focus_kind_code == HOME_FOCUS_KIND_START_FRESH {
        return "今天还没有历史记录。先选一个干净的格子开始，等节奏起来后再补名字或分类。"
            .to_string();
    }

    "看板已经准备好了。打开建议格子，把下一段工作重新接上。".to_string()
}

pub fn home_focus_guidance(
    focus_kind_code: i32,
    slot_id: i32,
    elapsed_millis: i64,
    has_title: bool,
    has_note: bool,
) -> String {
    let slot_id_label = slot_label(slot_id);
    match focus_kind_code {
        HOME_FOCUS_KIND_RUNNING => format!(
            "格子 {} 已经连续计时{}，继续保持这个节奏就好。",
            slot_id_label,
            format_compact_duration_label(elapsed_millis)
        ),
        HOME_FOCUS_KIND_RESUME if elapsed_millis > 0 => format!(
            "格子 {} 已经累计了{}，直接接着做，比重开更顺。",
            slot_id_label,
            format_compact_duration_label(elapsed_millis)
        ),
        HOME_FOCUS_KIND_RESUME if has_title => {
            format!(
                "格子 {} 已经有标题了，随时可以从上次停下的地方继续。",
                slot_id_label
            )
        }
        HOME_FOCUS_KIND_RESUME if has_note => {
            format!(
                "格子 {} 里还留着备注，重新打开它，让备注带你进入下一步。",
                slot_id_label
            )
        }
        HOME_FOCUS_KIND_RESUME => {
            format!(
                "格子 {} 正在等你，重新打开它，沿着原来的线继续。",
                slot_id_label
            )
        }
        _ => format!(
            "就从格子 {} 开始。先动起来，过几分钟再补标题或分类也不迟。",
            slot_id_label
        ),
    }
}

pub fn header_metric_value(
    metric_code: i32,
    active_count: i32,
    today_total: i64,
    archived_count: i32,
) -> String {
    match metric_code {
        0 => format!("{:02}", active_count.max(0)),
        1 => format_compact_duration_label(today_total),
        2 => format!("{:02}", archived_count.max(0)),
        _ => String::new(),
    }
}

pub fn header_metric_note(
    metric_code: i32,
    active_count: i32,
    today_session_count: i32,
    archived_count: i32,
) -> String {
    match metric_code {
        0 if active_count <= 0 => "\u{5f53}\u{524d}\u{6ca1}\u{6709}\u{683c}\u{5b50}\u{5728}\u{8ba1}\u{65f6}".to_string(),
        0 if active_count == 1 => "\u{73b0}\u{5728}\u{6709} 1 \u{4e2a}\u{683c}\u{5b50}\u{6b63}\u{5728}\u{8ba1}\u{65f6}".to_string(),
        0 => "\u{73b0}\u{5728}\u{6709}\u{591a}\u{4e2a}\u{683c}\u{5b50}\u{5728}\u{5e76}\u{884c}\u{8ba1}\u{65f6}".to_string(),
        1 if today_session_count <= 0 => "\u{7b2c}\u{4e00}\u{6761}\u{5b8c}\u{6210}\u{8bb0}\u{5f55}\u{4f1a}\u{51fa}\u{73b0}\u{5728}\u{8fd9}\u{91cc}".to_string(),
        1 => format!(
            "\u{4eca}\u{5929}\u{5df2}\u{8bb0}\u{5f55} {} \u{6b21}",
            today_session_count.max(0)
        ),
        2 if archived_count <= 0 => "\u{76ee}\u{524d}\u{770b}\u{677f}\u{8fd8}\u{662f}\u{7a7a}\u{7684}".to_string(),
        2 => "\u{5f52}\u{6863}\u{4f1a}\u{4fdd}\u{5b58}\u{5728}\u{8fd9}\u{91cc}\u{ff0c}\u{968f}\u{65f6}\u{90fd}\u{80fd}\u{6062}\u{590d}".to_string(),
        _ => String::new(),
    }
}

pub fn timer_slot_status_label(
    is_running: bool,
    is_on_break: bool,
    phase_remaining_millis: i64,
    running_since_label: &str,
    is_blank_slate: bool,
    today_duration: i64,
    latest_finish_label: &str,
) -> String {
    if is_running && is_on_break {
        return format!(
            "\u{5fae}\u{4f11}\u{606f}\u{4e2d}\u{ff0c}\u{8fd8}\u{5269} {}",
            format_countdown_duration(phase_remaining_millis)
        );
    }
    if is_running {
        let label = if running_since_label.trim().is_empty() {
            "--"
        } else {
            running_since_label.trim()
        };
        return format!("\u{4ece} {label} \u{5f00}\u{59cb}\u{8ba1}\u{65f6}");
    }
    if is_blank_slate {
        return "\u{7a7a}\u{767d}\u{683c}\u{5b50}\u{ff0c}\u{70b9}\u{4e00}\u{4e0b}\u{5c31}\u{80fd}\u{5f00}\u{59cb}\u{65b0}\u{7684}\u{4e13}\u{6ce8}\u{65f6}\u{6bb5}\u{3002}".to_string();
    }
    if today_duration > 0 {
        return format!(
            "\u{4eca}\u{5929}\u{5df2}\u{4e13}\u{6ce8} {}",
            format_compact_duration_label(today_duration)
        );
    }
    if !latest_finish_label.trim().is_empty() {
        return format!(
            "\u{6700}\u{8fd1}\u{7ed3}\u{675f}\u{4e8e} {}",
            latest_finish_label.trim()
        );
    }
    "\u{8fd9}\u{4e2a}\u{683c}\u{5b50}\u{968f}\u{65f6}\u{53ef}\u{4ee5}\u{7ee7}\u{7eed}\u{3002}"
        .to_string()
}

pub fn timer_slot_footer_label(
    note: &str,
    is_running: bool,
    is_on_break: bool,
    can_archive: bool,
    session_count: i32,
    today_duration: i64,
    is_blank_slate: bool,
) -> String {
    let note = note.trim();
    if !note.is_empty() {
        return note.to_string();
    }
    if is_running && is_on_break {
        return "\u{8fd9}\u{4e00}\u{8f6e}\u{6b63}\u{5728}\u{4f11}\u{606f} 15 \u{79d2}\u{ff0c}\u{7ed3}\u{675f}\u{94c3}\u{58f0}\u{54cd}\u{8d77}\u{540e}\u{4f1a}\u{81ea}\u{52a8}\u{56de}\u{5230}\u{4e13}\u{6ce8}\u{3002}".to_string();
    }
    if is_running {
        return "\u{8fd9}\u{4e2a}\u{683c}\u{5b50}\u{4f1a}\u{6309}\u{6bcf}\u{8f6e}\u{4e13}\u{6ce8} 3 \u{5230} 5 \u{5206}\u{949f}\u{3001}\u{4f11}\u{606f} 15 \u{79d2}\u{7684}\u{8282}\u{594f}\u{7ee7}\u{7eed}\u{ff0c}\u{5e76}\u{5728}\u{5207}\u{6362}\u{65f6}\u{7528}\u{4e0d}\u{540c}\u{94c3}\u{58f0}\u{63d0}\u{9192}\u{3002}".to_string();
    }
    if can_archive {
        return "\u{8fd9}\u{91cc}\u{53ef}\u{4ee5}\u{5f52}\u{6863}\u{5df2}\u{5b8c}\u{6210}\u{7684}\u{4efb}\u{52a1}\u{ff0c}\u{817e}\u{51fa}\u{7a7a}\u{95f4}\u{53c8}\u{4e0d}\u{4f1a}\u{4e22}\u{8bb0}\u{5f55}\u{3002}".to_string();
    }
    if session_count > 0 && today_duration > 0 {
        return format!(
            "\u{5df2}\u{8bb0}\u{5f55} {} \u{6b21}\u{ff0c}\u{4eca}\u{5929}\u{7d2f}\u{8ba1} {}",
            session_count,
            format_compact_duration_label(today_duration)
        );
    }
    if session_count > 0 {
        return format!(
            "\u{76ee}\u{524d}\u{5df2}\u{7ecf}\u{8bb0}\u{5f55} {} \u{6b21}",
            session_count
        );
    }
    if is_blank_slate {
        return "\u{8865}\u{4e00}\u{4e2a}\u{6807}\u{9898}\u{3001}\u{5206}\u{7c7b}\u{6216}\u{5907}\u{6ce8}\u{ff0c}\u{8fd9}\u{4e2a}\u{683c}\u{5b50}\u{5c31}\u{4f1a}\u{7acb}\u{523b}\u{66f4}\u{6709}\u{72b6}\u{6001}\u{3002}".to_string();
    }
    "\u{7559}\u{4e00}\u{53e5}\u{5907}\u{6ce8}\u{ff0c}\u{56de}\u{6765}\u{65f6}\u{5c31}\u{80fd}\u{7acb}\u{523b}\u{77e5}\u{9053}\u{4e0b}\u{4e00}\u{6b65}\u{505a}\u{4ec0}\u{4e48}\u{3002}".to_string()
}

pub fn timer_detail_status_line(
    is_running: bool,
    is_on_break: bool,
    phase_remaining_millis: i64,
    can_archive: bool,
) -> String {
    if is_running && is_on_break {
        return format!(
            "\u{6b63}\u{5728}\u{5fae}\u{4f11}\u{606f}\u{ff0c}\u{8fd8}\u{5269} {}\u{ff0c}\u{7ed3}\u{675f}\u{540e}\u{4f1a}\u{81ea}\u{52a8}\u{56de}\u{5230}\u{4e13}\u{6ce8}\u{3002}",
            format_countdown_duration(phase_remaining_millis)
        );
    }
    if is_running {
        return "\u{6b63}\u{5728}\u{8ba1}\u{65f6}\u{3002}\u{53ea}\u{8981}\u{4e0d}\u{6682}\u{505c}\u{ff0c}\u{8fd9}\u{4e2a}\u{683c}\u{5b50}\u{5c31}\u{4f1a}\u{6301}\u{7eed}\u{7d2f}\u{8ba1}\u{65f6}\u{95f4}\u{3002}".to_string();
    }
    if can_archive {
        return "\u{8fd9}\u{9879}\u{5de5}\u{4f5c}\u{5df2}\u{7ecf}\u{5b8c}\u{6210}\u{3002}\u{60f3}\u{8ba9}\u{770b}\u{677f}\u{66f4}\u{6e05}\u{723d}\u{65f6}\u{ff0c}\u{53ef}\u{4ee5}\u{628a}\u{5b83}\u{5f52}\u{6863}\u{3002}".to_string();
    }
    "\u{8fd9}\u{4e2a}\u{683c}\u{5b50}\u{5df2}\u{7ecf}\u{51c6}\u{5907}\u{597d}\u{ff0c}\u{60f3}\u{7ee7}\u{7eed}\u{65f6}\u{968f}\u{65f6}\u{56de}\u{6765}\u{3002}".to_string()
}

pub fn timer_detail_summary_note(
    summary_kind_code: i32,
    is_running: bool,
    is_on_break: bool,
    can_archive: bool,
    today_session_count: i32,
    moment_label: &str,
) -> String {
    match summary_kind_code {
        0 if is_running && is_on_break => "\u{5f53}\u{524d}\u{6b63}\u{5728} 15 \u{79d2}\u{5fae}\u{4f11}\u{606f}\u{ff0c}\u{7d2f}\u{8ba1}\u{4e13}\u{6ce8}\u{65f6}\u{957f}\u{4e0d}\u{4f1a}\u{628a}\u{4f11}\u{606f}\u{65f6}\u{95f4}\u{7b97}\u{8fdb}\u{53bb}\u{3002}".to_string(),
        0 if is_running => "\u{8fd9}\u{4e2a}\u{683c}\u{5b50}\u{73b0}\u{5728}\u{6b63}\u{5728}\u{6301}\u{7eed}\u{8ba1}\u{65f6}\u{3002}".to_string(),
        0 if can_archive => "\u{8fd9}\u{91cc}\u{7684}\u{5df2}\u{5b8c}\u{6210}\u{4efb}\u{52a1}\u{53ef}\u{4ee5}\u{5f52}\u{6863}\u{ff0c}\u{8ba9}\u{770b}\u{677f}\u{66f4}\u{5e72}\u{51c0}\u{3002}".to_string(),
        0 => "\u{51c6}\u{5907}\u{597d}\u{4ee5}\u{540e}\u{ff0c}\u{53ef}\u{4ee5}\u{5728}\u{8fd9}\u{91cc}\u{6682}\u{505c}\u{3001}\u{7f16}\u{8f91}\u{6216}\u{91cd}\u{65b0}\u{5f00}\u{59cb}\u{3002}".to_string(),
        1 if today_session_count <= 0 => "\u{4eca}\u{5929}\u{8fd9}\u{4e2a}\u{683c}\u{5b50}\u{8fd8}\u{6ca1}\u{6709}\u{5b8c}\u{6210}\u{8bb0}\u{5f55}\u{3002}".to_string(),
        1 => format!(
            "\u{4eca}\u{5929}\u{5df2}\u{5b8c}\u{6210} {} \u{6b21}\u{8bb0}\u{5f55}\u{3002}",
            today_session_count
        ),
        2 if !moment_label.trim().is_empty() => {
            format!("\u{5b8c}\u{6210}\u{4e8e} {}", moment_label.trim())
        }
        2 => "\u{8fd9}\u{4e2a}\u{683c}\u{5b50}\u{8fd8}\u{6ca1}\u{6709}\u{8f83}\u{957f}\u{7684}\u{5df2}\u{5b8c}\u{6210}\u{8bb0}\u{5f55}\u{3002}".to_string(),
        3 => "\u{8fd9}\u{662f}\u{4eca}\u{5929}\u{6240}\u{6709}\u{683c}\u{5b50}\u{5df2}\u{7ecf}\u{5b8c}\u{6210}\u{7684}\u{603b}\u{65f6}\u{957f}\u{3002}".to_string(),
        _ => String::new(),
    }
}

pub fn history_empty_message(has_search_query: bool, has_archived_tasks: bool) -> String {
    match (has_search_query, has_archived_tasks) {
        (true, false) => "\u{6ca1}\u{6709}\u{627e}\u{5230}\u{5339}\u{914d}\u{7684}\u{5386}\u{53f2}\u{5185}\u{5bb9}\u{ff0c}\u{8bd5}\u{8bd5}\u{522b}\u{7684}\u{5173}\u{952e}\u{8bcd}\u{3002}".to_string(),
        (true, true) => "\u{6ca1}\u{6709}\u{627e}\u{5230}\u{5339}\u{914d}\u{7684}\u{8ba1}\u{65f6}\u{8bb0}\u{5f55}\u{3002}".to_string(),
        (false, false) => "\u{8fd8}\u{6ca1}\u{6709}\u{5f52}\u{6863}\u{4efb}\u{52a1}\u{3002}\u{5b8c}\u{6210}\u{7684}\u{4efb}\u{52a1}\u{5f52}\u{6863}\u{540e}\u{4f1a}\u{51fa}\u{73b0}\u{5728}\u{8fd9}\u{91cc}\u{3002}".to_string(),
        (false, true) => "\u{5f52}\u{6863}\u{4efb}\u{52a1}\u{4f1a}\u{4fdd}\u{5b58}\u{5728}\u{8fd9}\u{91cc}\u{ff0c}\u{968f}\u{65f6}\u{90fd}\u{80fd}\u{6062}\u{590d}\u{5230}\u{4e5d}\u{5bab}\u{683c}\u{3002}".to_string(),
    }
}

pub fn archived_task_detail_text(archived_at_label: &str, original_slot_id: i32) -> String {
    format!(
        "\u{5f52}\u{6863}\u{4e8e} {}\u{ff0c}\u{6765}\u{81ea}\u{683c}\u{5b50} {}",
        if archived_at_label.trim().is_empty() {
            "--"
        } else {
            archived_at_label.trim()
        },
        slot_label(original_slot_id)
    )
}

pub fn archived_task_restore_target_text(restore_target_slot_id: i32) -> String {
    if restore_target_slot_id < 0 {
        "\u{5f53}\u{524d}\u{6ca1}\u{6709}\u{7a7a}\u{95f2}\u{683c}\u{5b50}\u{53ef}\u{6062}\u{590d}\u{3002}".to_string()
    } else {
        format!(
            "\u{5c06}\u{6062}\u{590d}\u{5230}\u{683c}\u{5b50} {}",
            slot_label(restore_target_slot_id)
        )
    }
}

pub fn archived_task_restore_action_label(
    restore_target_slot_id: i32,
    original_slot_id: i32,
) -> String {
    if restore_target_slot_id < 0 {
        "\u{6ca1}\u{6709}\u{7a7a}\u{95f2}\u{683c}\u{5b50}".to_string()
    } else if restore_target_slot_id == original_slot_id {
        "\u{6062}\u{590d}\u{539f}\u{683c}\u{5b50}".to_string()
    } else {
        format!(
            "\u{6062}\u{590d}\u{5230}\u{683c}\u{5b50} {}",
            slot_label(restore_target_slot_id)
        )
    }
}

pub fn note_updated_label(is_today: bool, time_label: &str, date_label: &str) -> String {
    if is_today {
        format!(
            "\u{4eca}\u{5929} {} \u{66f4}\u{65b0}",
            if time_label.trim().is_empty() {
                "--:--"
            } else {
                time_label.trim()
            }
        )
    } else if date_label.trim().is_empty() {
        "--".to_string()
    } else {
        date_label.trim().to_string()
    }
}

pub fn shareable_note_text(title: &str, content: &str) -> String {
    if content.trim().is_empty() {
        title.to_string()
    } else {
        format!("{title}\n\n{content}")
    }
}

pub fn diagnostic_log_button_label(is_preparing_log: bool, recording_active: bool) -> String {
    if is_preparing_log {
        "导出中...".to_string()
    } else if recording_active {
        "导出日志".to_string()
    } else {
        "开始记录".to_string()
    }
}

pub fn diagnostic_data_export_button_label(is_exporting_data: bool) -> String {
    if is_exporting_data {
        "\u{6253}\u{5305}\u{4e2d}...".to_string()
    } else {
        "\u{5bfc}\u{51fa}\u{6570}\u{636e}".to_string()
    }
}

pub fn diagnostic_data_export_status_message(status_kind_code: i32) -> String {
    match status_kind_code {
        0 => "\u{6b63}\u{5728}\u{6253}\u{5305}\u{6570}\u{636e}\u{ff0c}\u{5185}\u{5bb9}\u{5305}\u{62ec}\u{5f53}\u{524d}\u{6570}\u{636e}\u{3001}\u{5907}\u{4efd}\u{72b6}\u{6001}\u{548c}\u{6700}\u{8fd1}\u{7684}\u{5e94}\u{7528}\u{4e8b}\u{4ef6}\u{3002}".to_string(),
        1 => "\u{5bfc}\u{51fa}\u{6570}\u{636e}\u{5305}\u{5931}\u{8d25}\u{ff0c}\u{8bf7}\u{91cd}\u{8bd5}\u{3002}".to_string(),
        _ => String::new(),
    }
}

pub fn finance_data_status_message(status_kind_code: i32, file_name: &str) -> String {
    let safe_file_name = if file_name.trim().is_empty() {
        "--"
    } else {
        file_name.trim()
    };
    match status_kind_code {
        0 => "\u{5df2}\u{53d6}\u{6d88}\u{6062}\u{590d}\u{ff0c}\u{5f53}\u{524d}\u{8d22}\u{52a1}\u{6570}\u{636e}\u{672a}\u{6539}\u{52a8}\u{3002}".to_string(),
        1 => "\u{6b63}\u{5728}\u{8bfb}\u{53d6}\u{8d22}\u{52a1}\u{5907}\u{4efd}...".to_string(),
        2 => "\u{8d22}\u{52a1}\u{6570}\u{636e}\u{5df2}\u{6062}\u{590d}\u{ff0c}\u{53ea}\u{8986}\u{76d6}\u{8d22}\u{52a1}\u{677f}\u{5757}\u{3002}".to_string(),
        3 => "\u{6062}\u{590d}\u{5931}\u{8d25}\u{ff0c}\u{8bf7}\u{786e}\u{8ba4}\u{9009}\u{62e9}\u{7684}\u{662f}\u{8d22}\u{52a1}\u{5907}\u{4efd}\u{6587}\u{4ef6}\u{3002}".to_string(),
        4 => "\u{6b63}\u{5728}\u{751f}\u{6210}\u{8d22}\u{52a1}\u{5907}\u{4efd}\u{ff0c}\u{5b8c}\u{6210}\u{540e}\u{4f1a}\u{81ea}\u{52a8}\u{6253}\u{5f00}\u{7cfb}\u{7edf}\u{5206}\u{4eab}\u{9762}\u{677f}\u{3002}".to_string(),
        5 => format!("\u{5df2}\u{521b}\u{5efa} {safe_file_name}\u{ff0c}\u{7cfb}\u{7edf}\u{5206}\u{4eab}\u{9762}\u{677f}\u{5df2}\u{7ecf}\u{6253}\u{5f00}\u{3002}"),
        6 => format!("\u{5df2}\u{521b}\u{5efa} {safe_file_name}\u{ff0c}\u{4f46}\u{6253}\u{5f00}\u{5206}\u{4eab}\u{9762}\u{677f}\u{5931}\u{8d25}\u{ff0c}\u{8bf7}\u{91cd}\u{8bd5}\u{3002}"),
        7 => "\u{5bfc}\u{51fa}\u{8d22}\u{52a1}\u{5907}\u{4efd}\u{5931}\u{8d25}\u{ff0c}\u{8bf7}\u{91cd}\u{8bd5}\u{3002}".to_string(),
        _ => String::new(),
    }
}

pub fn finance_data_action_label(is_exporting: bool) -> String {
    if is_exporting {
        "\u{5bfc}\u{51fa}\u{4e2d}...".to_string()
    } else {
        "\u{5bfc}\u{51fa}\u{8d22}\u{52a1}".to_string()
    }
}

pub fn finance_backup_file_name(version_name: &str, timestamp_label: &str) -> String {
    let version = trimmed_or(version_name, "unknown");
    let timestamp = trimmed_or(timestamp_label, "manual");
    format!("finance_backup_v{version}_{timestamp}.json")
}

pub fn finance_data_idle_status_message(version_name: &str) -> String {
    let version = trimmed_or(version_name, "unknown");
    format!(
        "\u{5efa}\u{8bae}\u{6bcf}\u{6b21}\u{505a}\u{5927}\u{6539}\u{524d}\u{5148}\u{5bfc}\u{51fa}\u{4e00}\u{4efd}\u{8d22}\u{52a1}\u{5907}\u{4efd}\u{3002}\u{6587}\u{4ef6}\u{4f1a}\u{5e26}\u{4e0a}\u{5f53}\u{524d}\u{7248}\u{672c}\u{53f7} {version}\u{3002}"
    )
}

pub fn finance_monthly_ledger_state_values(
    asset_count: i32,
    liability_count: i32,
    asset_draft_count: i32,
    liability_draft_count: i32,
    days_with_entries: i64,
    net_cashflow: i64,
    previous_snapshot_exists: bool,
    can_fill_asset_templates: bool,
    can_fill_liability_templates: bool,
) -> [i64; 6] {
    let asset_rows = asset_count.max(0) as i64 + asset_draft_count.max(0) as i64;
    let liability_rows = liability_count.max(0) as i64 + liability_draft_count.max(0) as i64;
    let avg_daily_cashflow = if days_with_entries > 0 {
        net_cashflow / days_with_entries
    } else {
        0
    };
    [
        asset_rows,
        liability_rows,
        avg_daily_cashflow,
        bool_as_i64(
            previous_snapshot_exists || can_fill_asset_templates || can_fill_liability_templates,
        ),
        bool_as_i64(asset_rows == 0),
        bool_as_i64(liability_rows == 0),
    ]
}

pub fn diagnostic_status_message(
    explicit_status: &str,
    recording_active: bool,
    has_crash_report: bool,
    recording_started_at_label: &str,
) -> String {
    let explicit = explicit_status.trim();
    if !explicit.is_empty() {
        return explicit.to_string();
    }
    let started_at = if recording_started_at_label.trim().is_empty() {
        "刚刚"
    } else {
        recording_started_at_label.trim()
    };
    if recording_active && has_crash_report {
        format!(
            "已从 {started_at} 开始持续记录，并检测到上次闪退信息。现在点“导出日志”把文件发回来。"
        )
    } else if recording_active {
        format!(
            "已从 {started_at} 开始持续记录应用事件。复现问题后点“导出日志”；如果中途闪退，重新打开应用后也能导出。"
        )
    } else {
        "先点“开始记录”，再按平时操作复现问题。日志会持续记录后续事件；如果闪退，重开应用后仍可导出崩溃信息。".to_string()
    }
}

pub fn diagnostic_share_status_message(file_name: &str, share_opened: bool) -> String {
    let safe_file_name = if file_name.trim().is_empty() {
        "--"
    } else {
        file_name.trim()
    };
    if share_opened {
        format!("已创建 {safe_file_name}，系统分享面板已经打开。")
    } else {
        format!("已创建 {safe_file_name}，但打开分享面板失败，请重试。")
    }
}

pub fn diagnostic_recording_started_status_message() -> String {
    "已开始持续记录。请按平时操作复现问题；如果中途闪退，重新打开应用后再点“导出日志”。".to_string()
}

pub fn diagnostic_recording_exported_status_message(file_name: &str) -> String {
    let safe_file_name = if file_name.trim().is_empty() {
        "--"
    } else {
        file_name.trim()
    };
    format!("已导出 {safe_file_name}。如果还要继续排查，请重新点“开始记录”。")
}

pub fn home_dock_board_summary_label(
    active_count: i32,
    active_label: &str,
    today_label: &str,
) -> String {
    if active_count > 0 {
        active_label.to_string()
    } else {
        today_label.to_string()
    }
}

pub fn home_dock_board_summary_value(
    active_count: i32,
    today_total: i64,
    today_total_label: &str,
) -> String {
    if active_count > 0 {
        format!("{:02}", active_count.max(0))
    } else if today_total > 0 {
        today_total_label.to_string()
    } else {
        "--".to_string()
    }
}

pub fn home_dock_history_summary_label(
    finance_has_entries: bool,
    latest_session_exists: bool,
    coverage_label: &str,
    recent_label: &str,
    archive_label: &str,
) -> String {
    if finance_has_entries {
        coverage_label.to_string()
    } else if latest_session_exists {
        recent_label.to_string()
    } else {
        archive_label.to_string()
    }
}

pub fn home_dock_history_summary_value(
    finance_has_entries: bool,
    latest_session_exists: bool,
    coverage_value: &str,
    latest_session_time_label: &str,
    archived_task_count: i32,
) -> String {
    if finance_has_entries {
        coverage_value.to_string()
    } else if latest_session_exists {
        latest_session_time_label.to_string()
    } else {
        format!("{:02}", archived_task_count.max(0))
    }
}

fn phase_target_millis(slot_id: i32, cycle_index: i32, phase_code: i32) -> i64 {
    if phase_code == MICRO_BREAK_PHASE_BREAK {
        MICRO_BREAK_REST_MILLIS
    } else {
        focus_target_millis_for_slot(slot_id, cycle_index.max(0))
    }
}

fn focus_target_millis_for_slot(slot_id: i32, cycle_index: i32) -> i64 {
    let mixed = mix_micro_break_seed(slot_id, cycle_index);
    let variant_index = (mixed % MICRO_BREAK_FOCUS_VARIANT_COUNT as u64) as i64;
    MICRO_BREAK_FOCUS_MIN_MILLIS + (variant_index * MICRO_BREAK_FOCUS_STEP_MILLIS)
}

fn mix_micro_break_seed(slot_id: i32, cycle_index: i32) -> u64 {
    let mut value = ((slot_id as i64 as u64) << 32) ^ (cycle_index as i64 as u64);
    value ^= value >> 33;
    value = value.wrapping_mul(0xff51_afd7_ed55_8ccd);
    value ^= value >> 33;
    value = value.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    value ^= value >> 33;
    value & i64::MAX as u64
}

fn normalize_phase_code(phase_code: i32) -> i32 {
    match phase_code {
        MICRO_BREAK_PHASE_BREAK => MICRO_BREAK_PHASE_BREAK,
        _ => MICRO_BREAK_PHASE_FOCUS,
    }
}

fn safe_elapsed_since(started_at_epoch_millis: Option<i64>, now: i64) -> i64 {
    match started_at_epoch_millis {
        Some(started_at) if started_at >= 0 && started_at <= now => now.saturating_sub(started_at),
        _ => 0,
    }
}

fn sanitize_tracked_duration(value: i64) -> i64 {
    value.clamp(0, MAX_TRACKED_DURATION_MILLIS)
}

fn format_countdown_duration(duration_millis: i64) -> String {
    let total_seconds = (duration_millis / 1_000).max(0);
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}

fn format_compact_duration_label(duration_millis: i64) -> String {
    let total_minutes = duration_millis.max(0) / 60_000;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    if hours > 0 {
        if minutes > 0 {
            format!("{}小时 {}分", hours, minutes)
        } else {
            format!("{}小时", hours)
        }
    } else {
        format!("{}分", minutes)
    }
}

fn slot_label(slot_id: i32) -> String {
    format!("{:02}", slot_id)
}

fn trimmed_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback
    } else {
        trimmed
    }
}

fn bool_as_i64(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_metric_texts_match_home_strip_branches() {
        assert_eq!("02", header_metric_value(0, 2, 0, 0));
        assert_eq!("03", header_metric_value(2, 0, 0, 3));
        assert!(!header_metric_note(0, 0, 0, 0).is_empty());
        assert!(header_metric_note(1, 0, 4, 0).contains("4"));
        assert!(!header_metric_note(2, 0, 0, 2).is_empty());
    }

    #[test]
    fn timer_tile_texts_cover_running_break_archive_and_history_states() {
        assert!(
            timer_slot_status_label(true, true, 15_000, "09:40", false, 0, "").contains("00:15")
        );
        assert!(timer_slot_status_label(true, false, 0, "09:40", false, 0, "").contains("09:40"));
        assert_ne!(
            timer_detail_status_line(false, false, 0, true),
            timer_detail_status_line(false, false, 0, false)
        );
        assert_ne!(
            history_empty_message(true, true),
            history_empty_message(false, false)
        );
        assert!(note_updated_label(true, "09:40", "").contains("09:40"));
        assert_eq!("title\n\nbody", shareable_note_text("title", "body"));
    }

    #[test]
    fn diagnostic_status_texts_cover_recording_export_and_share_states() {
        assert_eq!("导出中...", diagnostic_log_button_label(true, false));
        assert_eq!("导出日志", diagnostic_log_button_label(false, true));
        assert_eq!("开始记录", diagnostic_log_button_label(false, false));
        assert_eq!(
            "已有状态",
            diagnostic_status_message(" 已有状态 ", true, true, "04月21日 10:00")
        );
        assert!(diagnostic_status_message("", true, true, "04月21日 10:00")
            .contains("检测到上次闪退信息"));
        assert!(diagnostic_status_message("", true, false, "")
            .contains("已从 刚刚 开始持续记录应用事件"));
        assert!(diagnostic_status_message("", false, false, "").contains("先点“开始记录”"));
        assert_eq!(
            "已创建 diag.txt，系统分享面板已经打开。",
            diagnostic_share_status_message("diag.txt", true)
        );
        assert_eq!(
            "已创建 diag.txt，但打开分享面板失败，请重试。",
            diagnostic_share_status_message("diag.txt", false)
        );
        assert!(
            diagnostic_recording_exported_status_message("diag.txt").contains("已导出 diag.txt")
        );
    }

    #[test]
    fn migrated_detail_export_and_finance_texts_keep_ui_branches() {
        assert!(timer_detail_summary_note(0, true, true, false, 0, "").contains("15"));
        assert!(timer_detail_summary_note(1, false, false, false, 3, "").contains("3"));
        assert!(timer_detail_summary_note(2, false, false, false, 0, "09:40").contains("09:40"));
        assert!(!timer_detail_summary_note(3, false, false, false, 0, "").is_empty());

        assert!(archived_task_restore_target_text(2).contains("02"));
        assert!(archived_task_restore_action_label(2, 1).contains("02"));
        assert_ne!(
            archived_task_restore_target_text(-1),
            archived_task_restore_target_text(2)
        );

        assert_eq!(
            "\u{6253}\u{5305}\u{4e2d}...",
            diagnostic_data_export_button_label(true)
        );
        assert_eq!(
            "\u{5bfc}\u{51fa}\u{6570}\u{636e}",
            diagnostic_data_export_button_label(false)
        );
        assert!(!diagnostic_data_export_status_message(0).is_empty());
        assert!(!diagnostic_data_export_status_message(1).is_empty());

        assert_eq!(
            "\u{5bfc}\u{51fa}\u{4e2d}...",
            finance_data_action_label(true)
        );
        assert_eq!(
            "\u{5bfc}\u{51fa}\u{8d22}\u{52a1}",
            finance_data_action_label(false)
        );
        assert!(finance_data_status_message(5, "finance.json").contains("finance.json"));
        assert!(finance_data_status_message(6, "finance.json").contains("finance.json"));
        assert_eq!(
            "finance_backup_v2.10.6_20260421_220500.json",
            finance_backup_file_name("2.10.6", "20260421_220500")
        );
        assert_eq!(
            "finance_backup_vunknown_manual.json",
            finance_backup_file_name(" ", "")
        );
        assert!(finance_data_idle_status_message("2.10.6").contains("2.10.6"));
        assert_eq!(
            [3, 1, -250, 1, 0, 0],
            finance_monthly_ledger_state_values(2, 0, 1, 1, 2, -500, true, false, false)
        );
        assert_eq!(
            [0, 0, 0, 0, 1, 1],
            finance_monthly_ledger_state_values(0, 0, -2, -3, 0, 200, false, false, false)
        );
    }

    #[test]
    fn dock_summary_values_keep_labels_language_neutral() {
        assert_eq!(
            "Active",
            home_dock_board_summary_label(1, "Active", "Today")
        );
        assert_eq!("Today", home_dock_board_summary_label(0, "Active", "Today"));
        assert_eq!("03", home_dock_board_summary_value(3, 0, "2小时"));
        assert_eq!("2小时", home_dock_board_summary_value(0, 120_000, "2小时"));
        assert_eq!("--", home_dock_board_summary_value(0, 0, "0分"));
        assert_eq!(
            "Coverage",
            home_dock_history_summary_label(true, true, "Coverage", "Recent", "Archive")
        );
        assert_eq!(
            "Recent",
            home_dock_history_summary_label(false, true, "Coverage", "Recent", "Archive")
        );
        assert_eq!(
            "Archive",
            home_dock_history_summary_label(false, false, "Coverage", "Recent", "Archive")
        );
        assert_eq!(
            "28%",
            home_dock_history_summary_value(true, true, "28%", "09:40", 7)
        );
        assert_eq!(
            "09:40",
            home_dock_history_summary_value(false, true, "28%", "09:40", 7)
        );
        assert_eq!(
            "07",
            home_dock_history_summary_value(false, false, "28%", "09:40", 7)
        );
    }

    #[test]
    fn sums_only_sessions_inside_window_and_clamps_duration() {
        let total = sum_timer_session_durations_in_window(
            &[99, 100, 199, 200],
            &[10, -10, i64::MAX, 50],
            100,
            200,
        );

        assert_eq!(Some(MAX_TRACKED_DURATION_MILLIS), total);
    }

    #[test]
    fn next_micro_break_delay_uses_active_slots_only() {
        let delay = next_micro_break_delay_millis(
            &[1, 2],
            &[-1, 1_000],
            &[MICRO_BREAK_PHASE_FOCUS, MICRO_BREAK_PHASE_BREAK],
            &[0, 0],
            &[0, 10_000],
            11_000,
        );

        assert_eq!(Some(0), delay);
    }

    #[test]
    fn summarizes_window_with_busiest_latest_and_peak_sessions() {
        let summary = summarize_timer_sessions_in_window(
            &[3, 2, 3, 2, 4],
            &[90, 110, 150, 170, 220],
            &[20_000, 40_000, -10, 70_000, 80_000],
            100,
            200,
        )
        .unwrap();

        assert_eq!(
            TimerSessionWindowSummary {
                total_duration_millis: 110_000,
                session_count: 3,
                busiest_slot_id: 2,
                busiest_slot_duration_millis: 110_000,
                latest_session_index: 3,
                peak_session_index: 3,
            },
            summary
        );
    }

    #[test]
    fn home_guidance_preserves_resume_branches() {
        assert_eq!(
            "格子 03 已经累计了5分，直接接着做，比重开更顺。",
            home_focus_guidance(HOME_FOCUS_KIND_RESUME, 3, 5 * 60_000, true, true)
        );
        assert_eq!(
            "格子 03 里还留着备注，重新打开它，让备注带你进入下一步。",
            home_focus_guidance(HOME_FOCUS_KIND_RESUME, 3, 0, false, true)
        );
    }

    #[test]
    fn rhythm_narrative_prefers_running_focus() {
        assert_eq!(
            "今天已经完成了 2 次记录，总计1小时 5分。 眼下最合适的下一步，是继续 写论文。沿着同一条线往前走，节奏最不容易断。",
            home_rhythm_narrative(
                1,
                2,
                65 * 60_000,
                "10:30",
                "阅读",
                20 * 60_000,
                90 * 60_000,
                HOME_FOCUS_KIND_RUNNING,
                "写论文",
                "阅读",
                70 * 60_000,
            )
        );
    }
}
