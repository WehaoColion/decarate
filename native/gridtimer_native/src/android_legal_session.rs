//! Android-only, one-shot legal scan sessions. Raw scan material stays in memory.

use crate::legal_scan::{self, LegalScanPrepared};
use crate::legal_sources::verified_laws;
use jni::objects::{JClass, JString};
use jni::sys::{jlong, jstring};
use jni::JNIEnv;
use rand::RngCore;
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

struct ScanSession {
    prepared: LegalScanPrepared,
    cancelled: AtomicBool,
    running: AtomicBool,
    completed_calls: AtomicUsize,
}

static SESSIONS: OnceLock<Mutex<HashMap<String, Arc<ScanSession>>>> = OnceLock::new();

fn sessions() -> &'static Mutex<HashMap<String, Arc<ScanSession>>> {
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn session(id: &str) -> Option<Arc<ScanSession>> {
    sessions().lock().ok()?.get(id).cloned()
}

fn input(env: &mut JNIEnv, value: JString) -> Option<String> {
    env.get_string(&value).ok().map(Into::into)
}

fn output(env: &mut JNIEnv, value: &str) -> jstring {
    env.new_string(value)
        .map(JString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

fn error(env: &mut JNIEnv, message: &str) -> jstring {
    output(env, &json!({"error": message}).to_string())
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativePrepareLegalScan(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    workspace_id: JString,
    captured_at: jlong,
    unlocked_notes_json: JString,
    attachments_json: JString,
) -> jstring {
    let Some(app_data_json) = input(&mut env, app_data_json) else {
        return error(&mut env, "应用数据无法读取");
    };
    let Some(workspace_id) = input(&mut env, workspace_id) else {
        return error(&mut env, "工作区无法读取");
    };
    let Some(unlocked_notes_json) = input(&mut env, unlocked_notes_json) else {
        return error(&mut env, "解锁资料无法读取");
    };
    let Some(attachments_json) = input(&mut env, attachments_json) else {
        return error(&mut env, "附件清单无法读取");
    };
    let prepared = match legal_scan::prepare_scan(
        &app_data_json,
        &workspace_id,
        captured_at,
        &unlocked_notes_json,
        &attachments_json,
    ) {
        Ok(prepared) => prepared,
        Err(reason) => return error(&mut env, &reason),
    };
    let mut id_bytes = [0_u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut id_bytes);
    let scan_id = id_bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let manifest = prepared.manifest.clone();
    let Ok(mut guard) = sessions().lock() else {
        return error(&mut env, "扫描会话暂不可用");
    };
    if guard.len() >= 4 {
        return error(&mut env, "已有过多待关闭扫描，请先关闭旧报告页面");
    }
    guard.insert(
        scan_id.clone(),
        Arc::new(ScanSession {
            prepared,
            cancelled: AtomicBool::new(false),
            running: AtomicBool::new(false),
            completed_calls: AtomicUsize::new(0),
        }),
    );
    output(
        &mut env,
        &json!({"scanId": scan_id, "manifest": manifest}).to_string(),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeLegalScanEvidenceIndexJson(
    mut env: JNIEnv,
    _class: JClass,
    scan_id: JString,
) -> jstring {
    let Some(scan_id) = input(&mut env, scan_id) else {
        return std::ptr::null_mut();
    };
    let Some(session) = session(&scan_id) else {
        return std::ptr::null_mut();
    };
    let items = session
        .prepared
        .evidence
        .iter()
        .map(|evidence| {
            json!({
                "id": evidence.id,
                "category": evidence.category,
                "sourcePath": evidence.source_path,
                "title": evidence.title,
                "hasImage": evidence.image_data_url.is_some(),
            })
        })
        .collect::<Vec<_>>();
    output(&mut env, &json!(items).to_string())
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeLegalScanEvidenceJson(
    mut env: JNIEnv,
    _class: JClass,
    scan_id: JString,
    evidence_id: JString,
) -> jstring {
    let Some(scan_id) = input(&mut env, scan_id) else {
        return std::ptr::null_mut();
    };
    let Some(evidence_id) = input(&mut env, evidence_id) else {
        return std::ptr::null_mut();
    };
    let Some(session) = session(&scan_id) else {
        return std::ptr::null_mut();
    };
    let Some(evidence) = session
        .prepared
        .evidence
        .iter()
        .find(|e| e.id == evidence_id)
    else {
        return std::ptr::null_mut();
    };
    output(
        &mut env,
        &json!({
            "id": evidence.id,
            "category": evidence.category,
            "sourcePath": evidence.source_path,
            "title": evidence.title,
            "text": evidence.text,
            "hasImage": evidence.image_data_url.is_some(),
        })
        .to_string(),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeRunLegalScan(
    mut env: JNIEnv,
    _class: JClass,
    scan_id: JString,
    api_key: JString,
    base_url: JString,
    model: JString,
) -> jstring {
    let Some(scan_id) = input(&mut env, scan_id) else {
        return std::ptr::null_mut();
    };
    let Some(api_key) = input(&mut env, api_key) else {
        return std::ptr::null_mut();
    };
    let Some(base_url) = input(&mut env, base_url) else {
        return std::ptr::null_mut();
    };
    let Some(model) = input(&mut env, model) else {
        return std::ptr::null_mut();
    };
    let Some(session) = session(&scan_id) else {
        return std::ptr::null_mut();
    };
    if session
        .running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return std::ptr::null_mut();
    }
    let report = legal_scan::run_scan(
        &session.prepared,
        &api_key,
        &base_url,
        &model,
        &verified_laws(),
        |completed, _total| {
            session.completed_calls.store(completed, Ordering::Release);
            !session.cancelled.load(Ordering::Acquire)
        },
    );
    session.running.store(false, Ordering::Release);
    match serde_json::to_string(&report) {
        Ok(value) => output(&mut env, &value),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeCancelLegalScan(
    mut env: JNIEnv,
    _class: JClass,
    scan_id: JString,
) {
    if let Some(scan_id) = input(&mut env, scan_id) {
        if let Some(session) = session(&scan_id) {
            session.cancelled.store(true, Ordering::Release);
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeCloseLegalScan(
    mut env: JNIEnv,
    _class: JClass,
    scan_id: JString,
) {
    if let Some(scan_id) = input(&mut env, scan_id) {
        if let Ok(mut guard) = sessions().lock() {
            if let Some(session) = guard.remove(&scan_id) {
                session.cancelled.store(true, Ordering::Release);
            }
        }
    }
}
