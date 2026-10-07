//! Android's explicit, synthetic AI connection test. No workspace state is read.

use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use zeroize::Zeroizing;

static AGENT_RUNS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

fn agent_runs() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    AGENT_RUNS.get_or_init(|| Mutex::new(HashMap::new()))
}

struct AgentRunGuard {
    task_id: String,
    cancelled: Arc<AtomicBool>,
}

impl Drop for AgentRunGuard {
    fn drop(&mut self) {
        if let Ok(mut runs) = agent_runs().lock() {
            if runs
                .get(&self.task_id)
                .is_some_and(|active| Arc::ptr_eq(active, &self.cancelled))
            {
                runs.remove(&self.task_id);
            }
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeRunAndroidKnowledgeAgent(
    mut env: JNIEnv,
    _class: JClass,
    api_key: JString,
    base_url: JString,
    model: JString,
    scope_json: JString,
    task_id: JString,
) -> jstring {
    let Ok(api_key) = env
        .get_string(&api_key)
        .map(|value| Zeroizing::new(String::from(value)))
    else {
        return std::ptr::null_mut();
    };
    let Ok(base_url) = env.get_string(&base_url).map(String::from) else {
        return std::ptr::null_mut();
    };
    let Ok(model) = env.get_string(&model).map(String::from) else {
        return std::ptr::null_mut();
    };
    let Ok(scope_json) = env.get_string(&scope_json).map(String::from) else {
        return std::ptr::null_mut();
    };
    let Ok(task_id) = env.get_string(&task_id).map(String::from) else {
        return std::ptr::null_mut();
    };
    if task_id.trim().is_empty() || task_id.len() > 100 {
        return std::ptr::null_mut();
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let guard = AgentRunGuard {
        task_id: task_id.clone(),
        cancelled: Arc::clone(&cancelled),
    };
    {
        let Ok(mut runs) = agent_runs().lock() else {
            return std::ptr::null_mut();
        };
        if runs.contains_key(&task_id) {
            return std::ptr::null_mut();
        }
        runs.insert(task_id, Arc::clone(&cancelled));
    }
    let result = crate::ai_client::run_android_knowledge_agent(
        &api_key,
        &base_url,
        &model,
        &scope_json,
        &cancelled,
    );
    drop(guard);
    let Ok(json) = serde_json::to_string(&result) else {
        return std::ptr::null_mut();
    };
    env.new_string(json)
        .map(jni::objects::JString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeCancelAndroidKnowledgeAgent(
    mut env: JNIEnv,
    _class: JClass,
    task_id: JString,
) -> jboolean {
    let Ok(task_id) = env.get_string(&task_id).map(String::from) else {
        return JNI_FALSE;
    };
    let Ok(runs) = agent_runs().lock() else {
        return JNI_FALSE;
    };
    let Some(cancelled) = runs.get(&task_id) else {
        return JNI_FALSE;
    };
    cancelled.store(true, Ordering::Release);
    JNI_TRUE
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeTestAiConnection(
    mut env: JNIEnv,
    _class: JClass,
    api_key: JString,
    base_url: JString,
    model: JString,
) -> jstring {
    let Ok(api_key) = env
        .get_string(&api_key)
        .map(|value| Zeroizing::new(String::from(value)))
    else {
        return std::ptr::null_mut();
    };
    let Ok(base_url) = env.get_string(&base_url).map(String::from) else {
        return std::ptr::null_mut();
    };
    let Ok(model) = env.get_string(&model).map(String::from) else {
        return std::ptr::null_mut();
    };
    let result = crate::ai_client::test_connection(&api_key, &base_url, &model);
    let Ok(json) = serde_json::to_string(&result) else {
        return std::ptr::null_mut();
    };
    env.new_string(json)
        .map(JString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}
