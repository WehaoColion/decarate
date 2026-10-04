//! Android's explicit, synthetic AI connection test. No workspace state is read.

use jni::objects::{JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;
use zeroize::Zeroizing;

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
