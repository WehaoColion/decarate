// v2.23.2.6 - Generate offline Markdown and formula reading with safe render boundaries.
// v2.23.2.3 - Generate usable AI entry points, request boundaries and safe sync failures.
// v2.23.2.2 - Generate Android AI connection state and business tests.
// v2.23.2 - Generate complete knowledge groups and workspace filter recovery.
// v2.23.1.1 - Generate Android report synchronization without editing Kotlin outputs.
// v2.23.1 - Generate the Android update history for the My page.
// v2.22.49.11 - Generate faster Android note save paths.
// v2.22.49.5 - Generate lightweight note queries and cached summaries.
// v2.22.49 - Generate bounded parallel verification and fully drawn startup reporting.
// v2.22.48 - Generate run-bound rest bells and phase-correct reminders.
// v2.22.45 - Generate transaction-scoped local startup verification.
// v2.22.44 - Retain note navigation when switching Android applications.
// v2.22.43 - Generate stable timer geometry and pause controls.
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[path = "../sourcegen/android_ai_answer_ui.rs"]
mod android_ai_answer_ui;
#[path = "../sourcegen/android_ai_workflow.rs"]
mod android_ai_workflow;
#[path = "../sourcegen/android_backup_availability.rs"]
mod android_backup_availability;
#[path = "../sourcegen/android_canvas_ui.rs"]
mod android_canvas_ui;
#[path = "../sourcegen/android_device_timer_sync.rs"]
mod android_device_timer_sync;
#[path = "../sourcegen/android_document_block_alignment.rs"]
mod android_document_block_alignment;
#[path = "../sourcegen/android_document_caret.rs"]
mod android_document_caret;
#[path = "../sourcegen/android_document_markdown.rs"]
mod android_document_markdown;
#[path = "../sourcegen/android_finance_workspace.rs"]
mod android_finance_workspace;
#[path = "../sourcegen/android_history_storage.rs"]
mod android_history_storage;
#[path = "../sourcegen/android_jvm_test_sources.rs"]
mod android_jvm_test_sources;
#[path = "../sourcegen/android_knowledge_compat.rs"]
mod android_knowledge_compat;
#[path = "../sourcegen/android_knowledge_filters.rs"]
mod android_knowledge_filters;
#[path = "../sourcegen/android_knowledge_navigation.rs"]
mod android_knowledge_navigation;
#[path = "../sourcegen/android_legal_integration.rs"]
mod android_legal_integration;
#[path = "../sourcegen/android_legal_workflow.rs"]
mod android_legal_workflow;
#[path = "../sourcegen/android_local_startup.rs"]
mod android_local_startup;
#[path = "../sourcegen/android_note_background.rs"]
mod android_note_background;
#[path = "../sourcegen/android_note_collection_recovery.rs"]
mod android_note_collection_recovery;
#[path = "../sourcegen/android_note_latency.rs"]
mod android_note_latency;
#[path = "../sourcegen/android_note_list_performance.rs"]
mod android_note_list_performance;
#[path = "../sourcegen/android_note_save_performance.rs"]
mod android_note_save_performance;
#[path = "../sourcegen/android_note_save_queue.rs"]
mod android_note_save_queue;
#[path = "../sourcegen/android_performance_override.rs"]
mod android_performance_override;
#[path = "../sourcegen/android_privacy_generation.rs"]
mod android_privacy_generation;
#[path = "../sourcegen/android_responsiveness.rs"]
mod android_responsiveness;
#[path = "../sourcegen/android_rest_bell.rs"]
mod android_rest_bell;
#[path = "../sourcegen/android_snapshot_memory.rs"]
mod android_snapshot_memory;
#[path = "../sourcegen/android_sources.rs"]
mod android_sources;
#[path = "../sourcegen/android_startup_decode.rs"]
mod android_startup_decode;
#[path = "../sourcegen/android_startup_loading.rs"]
mod android_startup_loading;
#[path = "../sourcegen/android_startup_stream.rs"]
mod android_startup_stream;
#[path = "../sourcegen/android_sync_failure.rs"]
mod android_sync_failure;
#[path = "../sourcegen/android_test_fixes.rs"]
mod android_test_fixes;
#[path = "../sourcegen/android_timer_action.rs"]
mod android_timer_action;
#[path = "../sourcegen/android_timer_latency.rs"]
mod android_timer_latency;
#[path = "../sourcegen/android_timer_layout.rs"]
mod android_timer_layout;
#[path = "../sourcegen/android_ui_localization.rs"]
mod android_ui_localization;
#[path = "../sourcegen/android_update_history_source.rs"]
mod android_update_history_source;
#[path = "../sourcegen/diagnostics_collection_backend.rs"]
mod diagnostics_collection_backend;
#[path = "../sourcegen/diagnostics_export_backend.rs"]
mod diagnostics_export_backend;
#[path = "../sourcegen/diagnostics_export_ui.rs"]
mod diagnostics_export_ui;
#[path = "../sourcegen/finance_money_source.rs"]
mod finance_money_source;
#[path = "../sourcegen/finance_risk_v2_ui_source.rs"]
mod finance_risk_v2_ui_source;
#[path = "../sourcegen/kotlin_android_test_sources.rs"]
mod kotlin_android_test_sources;
#[path = "../sourcegen/kotlin_sources.rs"]
mod kotlin_sources;
#[path = "../sourcegen/legal_report_sync_source.rs"]
mod legal_report_sync_source;
#[path = "../sourcegen/legal_risk_ui_source.rs"]
mod legal_risk_ui_source;
#[path = "../sourcegen/legal_storage_source.rs"]
mod legal_storage_source;
#[path = "../sourcegen/my_account_source.rs"]
mod my_account_source;
#[path = "../sourcegen/remote_sync_source.rs"]
mod remote_sync_source;

fn main() {
    if let Err(error) = run() {
        eprintln!("source generation failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let output_root = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing output directory"))?;
    let android_test_output_root = env::args_os().nth(2).map(PathBuf::from);
    let unit_test_output_root = env::args_os().nth(3).map(PathBuf::from);
    let rendezvous_identity = gridtimer_native::sync_rendezvous::ensure_local_rendezvous_identity()
        .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?;
    let bootstrap_public_sync_server_url = read_bootstrap_public_sync_server_url()?;

    if output_root.exists() {
        fs::remove_dir_all(&output_root)?;
    }
    fs::create_dir_all(&output_root)?;

    if let Some(unit_test_output_root) = unit_test_output_root {
        write_source(
            &unit_test_output_root,
            android_knowledge_navigation::TEST_PATH,
            android_knowledge_navigation::TEST_CONTENTS,
        )?;
        write_source(
            &unit_test_output_root,
            android_knowledge_navigation::SNAPSHOT_TEST_PATH,
            android_knowledge_navigation::SNAPSHOT_TEST_CONTENTS,
        )?;
        write_source(
            &unit_test_output_root,
            android_document_markdown::TEST_PATH,
            android_document_markdown::TEST_CONTENTS,
        )?;
        write_source(
            &unit_test_output_root,
            android_finance_workspace::TEST_PATH,
            android_finance_workspace::TEST_CONTENTS,
        )?;
        write_source(
            &unit_test_output_root,
            android_document_caret::TEST_PATH,
            android_document_caret::TEST_CONTENTS,
        )?;
        write_source(
            &unit_test_output_root,
            android_ai_answer_ui::TEST_PATH,
            android_ai_answer_ui::TEST_CONTENTS,
        )?;
        write_source(
            &unit_test_output_root,
            android_ai_workflow::TEST_PATH,
            android_ai_workflow::TEST_CONTENTS,
        )?;
        write_source(
            &unit_test_output_root,
            android_sync_failure::TEST_PATH,
            android_sync_failure::TEST_CONTENTS,
        )?;
        write_source(
            &unit_test_output_root,
            my_account_source::AI_TEST_PATH,
            my_account_source::AI_TEST_CONTENTS,
        )?;
        write_source(
            &unit_test_output_root,
            finance_money_source::TEST_PATH,
            finance_money_source::TEST_CONTENTS,
        )?;
        write_source(
            &unit_test_output_root,
            android_jvm_test_sources::PATH,
            &android_jvm_test_sources::render(),
        )?;
        write_source(
            &unit_test_output_root,
            android_canvas_ui::TEST_PATH,
            android_canvas_ui::TEST_CONTENTS,
        )?;
        write_source(
            &unit_test_output_root,
            android_note_list_performance::TEST_PATH,
            android_note_list_performance::TEST_CONTENTS,
        )?;
        write_source(
            &unit_test_output_root,
            android_note_save_queue::TEST_PATH,
            android_note_save_queue::TEST_CONTENTS,
        )?;
        write_source(
            &unit_test_output_root,
            android_note_collection_recovery::TEST_PATH,
            android_note_collection_recovery::TEST_CONTENTS,
        )?;
        write_source(
            &unit_test_output_root,
            android_knowledge_filters::TEST_PATH,
            android_knowledge_filters::TEST_CONTENTS,
        )?;
        write_source(
            &unit_test_output_root,
            android_startup_decode::TEST_PATH,
            android_startup_decode::TEST_CONTENTS,
        )?;
        write_source(
            &unit_test_output_root,
            legal_risk_ui_source::TEST_PATH,
            legal_risk_ui_source::TEST_CONTENTS,
        )?;
    }

    write_source(
        &output_root,
        android_document_caret::POLICY_PATH,
        android_document_caret::POLICY_CONTENTS,
    )?;
    write_source(
        &output_root,
        android_document_caret::UI_PATH,
        android_document_caret::UI_CONTENTS,
    )?;

    write_source(
        &output_root,
        android_knowledge_navigation::INDEX_PATH,
        android_knowledge_navigation::INDEX_CONTENTS,
    )?;
    write_source(
        &output_root,
        android_knowledge_navigation::UI_PATH,
        android_knowledge_navigation::UI_CONTENTS,
    )?;

    for source in kotlin_sources::SOURCES {
        let rendered = render_source(
            source.contents,
            &rendezvous_identity,
            &bootstrap_public_sync_server_url,
        )?;
        let rendered = android_performance_override::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_responsiveness::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = diagnostics_export_backend::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = diagnostics_collection_backend::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = diagnostics_export_ui::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_snapshot_memory::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_backup_availability::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_timer_latency::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_note_latency::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_test_fixes::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_startup_loading::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_privacy_generation::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_timer_action::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_timer_layout::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_note_background::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_local_startup::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_note_save_performance::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_note_save_queue::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_device_timer_sync::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_knowledge_compat::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_canvas_ui::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_rest_bell::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_startup_stream::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_history_storage::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_startup_decode::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_note_list_performance::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_ui_localization::render(source.path, &rendered);
        let rendered = android_legal_integration::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_note_collection_recovery::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_knowledge_filters::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_ai_workflow::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = android_sync_failure::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let rendered = legal_report_sync_source::render(source.path, &rendered)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        write_source(&output_root, source.path, &rendered)?;
    }

    write_source(
        &output_root,
        android_note_collection_recovery::HELPER_PATH,
        android_note_collection_recovery::HELPER_CONTENTS,
    )?;
    write_source(
        &output_root,
        android_knowledge_filters::PATH,
        android_knowledge_filters::CONTENTS,
    )?;
    write_source(
        &output_root,
        android_note_background::PATH,
        android_note_background::CONTENTS,
    )?;

    write_source(
        &output_root,
        diagnostics_export_ui::PATH,
        &android_ui_localization::render(
            diagnostics_export_ui::PATH,
            diagnostics_export_ui::CONTENTS,
        ),
    )?;

    write_source(
        &output_root,
        my_account_source::PATH,
        my_account_source::CONTENTS,
    )?;
    write_source(
        &output_root,
        android_ai_workflow::PATH,
        android_ai_workflow::CONTENTS,
    )?;
    write_source(
        &output_root,
        android_ai_answer_ui::PATH,
        android_ai_answer_ui::CONTENTS,
    )?;
    write_source(
        &output_root,
        android_ai_answer_ui::BOUNDARY_PATH,
        android_ai_answer_ui::BOUNDARY_CONTENTS,
    )?;
    write_source(
        &output_root,
        my_account_source::AI_CONTROLLER_PATH,
        my_account_source::AI_CONTROLLER_CONTENTS,
    )?;

    write_source(
        &output_root,
        finance_money_source::PATH,
        finance_money_source::CONTENTS,
    )?;

    write_source(
        &output_root,
        android_update_history_source::PATH,
        android_update_history_source::CONTENTS,
    )?;

    write_source(
        &output_root,
        legal_storage_source::PATH,
        legal_storage_source::CONTENTS,
    )?;

    write_source(
        &output_root,
        legal_report_sync_source::PATH,
        legal_report_sync_source::CONTENTS,
    )?;

    write_source(
        &output_root,
        legal_risk_ui_source::PATH,
        legal_risk_ui_source::CONTENTS,
    )?;

    write_source(
        &output_root,
        finance_risk_v2_ui_source::PATH,
        finance_risk_v2_ui_source::CONTENTS,
    )?;

    write_source(
        &output_root,
        remote_sync_source::PATH,
        &render_source(
            remote_sync_source::CONTENTS,
            &rendezvous_identity,
            &bootstrap_public_sync_server_url,
        )?,
    )?;

    write_source(
        &output_root,
        android_knowledge_compat::PATH,
        &android_ui_localization::render(
            android_knowledge_compat::PATH,
            android_knowledge_compat::CONTENTS,
        ),
    )?;

    for source in android_sources::SOURCES {
        write_source(&output_root, source.path, source.contents)?;
    }

    write_source(
        &output_root,
        android_canvas_ui::PATH,
        &android_ui_localization::render(android_canvas_ui::PATH, android_canvas_ui::CONTENTS),
    )?;

    write_source(
        &output_root,
        android_ui_localization::KOTLIN_PATH,
        android_ui_localization::KOTLIN_CONTENTS,
    )?;
    for &(path, contents) in android_ui_localization::RESOURCES {
        write_source(&output_root, path, contents)?;
    }

    if let Some(android_test_output_root) = android_test_output_root {
        if android_test_output_root.exists() {
            fs::remove_dir_all(&android_test_output_root)?;
        }
        fs::create_dir_all(&android_test_output_root)?;
        for source in kotlin_android_test_sources::SOURCES {
            write_source(
                &android_test_output_root,
                source.path,
                &render_source(
                    source.contents,
                    &rendezvous_identity,
                    &bootstrap_public_sync_server_url,
                )?,
            )?;
        }
    }

    Ok(())
}

fn render_source(
    contents: &str,
    identity: &gridtimer_native::sync_rendezvous::LocalRendezvousIdentity,
    bootstrap_public_sync_server_url: &str,
) -> io::Result<String> {
    let bootstrap_url_literal = kotlin_string_literal(bootstrap_public_sync_server_url);
    let rendered = contents
        .replace(
            "__GRIDTIMER_RENDEZVOUS_PUBLIC_KEY_BASE64__",
            &identity.public_key_base64,
        )
        .replace("__GRIDTIMER_RENDEZVOUS_TOPIC__", &identity.topic)
        .replace(
            "__GRIDTIMER_RENDEZVOUS_SYNC_PROTOCOL_VERSION__",
            &gridtimer_native::sync_core::SYNC_PROTOCOL_VERSION.to_string(),
        )
        .replace(
            "__GRIDTIMER_BOOTSTRAP_PUBLIC_SYNC_SERVER_URL_LITERAL__",
            &bootstrap_url_literal,
        );
    if rendered.contains("__GRIDTIMER_RENDEZVOUS_")
        || rendered.contains("__GRIDTIMER_BOOTSTRAP_PUBLIC_SYNC_SERVER_URL_")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unresolved sync discovery source placeholder",
        ));
    }
    Ok(rendered)
}

fn read_bootstrap_public_sync_server_url() -> io::Result<String> {
    let configured_path = env::var_os("GRID_TIMER_PUBLIC_SERVER_URL_FILE")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("LOCALAPPDATA").map(|local_app_data| {
                PathBuf::from(local_app_data)
                    .join("GridTimerSync")
                    .join("sync_public_server_url.txt")
            })
        });
    let Some(path) = configured_path else {
        return Ok(String::new());
    };
    if !path.is_file() {
        return Ok(String::new());
    }
    let raw = fs::read_to_string(&path)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    let normalized = gridtimer_native::sync_rendezvous::validate_https_server_url(trimmed)
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "invalid bootstrap public sync URL in {}: {error}",
                    path.display()
                ),
            )
        })?;
    if normalized != trimmed {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "bootstrap public sync URL in {} is not canonical",
                path.display()
            ),
        ));
    }
    Ok(normalized)
}

fn kotlin_string_literal(value: &str) -> String {
    let mut literal = String::with_capacity(value.len() + 2);
    literal.push('"');
    for character in value.chars() {
        match character {
            '\\' => literal.push_str("\\\\"),
            '"' => literal.push_str("\\\""),
            '$' => literal.push_str("\\$"),
            '\n' => literal.push_str("\\n"),
            '\r' => literal.push_str("\\r"),
            '\t' => literal.push_str("\\t"),
            value if value.is_control() => {
                use std::fmt::Write as _;
                write!(&mut literal, "\\u{:04x}", value as u32)
                    .expect("writing to a String cannot fail");
            }
            value => literal.push(value),
        }
    }
    literal.push('"');
    literal
}

fn write_source(output_root: &Path, relative_path: &str, contents: &str) -> io::Result<()> {
    let destination = source_destination(output_root, relative_path);

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }

    // Apply the legal workflow last, including the separately emitted screen and tests.
    let contents = android_legal_workflow::render(relative_path, contents)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let contents = android_document_caret::render(relative_path, &contents)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let contents = android_finance_workspace::render(relative_path, &contents)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let contents = android_document_markdown::render(relative_path, &contents)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let contents = android_document_block_alignment::render(relative_path, &contents)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let contents = android_knowledge_navigation::render(relative_path, &contents)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let contents = contents
        .replace("十倍率", "tenfold")
        .replace("grid_timer_data_", "tenfold_data_")
        .replace("grid_timer_log_", "tenfold_log_");
    fs::write(destination, contents.trim_start())
}

fn source_destination(output_root: &Path, relative_path: &str) -> PathBuf {
    relative_path
        .split('/')
        .fold(output_root.to_path_buf(), |path, segment| {
            path.join(segment)
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn android_update_history_starts_with_the_release_version() {
        let release_version = gridtimer_native::product_identity::ANDROID_APP_VERSION;
        assert_eq!(
            super::android_update_history_source::LATEST_VERSION,
            release_version
        );
        let history = super::android_update_history_source::CONTENTS
            .split("internal val androidUpdateHistory: List<AndroidUpdateEntry> = listOf(")
            .nth(1)
            .expect("generated Android history list");
        let first_version = history
            .split("version = \"")
            .nth(1)
            .and_then(|value| value.split('"').next())
            .expect("first Android history entry version");
        assert_eq!(first_version, release_version);
    }

    #[test]
    fn performance_override_is_applied_before_sources_are_written() {
        let generator = include_str!("gridtimer_sourcegen.rs");
        let base_sources = generator
            .find("for source in kotlin_sources::SOURCES")
            .expect("base Kotlin source loop");
        let override_render = generator
            .find("android_performance_override::render(source.path, &rendered)")
            .expect("checked Android performance override");
        let source_write = generator[override_render..]
            .find("write_source(&output_root, source.path, &rendered)")
            .map(|offset| override_render + offset)
            .expect("base Kotlin source write");
        assert!(base_sources < override_render);
        assert!(override_render < source_write);
    }
}
