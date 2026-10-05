- v2.22.30：修复安卓端持续绘制、启动同步读取及历史索引重算导致的界面负担。

# 安卓流畅度修复与验证

## 修改范围

版本从 2.22.29-persistence-recovery 更新为 2.22.30-android-responsiveness。保留包名 com.ofairyo.gridtimer 和原正式签名。所有应用源码改动位于 Rust 源码生成器，未使用模拟器或真机。

## 已确认的开销与修改

| 路径 | 原来的开销 | 本次修改 |
| --- | --- | --- |
| 运行态视觉提示 | 多个 Animatable 永久循环，背景径向渐变逐帧创建 | 运行提示保持静态，秒级计时继续刷新；背景渐变缓存 |
| 表盘 | 每次绘制计算 60 组刻度坐标 | 刻度合为 4 组 Path，静态表盘独立于 elapsedMillis，使用 drawWithCache |
| 界面计时 | 页面保留在 composition 时后台继续唤醒 | 仅 STARTED 生命周期刷新，回前台立刻读取系统时刻 |
| 账户初始化 | 构造仓库时同步访问安全存储和恢复检查点 | 放到 IO 初始化任务；业务修改和账号设置等待初始化，草稿写入在工作区恢复前被拦截；失败保留原凭据与工作区证据 |
| 数据发布 | 修改后重建全部集合，StateFlow 主线程深比较 | 后台比较并复用完全相同的数据域；发布和保存登记保持取消原子性 |
| 历史索引与今日统计 | 主线程遍历历史，随无关数据修改重新计算 | 按相关数据域在后台计算，以请求身份隔离工作区结果 |
| 微休息推进失败 | 到期但未前进时可零延迟循环 | 确认提交结果，未前进时退避 1 秒，失败提交不发转场提醒 |
| 诊断日志 | 普通事件同步写盘和裁剪日志 | 256 条有界异步队列与文件轮转，导出和退出保留有界排空；业务数据不使用该有损队列 |
| Compose 编译 | 未启用强跳过 | 启用当前 1.5.14 编译器的 strong skipping，减少未变组件重组 |

## 验证结果

已通过以下检查：

| 检查 | 结果 |
| --- | --- |
| Rust 源码生成与回归 | 71 项通过 |
| Rust 核心逻辑 | 518 项通过 |
| Windows 客户端发布检查 | 168 项通过 |
| 同步启动器检查 | 37 项通过 |
| 打包器检查 | 41 项通过 |
| 稳定桌面入口检查 | 24 项通过 |

稳定桌面入口原有测试存在并发临时目录重名问题。本次仅修复其测试目录分配方式，新增同一时间值下 16 线程目录隔离回归，保留全部产品校验断言。

正式版 Compose 编译报告已经确认 GridTimerRoot、TimerHomeScreen、TimerTile 和 TimerDialFace 均为 restartable skippable。模块记录 640 个可跳过组件、1240 个已记忆化 lambda；这些是编译器静态指标，不是帧率或提升比例。报告保存在 release_artifacts/verification/v2.22.30。

安卓正式版编译成功，Gradle 共处理 81 项任务；lintDebug 结果为 0 个错误、16 个警告，lintVitalRelease 通过。项目没有独立的 Android 单元测试源，相应任务显示 NO-SOURCE，上表行为回归来自 Rust 测试及源码生成检查。APK 已通过 v2/v3 签名校验，证书 SHA-256 与上一版一致；正式发布与独立 tools/windows.ps1 verify 均以退出码 0 完成，版本、源码对应关系和交付一致性校验通过。编译、源码回归和签名校验无法代替真机帧耗时测试，本报告不声称已测得 FPS、启动耗时或提升百分比。

## 正式 APK

文件名：grid_timer_app_v2.22.30-android-responsiveness.apk。

包名 com.ofairyo.gridtimer，versionCode 22230，versionName 2.22.30-android-responsiveness。大小 19,178,419 字节，约 18.29 MiB，包含 arm64-v8a、armeabi-v7a、x86_64；AndroidManifest 未启用 debuggable 或 testOnly。证书与 2.22.29 相同，版本号递增，具备覆盖升级条件。

APK SHA-256：`021eacc963255845cde66593a1d48d97d8b7bac6732ed913352a8d3056790d71`。

签名证书 SHA-256：`4ac7b86a600bbb3215a6187018c1442efcf3ffb25cdbeea0082cab81ef4d605f`。
根目录、APK、release_artifacts/current 和正式构建输出共 4 份 APK 的 SHA-256 完全一致。上一版的交付 APK 已归入 old_apks。全项目包文件检查未发现 debug APK、app-release.apk 或 AAB。完整构建日志、独立核验日志、Lint、签名、清单与各副本校验记录已归档到 release_artifacts/verification/v2.22.30。

## 真机验证边界

真机验收应覆盖：打开应用；多个计时器连续运行及滚动首页；快速启停后退出重开；切换便签和历史；前后台切换；账户恢复及同步后的数据显示。计时秒数、保存结果和工作区隔离需要与流畅度一起核对。此次未连接或操作真机。

## 技术依据

当前 Compose 1.5.14 的 Kotlin 版本对应关系，以及 strong skipping 自 1.5.13 起可用于正式版本，见 [Android 官方编译器发布说明](https://developer.android.com/jetpack/androidx/releases/compose-compiler)。该优化用来跳过参数未变化的可组合项，详见 [Android 官方 strong skipping 说明](https://developer.android.com/develop/ui/compose/performance/stability/strongskipping)。