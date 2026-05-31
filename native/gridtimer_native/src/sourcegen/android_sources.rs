// Rust-owned Android manifest and resource templates emitted into Gradle's generated Android source set.
// Keep these strings exact, then let the generator materialize them under build/.

pub struct AndroidSource {
    pub path: &'static str,
    pub contents: &'static str,
}

pub const SOURCES: &[AndroidSource] = &[
    AndroidSource {
        path: "AndroidManifest.xml",
        contents: r####"
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    xmlns:tools="http://schemas.android.com/tools">

    <uses-permission android:name="android.permission.INTERNET" />
    <uses-permission android:name="android.permission.ACCESS_NETWORK_STATE" />
    <uses-permission android:name="android.permission.CHANGE_NETWORK_STATE" />
    <uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
    <uses-permission android:name="android.permission.RECORD_AUDIO" />
    <uses-permission android:name="android.permission.READ_CONTACTS" />
    <uses-permission android:name="android.permission.READ_CALL_LOG" />
    <uses-permission
        android:name="android.permission.WRITE_EXTERNAL_STORAGE"
        android:maxSdkVersion="28" />

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
        android:fillColor="#F2EADD"
        android:pathData="M0,0h108v108h-108z" />
    <path
        android:fillColor="#FCF9F3"
        android:pathData="M18,12h72a14,14 0 0 1 14,14v56a26,26 0 0 1 -26,26H18a14,14 0 0 1 -14,-14V26a14,14 0 0 1 14,-14z" />
    <path
        android:fillColor="#FFFFFF"
        android:fillAlpha="0.72"
        android:pathData="M13,18c10,-8 24,-12 41,-12 16.5,0 30.8,3.3 42,10 -8,-2.2 -16.6,-3.3 -25.7,-3.3 -20,0 -37.9,5.8 -51.3,15.3L13,18z" />
    <path
        android:fillColor="#E1D5C4"
        android:pathData="M4,74c11.3,9.2 28.7,14.8 50,14.8 21.4,0 38.8,-5.6 50,-14.8V108H4V74z" />
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
        android:fillColor="#92261F"
        android:pathData="M54,10C29.7,10 10,29.7 10,54s19.7,44 44,44 44,-19.7 44,-44S78.3,10 54,10z" />
    <path
        android:fillColor="#D65346"
        android:pathData="M54,14C31.9,14 14,31.9 14,54s17.9,40 40,40 40,-17.9 40,-40S76.1,14 54,14z" />
    <path
        android:fillColor="#FFFFFF"
        android:fillAlpha="0.26"
        android:pathData="M32,17c6.1,-2.1 12.7,-3.3 19.6,-3.3 18.7,0 34.7,8.3 42.3,24.6 -7.5,-8 -18.1,-12.7 -30,-12.7 -12.3,0 -23.1,5 -31,13.1L32,17z" />
    <path
        android:fillColor="#F7F2E9"
        android:pathData="M54,24C37.4,24 24,37.4 24,54s13.4,30 30,30 30,-13.4 30,-30S70.6,24 54,24z" />
    <path
        android:fillColor="#E4DBCE"
        android:pathData="M54,28C39.6,28 28,39.6 28,54s11.6,26 26,26 26,-11.6 26,-26S68.4,28 54,28z" />
    <path
        android:fillColor="#FFFFFF"
        android:fillAlpha="0.65"
        android:pathData="M39,34c4.1,-3.8 9.7,-6.1 15.9,-6.1 10.4,0 19.4,6.4 23.3,15.5 -4.2,-4.9 -10.5,-7.9 -17.5,-7.9 -6.4,0 -12.2,2.5 -16.5,6.6L39,34z" />
    <path
        android:fillColor="#2B2B2B"
        android:pathData="M52,36h4v18.8l11.4,7.2 -2.4,3.6L52,57.6V36z" />
    <path
        android:fillColor="#C0392B"
        android:pathData="M53,54h2v14h-2z" />
    <path
        android:fillColor="#2B2B2B"
        android:pathData="M49,49a5,5 0 1,0 10,0a5,5 0 1,0 -10,0" />
    <path
        android:fillColor="#C0392B"
        android:pathData="M48,8h12v9H48z" />
    <path
        android:fillColor="#2B2B2B"
        android:pathData="M35,16l-8,8 4,4 8,-8zM73,16l4,4 -8,8 -4,-4z" />
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
        android:pathData="M54,15c-21.5,0 -39,17.5 -39,39s17.5,39 39,39 39,-17.5 39,-39S75.5,15 54,15z" />
    <path
        android:fillColor="#000000"
        android:pathData="M49.5,8h9v11h-9zM31.2,16.8l-7.6,7.6 3.8,3.8 7.6,-7.6zM76.8,16.8l3.8,3.8 -7.6,7.6 -3.8,-3.8z" />
    <path
        android:fillColor="#FFFFFF"
        android:pathData="M54,26c-15.5,0 -28,12.5 -28,28s12.5,28 28,28 28,-12.5 28,-28S69.5,26 54,26z" />
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
    <string name="app_name">十倍率</string>
</resources>
"####,
    },
    AndroidSource {
        path: "res/values/themes.xml",
        contents: r####"
<resources xmlns:tools="http://schemas.android.com/tools">
    <style name="Theme.GridTimer" parent="Theme.AppCompat.DayNight.NoActionBar">
        <item name="android:windowBackground">@android:color/transparent</item>
        <item name="android:statusBarColor">@android:color/transparent</item>
        <item name="android:navigationBarColor">@android:color/transparent</item>
        <item name="android:windowTranslucentStatus">false</item>
        <item name="android:windowTranslucentNavigation">false</item>
        <item name="android:forceDarkAllowed" tools:targetApi="q">false</item>
    </style>
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
    <string name="app_name">Grid Timer</string>
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
    <string name="app_name">グリッドタイマー</string>
</resources>
"####,
    },
    AndroidSource {
        path: "res/values-v27/themes.xml",
        contents: r####"
<resources>
    <style name="Theme.GridTimer" parent="Theme.AppCompat.DayNight.NoActionBar">
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
    <include domain="file" path="timer_state.json" />
    <include domain="file" path="timer_state_backup.json" />
</full-backup-content>
"####,
    },
    AndroidSource {
        path: "res/xml/data_extraction_rules.xml",
        contents: r####"
<?xml version="1.0" encoding="utf-8"?>
<data-extraction-rules>
    <cloud-backup>
        <include domain="file" path="timer_state.json" />
        <include domain="file" path="timer_state_backup.json" />
    </cloud-backup>
    <device-transfer>
        <include domain="file" path="timer_state.json" />
        <include domain="file" path="timer_state_backup.json" />
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
    <locale android:name="ja-JP" />
</locale-config>
"####,
    },
];
