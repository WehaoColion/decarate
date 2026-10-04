# v2.22.48.1 - 缩短安卓首次启动加载时间

已在连接的 vivo V2352A 真机上覆盖安装正式签名版本 2.22.48.1，版本代码 22249。三次验收均为冷启动，界面显示 35 项归档，启动日志没有致命异常。测试保留原应用数据，未卸载、清空存储或更改 VPN。

## 真机结果

原版本 2.22.48-rest-bell 在启动后 70.165 秒仍显示加载，74.268 秒首次观测到主界面。新版本的三次主界面核验耗时为 10.635 至 12.463 秒。以原版本仍在加载的时间与新版最慢核验时间计算，等待时间至少减少约 82.2%。

| 启动场景 | 应用数据就绪 秒 | 主界面核验完成 秒 | 已校验快照数 |
| --- | ---: | ---: | ---: |
| 覆盖安装后首次启动 | 8.908 | 12.463 | 1253 |
| 再次冷启动 1 | 7.047 | 10.635 | 1254 |
| 再次冷启动 2 | 6.722 | 11.369 | 1254 |

应用数据就绪时间来自同一进程的单调时钟。主界面核验时间包含 ADB 启动命令、日志轮询和界面层级读取开销。原版采用界面层级轮询，因此原版真实就绪时间处于上述区间。重复冷启动由停止应用进程后重新打开完成。

## 修复内容

启动时在原有 SQLite 事务内逐条读取当前快照和历史快照，由 Rust 校验原始内容摘要、元数据摘要和顶层数据版本。当前快照通过校验并成功解码后，复用本次事务的校验结果，省去再次遍历历史大文本的开销。校验结果绑定工作区、数据库写入标记和事务生命周期。

历史版本兼容、损坏数据恢复、未来版本保护和草稿恢复仍经过原有入口。字节读取使用 SQL 的 BLOB 转换，避免 Android 文本游标内部结束字节影响摘要。

## 验证

源代码生成测试 109 项、打包测试 52 项、Android JVM 测试 78 项通过。Rust 全量基线 639 项通过，修订的校验模块另有 5 项针对性测试通过。未改变的模块复用此前同轮完整测试结果，详见 offline_build_acceptance.json 与代码摘要。

内容摘要、元数据摘要和版本保护分别删除后，相关测试均按预期失败；只修改加载文案，状态测试继续通过；文案随后已恢复。正式构建、Android lint、签名、包名、版本代码和 16 KB 对齐检查通过。

主目录、APK 目录、release_artifacts/current 与构建输出的正式 APK 内容一致。旧交付 APK 已移入项目 old_apks。

最终 APK SHA-256：a9ec931c39d6f15427ed93b6fecfe3eb9db0e96ddd5385efcecfe1605009ae2c

测量原始记录：baseline_result.json、final_first_result.json、final_cold_1_result.json、final_cold_2_result.json。发布检查记录：signature.txt、badging.txt、alignment.txt、apk_copies_verified.json。

Android 字节读取行为依据：[AOSP CursorWindow JNI 实现](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/main/core/jni/android_database_CursorWindow.cpp)。