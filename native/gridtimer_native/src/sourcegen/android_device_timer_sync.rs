// v2.22.46 - Apply account data without importing another device's timer controls.
fn replace(source: &mut String, old: &str, new: &str) -> Result<(), String> {
    if source.matches(old).count() != 1 {
        return Err(format!("device timer sync anchor is not unique: {old}"));
    }
    *source = source.replacen(old, new, 1);
    Ok(())
}

pub fn render(path: &str, base: &str) -> Result<String, String> {
    let mut source = base.to_owned();
    match path {
        "com/ofairyo/gridtimer/core/NativeOptimizerBridge.kt" => {
            replace(
                &mut source,
                "    fun mergeSyncAppDataJson(",
                &format!("{BRIDGE}    fun mergeSyncAppDataJson("),
            )?;
            replace(&mut source, "    @JvmStatic\n    private external fun nativeMergeSyncAppDataJson(",
                "    @JvmStatic\n    private external fun nativeLocalizeTimerSyncAppDataJson(\n        incoming: String,\n        local: String,\n        now: Long\n    ): String?\n\n    @JvmStatic\n    private external fun nativeMergeSyncAppDataJson(")?;
        }
        "com/ofairyo/gridtimer/data/TimerRepository.kt" => {
            replace(
                &mut source,
                "            keptNewerLocalData = !effectiveForceReplace && applied != received",
                APPLY,
            )?;
        }
        _ => {}
    }
    Ok(source)
}

const BRIDGE: &str = r####"    fun localizeTimerSyncAppDataJson(incoming: String, local: String, now: Long): String? {
        if (!nativeAvailable) return null
        return runCatching { nativeLocalizeTimerSyncAppDataJson(incoming, local, now) }.getOrNull()
    }

"####;

const APPLY: &str = r####"            // Read the latest run while holding the workspace write lock.
            val localizedTimerJson = NativeOptimizerBridge.localizeTimerSyncAppDataJson(
                incoming = strictPersistedJson.encodeToString(applied),
                local = if (recoveringMissingWorkspace) "{}" else strictPersistedJson.encodeToString(_appData.value),
                now = now()
            ) ?: return@withLock false
            applied = runCatching {
                strictPersistedJson.decodeFromString<AppData>(localizedTimerJson)
            }.getOrNull() ?: return@withLock false
            keptNewerLocalData = !effectiveForceReplace && applied != received"####;
