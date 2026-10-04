// v2.23.2.6 - Include the bundled offline mathematical engine license.
// v2.22.38 - Use the tenfold application label in every Android locale.
// v2.22.21 - Reduce the launcher mark to a flat clock ring and two hands.
// Rust-owned Android manifest and resource templates emitted into Gradle's generated Android source set.
// Keep these strings exact, then let the generator materialize them under build/.

pub struct AndroidSource {
    pub path: &'static str,
    pub contents: &'static str,
}

pub const SOURCES: &[AndroidSource] = &[
    AndroidSource {
        path: "res/raw/katex_license.txt",
        contents: include_str!("../desktop/assets/katex_LICENSE.txt"),
    },
    AndroidSource {
        path: "AndroidManifest.xml",
        contents: r####"
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    xmlns:tools="http://schemas.android.com/tools">

    <uses-permission android:name="android.permission.INTERNET" />
    <uses-permission android:name="android.permission.ACCESS_NETWORK_STATE" />
    <uses-permission android:name="android.permission.CHANGE_NETWORK_STATE" />
    <uses-permission android:name="android.permission.CHANGE_WIFI_MULTICAST_STATE" />
    <uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
    <uses-permission
        android:name="android.permission.SCHEDULE_EXACT_ALARM"
        android:maxSdkVersion="32" />
    <uses-permission android:name="android.permission.USE_EXACT_ALARM" />
    <uses-permission android:name="android.permission.WAKE_LOCK" />
    <uses-permission android:name="android.permission.READ_CALL_LOG" />

    <uses-feature
        android:name="android.hardware.wifi"
        android:required="false" />

    <application
        android:name=".GridTimerApplication"
        android:allowBackup="true"
        android:dataExtractionRules="@xml/data_extraction_rules"
        android:enableOnBackInvokedCallback="false"
        android:fullBackupContent="@xml/backup_rules"
        android:icon="@mipmap/ic_launcher"
        android:label="@string/app_name"
        android:localeConfig="@xml/locales_config"
        android:roundIcon="@mipmap/ic_launcher_round"
        android:supportsRtl="true"
        android:theme="@style/Theme.GridTimer"
        android:usesCleartextTraffic="true">
        <meta-data
            android:name="com.xiaomi.xms.APP_ID"
            android:value="${xiaomiAppId}" />

        <meta-data
            android:name="com.xiaomi.xms.BUILD_TYPE_DEBUG"
            android:value="${xiaomiBuildTypeDebug}" />

        <provider
            android:name="androidx.startup.InitializationProvider"
            android:authorities="${applicationId}.androidx-startup"
            android:exported="false"
            tools:node="remove" />

        <receiver
            android:name="androidx.profileinstaller.ProfileInstallReceiver"
            tools:node="remove" />

        <service
            android:name="androidx.appcompat.app.AppLocalesMetadataHolderService"
            android:enabled="false"
            android:exported="false">
            <meta-data
                android:name="autoStoreLocales"
                android:value="true" />
        </service>

        <provider
            android:name="androidx.core.content.FileProvider"
            android:authorities="${applicationId}.fileprovider"
            android:exported="false"
            android:grantUriPermissions="true">
            <meta-data
                android:name="android.support.FILE_PROVIDER_PATHS"
                android:resource="@xml/file_paths" />
        </provider>

        <activity
            android:name=".MainActivity"
            android:exported="true"
            android:windowLayoutInDisplayCutoutMode="shortEdges"
            android:windowSoftInputMode="adjustResize"
            tools:targetApi="33">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />

                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>

        <receiver
            android:name=".notifications.TimerNotificationActionReceiver"
            android:exported="false" />
        <receiver
            android:name=".notifications.MicroBreakAlarmReceiver"
            android:exported="false" />
    </application>

</manifest>
"####,
    },
    AndroidSource {
        path: "res/drawable/ic_launcher_background.xml",
        contents: r####"
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="108dp"
    android:height="108dp"
    android:viewportWidth="108"
    android:viewportHeight="108">
    <path
        android:fillColor="#F4EEE6"
        android:pathData="M0,0h108v108h-108z" />
</vector>
"####,
    },
    AndroidSource {
        path: "res/drawable/ic_launcher_foreground.xml",
        contents: r####"
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="108dp"
    android:height="108dp"
    android:viewportWidth="108"
    android:viewportHeight="108">
    <path
        android:fillColor="#C7463A"
        android:fillType="evenOdd"
        android:pathData="M54,20a34,34 0,1 0,0 68a34,34 0,1 0,0 -68zM54,29a25,25 0,1 1,0 50a25,25 0,1 1,0 -50z" />
    <path
        android:fillColor="#292624"
        android:pathData="M51.5,37h5v18.4l12.2,7.1 -2.6,4.3 -14.6,-8.5z" />
    <path
        android:fillColor="#292624"
        android:pathData="M50,50a4,4 0,1 0,8 0a4,4 0,1 0,-8 0M49,11h10v9h-10z" />
</vector>
"####,
    },
    AndroidSource {
        path: "res/drawable/ic_launcher_monochrome.xml",
        contents: r####"
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="108dp"
    android:height="108dp"
    android:viewportWidth="108"
    android:viewportHeight="108">
    <path
        android:fillColor="#000000"
        android:fillType="evenOdd"
        android:pathData="M54,20a34,34 0,1 0,0 68a34,34 0,1 0,0 -68zM54,29a25,25 0,1 1,0 50a25,25 0,1 1,0 -50z" />
    <path
        android:fillColor="#000000"
        android:pathData="M52,36h4v18.5l11.2,7.1 -2.1,3.4L52,57.3V36z" />
    <path
        android:fillColor="#000000"
        android:pathData="M49.5,49.5a4.5,4.5 0 1,0 9,0a4.5,4.5 0 1,0 -9,0" />
</vector>
"####,
    },
    AndroidSource {
        path: "res/drawable/ic_notification_pause.xml",
        contents: r####"
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="24dp"
    android:height="24dp"
    android:viewportWidth="24"
    android:viewportHeight="24">
    <path
        android:fillColor="#FFFFFFFF"
        android:pathData="M7,5h3v14H7z" />
    <path
        android:fillColor="#FFFFFFFF"
        android:pathData="M14,5h3v14h-3z" />
</vector>
"####,
    },
    AndroidSource {
        path: "res/drawable/ic_notification_timer.xml",
        contents: r####"
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="24dp"
    android:height="24dp"
    android:viewportWidth="24"
    android:viewportHeight="24">
    <path
        android:fillColor="#FFFFFFFF"
        android:pathData="M9,1h6v2H9zM12,8c-2.21,0 -4,1.79 -4,4s1.79,4 4,4 4,-1.79 4,-4h-4zM18.03,7.39l1.41,-1.41 -1.42,-1.42 -1.41,1.41C15.3,5.36 13.71,5 12,5 7.03,5 3,9.03 3,14s4.03,9 9,9 9,-4.03 9,-9c0,-2.32 -0.88,-4.43 -2.97,-6.61z" />
</vector>
"####,
    },
    AndroidSource {
        path: "res/layout/timer_focus_status_bar.xml",
        contents: r####"
<?xml version="1.0" encoding="utf-8"?>
<LinearLayout xmlns:android="http://schemas.android.com/apk/res/android"
    android:layout_width="wrap_content"
    android:layout_height="wrap_content"
    android:gravity="center_vertical"
    android:orientation="horizontal"
    android:paddingStart="4dp"
    android:paddingEnd="4dp">

    <ImageView
        android:id="@+id/timer_status_icon"
        android:layout_width="14dp"
        android:layout_height="14dp"
        android:contentDescription="@null" />

    <TextView
        android:id="@+id/timer_status_label"
        android:layout_width="wrap_content"
        android:layout_height="wrap_content"
        android:layout_marginStart="4dp"
        android:ellipsize="end"
        android:maxLines="1"
        android:textSize="11sp" />

    <Chronometer
        android:id="@+id/timer_status_chronometer"
        android:layout_width="wrap_content"
        android:layout_height="wrap_content"
        android:layout_marginStart="4dp"
        android:format="%s"
        android:singleLine="true"
        android:textSize="11sp" />

</LinearLayout>
"####,
    },
    AndroidSource {
        path: "res/mipmap-anydpi/ic_launcher.xml",
        contents: r####"
<?xml version="1.0" encoding="utf-8"?>
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
    <background android:drawable="@drawable/ic_launcher_background" />
    <foreground android:drawable="@drawable/ic_launcher_foreground" />
    <monochrome android:drawable="@drawable/ic_launcher_monochrome" />
</adaptive-icon>
"####,
    },
    AndroidSource {
        path: "res/mipmap-anydpi/ic_launcher_round.xml",
        contents: r####"
<?xml version="1.0" encoding="utf-8"?>
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
    <background android:drawable="@drawable/ic_launcher_background" />
    <foreground android:drawable="@drawable/ic_launcher_foreground" />
    <monochrome android:drawable="@drawable/ic_launcher_monochrome" />
</adaptive-icon>
"####,
    },
    AndroidSource {
        path: "res/values/notification_strings.xml",
        contents: r####"
<resources>
    <string name="timer_live_update_channel_name">Timer Live Updates</string>
    <string name="timer_live_update_channel_description">Keeps active timers visible in the notification shade and HyperOS top area.</string>
    <string name="micro_break_channel_name">Micro-break Alerts</string>
    <string name="micro_break_channel_description">Rings when focus starts, when a 15-second break starts, and when focus resumes.</string>
    <string name="xiaomi_timer_action_open">打开</string>
    <string name="xiaomi_timer_action_pause">暂停</string>
    <string name="xiaomi_timer_action_pause_all">暂停全部</string>
</resources>
"####,
    },
    AndroidSource {
        path: "res/values/strings.xml",
        contents: r####"
<resources>
    <string name="app_name">tenfold</string>
</resources>
"####,
    },
    AndroidSource {
        path: "res/values/themes.xml",
        contents: r####"
<resources xmlns:tools="http://schemas.android.com/tools">
    <style name="Theme.GridTimer.Base" parent="Theme.AppCompat.DayNight.NoActionBar">
        <item name="android:windowBackground">@android:color/transparent</item>
        <item name="android:statusBarColor">@android:color/transparent</item>
        <item name="android:navigationBarColor">@android:color/transparent</item>
        <item name="android:windowTranslucentStatus">false</item>
        <item name="android:windowTranslucentNavigation">false</item>
        <item name="android:forceDarkAllowed" tools:targetApi="q">false</item>
    </style>
    <style name="Theme.GridTimer" parent="Theme.GridTimer.Base" />
</resources>
"####,
    },
    AndroidSource {
        path: "res/values-en/notification_strings.xml",
        contents: r####"
<resources>
    <string name="timer_live_update_channel_name">Timer Live Updates</string>
    <string name="timer_live_update_channel_description">Keeps active timers visible in the notification shade and HyperOS top area.</string>
    <string name="micro_break_channel_name">Micro-break Alerts</string>
    <string name="micro_break_channel_description">Rings when focus starts, when a 15-second break starts, and when focus resumes.</string>
    <string name="xiaomi_timer_action_open">Open</string>
    <string name="xiaomi_timer_action_pause">Pause</string>
    <string name="xiaomi_timer_action_pause_all">Pause All</string>
</resources>
"####,
    },
    AndroidSource {
        path: "res/values-en/strings.xml",
        contents: r####"
<resources>
    <string name="app_name">tenfold</string>
</resources>
"####,
    },
    AndroidSource {
        path: "res/values-ja/notification_strings.xml",
        contents: r####"
<resources>
    <string name="timer_live_update_channel_name">タイマーのライブ表示</string>
    <string name="timer_live_update_channel_description">進行中のタイマーを通知欄と HyperOS 上部エリアに表示します。</string>
    <string name="micro_break_channel_name">小休憩アラート</string>
    <string name="micro_break_channel_description">集中開始、15 秒休憩開始、集中再開のタイミングで通知します。</string>
    <string name="xiaomi_timer_action_open">開く</string>
    <string name="xiaomi_timer_action_pause">一時停止</string>
    <string name="xiaomi_timer_action_pause_all">すべて停止</string>
</resources>
"####,
    },
    AndroidSource {
        path: "res/values-ja/strings.xml",
        contents: r####"
<resources>
    <string name="app_name">tenfold</string>
</resources>
"####,
    },
    AndroidSource {
        path: "res/values-v27/themes.xml",
        contents: r####"
<resources>
    <style name="Theme.GridTimer" parent="Theme.GridTimer.Base">
        <item name="android:windowLayoutInDisplayCutoutMode">shortEdges</item>
    </style>
</resources>
"####,
    },
    AndroidSource {
        path: "res/xml/backup_rules.xml",
        contents: r####"
<?xml version="1.0" encoding="utf-8"?>
<full-backup-content>
    <include domain="file" path="timer_state.json" requireFlags="clientSideEncryption" />
    <include domain="file" path="timer_state_backup.json" requireFlags="clientSideEncryption" />
    <include domain="file" path="state_workspaces/" requireFlags="clientSideEncryption" />
    <include domain="file" path="note_media/" requireFlags="clientSideEncryption" />
    <include domain="database" path="app_state.db" requireFlags="clientSideEncryption" />
    <include domain="database" path="app_state.db-wal" requireFlags="clientSideEncryption" />
</full-backup-content>
"####,
    },
    AndroidSource {
        path: "res/xml/data_extraction_rules.xml",
        contents: r####"
<?xml version="1.0" encoding="utf-8"?>
<data-extraction-rules>
    <cloud-backup disableIfNoEncryptionCapabilities="true">
        <include domain="file" path="timer_state.json" />
        <include domain="file" path="timer_state_backup.json" />
        <include domain="file" path="state_workspaces/" />
        <include domain="file" path="note_media/" />
        <include domain="database" path="app_state.db" />
        <include domain="database" path="app_state.db-wal" />
    </cloud-backup>
    <device-transfer>
        <include domain="file" path="timer_state.json" />
        <include domain="file" path="timer_state_backup.json" />
        <include domain="file" path="state_workspaces/" />
        <include domain="file" path="note_media/" />
        <include domain="database" path="app_state.db" />
        <include domain="database" path="app_state.db-wal" />
    </device-transfer>
</data-extraction-rules>
"####,
    },
    AndroidSource {
        path: "res/xml/file_paths.xml",
        contents: r####"
<?xml version="1.0" encoding="utf-8"?>
<paths>
    <cache-path
        name="shared_exports"
        path="shared_exports/" />
    <cache-path
        name="note_shares"
        path="note_shares/" />
    <cache-path
        name="note_capture"
        path="note_capture/" />
    <files-path
        name="finance_backups"
        path="finance_backups/" />
</paths>
"####,
    },
    AndroidSource {
        path: "res/xml/locales_config.xml",
        contents: r####"
<?xml version="1.0" encoding="utf-8"?>
<locale-config xmlns:android="http://schemas.android.com/apk/res/android">
    <locale android:name="zh-CN" />
    <locale android:name="en-US" />
    <locale android:name="hi-IN" />
    <locale android:name="es-ES" />
    <locale android:name="ar" />
    <locale android:name="fr-FR" />
    <locale android:name="bn-BD" />
    <locale android:name="pt-BR" />
    <locale android:name="id-ID" />
    <locale android:name="ur-PK" />
    <locale android:name="ja-JP" />
</locale-config>
"####,
    },
];

#[cfg(test)]
mod tests {
    use super::SOURCES;

    fn source(path: &str) -> &'static str {
        SOURCES
            .iter()
            .find(|source| source.path == path)
            .map(|source| source.contents)
            .unwrap_or_else(|| panic!("missing generated source: {path}"))
    }

    #[test]
    fn android_backup_requires_encryption_and_keeps_device_transfer() {
        for path in [
            "res/xml/backup_rules.xml",
            "res/xml/data_extraction_rules.xml",
        ] {
            let rules = source(path);
            assert!(rules.contains("path=\"state_workspaces/\""));
            assert!(rules.contains("path=\"note_media/\""));
            assert!(rules.contains("domain=\"database\" path=\"app_state.db\""));
        }
        assert!(
            source("res/xml/backup_rules.xml").contains("requireFlags=\"clientSideEncryption\"")
        );
        assert!(source("res/xml/data_extraction_rules.xml")
            .contains("cloud-backup disableIfNoEncryptionCapabilities=\"true\""));
        let manifest = source("AndroidManifest.xml");
        assert!(!manifest.contains("android.permission.RECORD_AUDIO"));
        assert!(!manifest.contains("android.permission.READ_CONTACTS"));
        assert!(!manifest.contains("android.permission.WRITE_EXTERNAL_STORAGE"));
        assert!(manifest.contains("android:maxSdkVersion=\"32\""));
    }
}
