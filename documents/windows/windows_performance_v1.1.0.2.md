v1.1.0.2（Windows）- 启动、页面缓存与法律任务性能验证记录。

# Windows 1.1.0.2 性能与验收记录

## 1. 当前状态与结论范围

**发布状态：Windows 1.1.0.2 已正式发布，独立 verify 退出码为 0，正式 EXE 的 5 组隔离业务自检通过。** 发布验证收据（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/published_windows_build_verification.json`） 记录 `passed: true`、`publicationPerformed: true`，与 正式发布清单（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/published_release_manifest.json`） 一致。打包外层 PowerShell 输出进程曾持续等待，未捕获 package shell 退出码，按第 9.2 节单独记录，不改写成命令退出 0。第二组性能测量使用发布前候选程序：首次可编辑帧中位数从 ProcessStart 算起缩短 **30.48%**，从 launchCall 算起缩短 **29.05%**，**全程缩短 30% 的目标尚未达到**；包含启动调用的首窗有一轮为 **2,378.8527 ms**，严格的“5 轮均不超过 2 秒”也未全部满足。CPU 未见可宣称的改善，候选启动峰值内存增加。正式程序与性能候选字节不同，性能测量与正式程序验证分别记录。历史失败与外部界面验证限制保留，发布不代表所有性能目标达标。

正式 EXE 同字节副本已完成隔离工作区的实际进程启动、正常关闭及不重置资料重开。随后直接从 `current` 正式目录启动用户日常工作区，窗口版本和路径已观察并激活，程序响应且资料可编辑，客户端保持打开。该真实工作区**单次**从启动调用到首次可编辑帧为 **18.737 秒**，没有旧版对照；第 5 节五轮约 7.2 秒的指标属于隔离合成资料，不能作为用户实际资料的加载时间。

本次优化涉及 Windows 启动核验、按内容类别失效的页面缓存、法律预览与报告共享持有、法律扫描准备和本机存储后台任务，以及法律报告同步的无变化处理和重试调度。详细改动见 [Windows 1.1.0.2 更新说明](../releases/windows/release_notes_windows_v1.1.0.2.md)。Android 保持 2.23.1.1；本报告不把 Windows 测量结果外推为 Android 的性能变化。

本文全部验证证据位于 `release_artifacts/verification/windows_v1.1.0.2`（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2`）。性能对照、专项测量和业务自检使用隔离合成工作区、合成报告及测试请求；交付前另记录一次真实用户工作区启动，仅在本报告呈现程序身份、启动状态与时点，不披露资料正文。尚未测得的数值统一标为“待测”，不以估算值补位，不根据代码改动预先给出速度或内存改善百分比。

## 2. 基线、候选与构建配置

### 2.1 测量程序与正式发布程序分别记录

| 程序 | 身份与用途 | 当前证据 |
|---|---|---|
| 原始正式客户端 1.1.0.1 | 保存原始发布字节，用于正式程序身份核对；它没有新增启动计时点 | 原正式清单（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/baseline_release/release_manifest.json`）、基线文件哈希（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/baseline_release_hashes.json`） |
| 正式配置基线测量程序 | 由保存的基线源码构建，加入首次界面更新的进入、完成两处计时日志；使用 release 优化配置，专供可比较的启动测量 | 计时改动收据（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/baseline_ui_probe_receipt.json`）、release 构建日志（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/baseline_release_build.log`） |
| 1.1.0.2 性能候选 | 用于发布前正式配置测试、两组 GUI 和专项性能测量 | 两组 GUI 结果见第 5 节，原始崩溃证据见第 8 节；保留其独立程序哈希 |
| 1.1.0.2 正式发布客户端 | package 完成发布内嵌完整性绑定后的程序 | 发布、独立 verify、正式 EXE 自检及文件核对见第 9 节；不能当作与性能候选同字节 |

原始正式客户端文件为 `grid_timer_windows_client_v1.1.0.1.exe`，大小 **23,883,776 字节**，SHA-256 为 `9de72ca45f603add29710805bd38e7d38dac17ef8cf42d1b69677a513688ff49`。

基线测量程序为 `probe_binaries/baseline_1.1.0.1_ui.exe`，大小 **23,866,368 字节**，本次文档核对时的 SHA-256 为 `2a9d1aba75158bde278cb9ad14b57c971f5e6788580b58892fc1ed5c5ed866df`。该程序**不等同于原始正式客户端字节**。计时改动收据记录了 `src/bin/timer_windows_client.rs` 的原始与测量版哈希：每次更新增加一次 relaxed 原子标志检查，首次更新增加两条本机日志；没有更改启动加载器、跳过核验或加入自动退出行为。构建日志已记录 `Finished release profile [optimized]`，耗时属于构建耗时，不属于启动耗时。

正式客户端 `grid_timer_windows_client_v1.1.0.2.exe` 为 **24,136,192 字节**，SHA-256 为 `86d9b637707cad9ea820a53ad3b3c234501296f16fcaf4f2367876ca448069cc`。第二组性能候选的哈希为 `27384398a8487cefb28cdc0af701c4a2cfa37b6aee36440f555c008ca04ebf7c`。package 包含发布内嵌完整性绑定，因此不能把第二组测量记为正式发布 EXE 的同字节性能结果。

### 2.2 debug 测试与 release 性能分开

第 3 节保留的关键状态测试及原先 8 轮原生同步测试来自 debug 测试程序。当时的候选测试构建日志（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/compile_candidate.log`） 明确记录 `test profile [unoptimized + debuginfo]`。这些结果用于确认当时已覆盖的业务状态与实际调用路径；日志中的 7.76 秒、32.60 秒等执行时间不作为正式客户端的性能结论，也不与 release 启动或帧时间直接比较。随后发生过 release 全套测试与单线程复测原生异常，原记录仍保留。

请求对象借用修复后的阶段测试使用 `probe_binaries/candidate_tests_request_fix.exe`，SHA-256 为 `426c52ee0f4762e9ca5f9ab498c72d8beb83f40695bf2391e3f8440cc3431da3`。修复后构建日志（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/candidate_release_test_request_fix_build.log`） 记录 `Finished release profile [optimized]`。以下结果保留其程序身份；后续异步字体版本的重新验证另列于第 2.3 节：

| release 验证项目 | 已完成结果 | 原始证据 |
|---|---|---|
| 原失败路径精确复测 | 1 通过、0 失败，0.15 秒 | legal_request_fix_exact.log（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/legal_request_fix_exact.log`） |
| 客户端全套状态测试 | 474 通过、0 失败、30 ignored，72.15 秒 | client_release_tests_request_fix.log（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/client_release_tests_request_fix.log`） |
| 显式原生法律报告往返 | 8 轮、98 请求，4,454 毫秒，1 通过 | legal_native_release_request_fix.log（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/legal_native_release_request_fix.log`） |
| 显式法律预览、报告帧测量 | 各负载 30 帧，已完成 | 第 6 节 JSON 与日志 |
| 显式 32 MiB 准备回收探针 | 1 通过、0 失败，进程退出码 0 | 第 7 节 JSON 与日志 |
| 显式页面切换和编辑测量 | 1 通过、0 失败，常用与压力两种合成规模 | 第 6.2 节 JSON 与日志 |

30 个 ignored 是全套运行时未自动执行的项目；随后单独运行的专项测量按自己的日志记录，不能直接将 ignored 全部算为通过。全套状态测试耗时也不是客户端启动时间。

正式配置的启动比较每组按基线、候选各 **5 次独立 GUI 进程**执行，两组结果见第 5 节。原先为独立启动测试程序准备的 注入记录（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/startup_probe_injection.json`） 留作过程记录，其中“5 次测试进程”的方案不代表实际 GUI 测量。本轮不再为启动比较额外编译基线 release 测试程序。

### 2.3 异步字体版本的正式配置验证

最新候选测试程序为 `probe_binaries/candidate_tests_async_fonts.exe`，SHA-256 为 `3996b7cd01ac76c32555e8aad2f20d15813d7ee27b7c32281d03761cf61f2c9a`，见 测试程序哈希收据（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/candidate_tests_async_fonts_hash.json`）。client_release_tests_async_fonts.log（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/client_release_tests_async_fonts.log`） 记录 **477 通过、0 失败、30 ignored**，用时 172.19 秒；该时间仍是状态测试总时间，不是启动性能。格式检查日志（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/format_windows_async_fonts.log`） 对应的 **205 个共享及 Windows Rust 源文件**检查通过，Android 源码生成器不在此格式检查范围。

第二组 GUI 使用 `probe_binaries/candidate_1.1.0.2_async_fonts.exe`，SHA-256 为 `27384398a8487cefb28cdc0af701c4a2cfa37b6aee36440f555c008ca04ebf7c`，见 GUI 程序哈希收据（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/candidate_async_fonts_hash.json`）。构建日志（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/candidate_release_async_fonts_build.log`） 记录 release optimized 构建完成，耗时 6 分 59 秒仅为构建耗时。测试程序、GUI 程序与最终 package 文件分别核对，不能互换哈希。

`tests::native_window_visual_review` 在该阶段正式配置测试程序中通过，1 通过、0 失败，**27.41 秒**，见 原生视口检查日志（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/native_visual_review_async_fonts.log`）。视口证据目录（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/native_visual_review_async_fonts`） 保存 **32 张原生视口 PNG 和 1 张笔记导出长图**，导出状态（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/native_visual_review_async_fonts/export_status.json`） 确认导出完成。已查看看板、笔记、财务、账户四类页面图像，本轮查看未发现明显布局异常。该测试确实创建原生视口，但由应用内部夹具切换场景，未替代完整启动流程，也没有通过外部控件树执行编辑、保存、重开的操作链路。

## 3. 已完成的历史 debug 状态验证

### 3.1 测试结果

| 范围 | 已完成结果 | 验证重点 | 原始日志 |
|---|---:|---|---|
| 启动读取与日志库 | 56 通过，0 失败，1 个显式性能项未运行 | 按原始字节与时间匹配的复用、所有者与恢复证据、损坏回退、未来格式、隐私删除、原子提交与迁移回滚 | startup_library_tests.log（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/startup_library_tests.log`） |
| 启动恢复及桌面数据边界 | 78 通过，0 失败，3 个显式夹具或性能项未运行 | 取消不开放编辑和同步、未经核验的结果进入恢复状态、检查点提交后才允许写入、账户隔离、恢复和隐私边界 | startup_recovery_tests.log（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/startup_recovery_tests.log`） |
| 法律界面、存储与同步 | 16 通过，0 失败，3 个显式性能项未运行 | 发送配置及摘要、取消和换账户、过期准备结果、失败保存、后台完成唤醒、关闭页面释放、墓碑与分块提交 | legal_tests_final.log（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/legal_tests_final.log`） |
| 原生法律报告往返 | 1 通过，0 失败，显式运行 8 轮 | 生产 HTTP 调用路径、Windows 原生加密、提交回包丢失后重试、取消下载后重试 | legal_native_final.log（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/legal_native_final.log`） |

以上为各自 debug 过滤器在当时源码下的历史结果，范围可能交叉，不相加宣称为全项目测试总数。“未运行”的显式性能项不计为通过。修复后的 release 结果另见第 2.2 节；原始异常、独立复现与修复依据见第 8 节。

法律状态验证覆盖了：未配置 AI 或摘要不匹配时不能启动发送；资料版本、账户、工作区、脏编辑和取消状态变化后，旧准备结果不得重新启用发送；合成能力检查返回后，取消或换账户不能继续发出下一请求；保存失败不能进入准备任务；任务完成主动唤醒界面；窗口隐藏后仍回收完成结果，关闭页面后不重新装入已关闭的预览和报告。

### 3.2 无变化同步的实际工作量

`legal_sync_unchanged_nonempty_and_empty_use_one_manifest_without_body_reads_or_writes` 已通过。其日志记录：**1 次 manifest 请求、1 次索引读取、0 次报告正文读取、0 次索引写入**。该结论限定于测试覆盖的空工作区和正文元数据已匹配的非空工作区；需要迁移旧索引或发生新增、删除、下载时，仍会执行对应校验与写入。

### 3.3 8 轮原生同步验证

最终原生日志（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/legal_native_final.log`） 输出 `rounds: 8`、`requests: 98`、`productionHttp: true`、`nativeEncryption: true`、`ambiguousCommitRetry: true`、`cancelledDownloadRetried: true`、`syntheticOnly: true`。本轮使用实际 HTTP 客户端及 Windows 原生加密路径，完成非空报告往返和两类重试验证。98 是整轮测试中的请求总数，包含特意制造的失败与重试；不能换算为日常每次同步必需的请求数。

该 debug 测试耗时 **32,600 毫秒**。它证明该次运行的相应实际路径完成，不用于宣布 release 吞吐率或用户网络耗时。此前 legal_native.log（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/legal_native.log`） 保留了一次测试服务器非阻塞读取 `WouldBlock` 失败，随后出现连接拒绝；该失败不计为通过，也不删除。这一 debug 原生往返项目的历史结果以修正测试服务器处理后的日志为准。

修复后另外运行了正式优化配置的同一专项测试：legal_native_release_request_fix.log（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/legal_native_release_request_fix.log`） 记录 **8 轮、98 请求、4,454 毫秒**，上述实际 HTTP、原生加密、丢失提交回包重试、取消下载重试和合成资料标志均为 true，测试通过。它是本轮隔离合成场景的 release 实测；不把 debug 与 release 的时间差当作本次产品优化比例，也不外推为真实网络中的固定同步耗时。

## 4. 实际变异验证与源码还原

本次实际修改目标源码、重新运行指定测试，再恢复原始字节。四个变异均按预期完成，失败均为测试断言失败，不以编译失败代替关键状态验证。

| 变异 | 对应状态 | 预期 | 实际 |
|---|---|---|---|
| `wording` | 修改“发送并分析”文案 | 测试继续通过 | 退出码 0，1 通过 |
| `send_authorization` | 移除启动发送前的授权状态判断 | 测试失败 | 退出码 101，1 失败，错误启动了后台任务 |
| `loading_write_boundary` | 移除加载结果必须可持久化的分支条件 | 测试失败 | 退出码 101，1 失败，恢复状态断言不符 |
| `stale_preparation` | 移除扫描准备结果的数据版本判断 | 测试失败 | 退出码 101，1 失败，过期预览被接收 |

结果见 mutation_results.json（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/mutations/mutation_results.json`）；各次完整日志保存在同一 `mutations` 目录。source_restore_hashes.json（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/mutations/source_restore_hashes.json`） 记录 `AllRestored: true`，三个变异目标在恢复后的 SHA-256 与变异前一致：

| 文件 | 恢复后 SHA-256 |
|---|---|
| `legal_risk.rs` | `098e5776055ff64052284b149a9794a99d2157ee145972f09502a3d106cec73a` |
| `legal_background.rs` | `ff0351453c0b36fff32fb2872fcb1f55407e6f618ebe3c4a83a364cc6217e803` |
| `startup_performance.rs` | `92366330fb8c866ff112b3ee723eac0c982e31baa659a183309f1a3a20c83436` |

以上哈希证明本次变异结束时的字节恢复；最终发布源码及发布程序仍使用最终清单重新核对。

## 5. 正式配置启动测量（两组完成，保留全部样本）

### 5.1 合成资料与采样定义

合成夹具包含 **128 篇笔记**，主状态 **3,148,475 字节**，历史 SQLite **69,492,736 字节**。数量由 startup_fixture.log（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/startup_fixture.log`） 给出，文件清单、字节数和哈希由 fixture_receipt.json（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/fixture_receipt.json`） 保存。夹具生成用时 419.39 秒仅为 debug 环境准备时间，不是客户端启动时间。

夹具中的恢复证据与隔离目录 `C:\tools\tmp\startup_synthetic_v1_1_0_2` 绑定，每轮恢复到同一目录。第一组在北京时间 **2026 年 9 月 30 日**完成，使用同一合成模板和 8 个逻辑处理器的机器，各启动 5 个独立新进程；操作系统磁盘缓存未清空，运行前可用物理内存逐轮记录。外部采样间隔为 50 ms，测量格式版本为 2；脚本为 [measure_windows_process_performance.ps1](../../tools/measure_windows_process_performance.ps1)，各轮原始结果完整保存在 samples（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples`）。

保留组标签为 `baseline_final`、`candidate_final`，其中 `final` 是当时的采样标签，**不代表字体优化后或最终验收通过**。基线使用第 2.1 节的测量程序及哈希；候选为 `probe_binaries/candidate_1.1.0.2_fixed_ui.exe`，5 轮均为 SHA-256 `1981cd3eae3cb63db4e8e14637b0bceaa54a322e47d343617fd67e9ac381c380`。异步字体版本另采第二组 `baseline_fonts`、`candidate_fonts`，见第 5.4 节，未覆盖或混合第一组原始记录。

### 5.2 第一组逐轮启动时间与未达标样本

下表单位为 ms，每格按 **ProcessStart / launchCall** 列出两个起算点：前者从操作系统记录的进程启动时间算起，包含框架创建前的初始化；后者从测量脚本调用启动算起，另含调用到创建进程的等待。首个有标题窗口来自外部窗口元数据观察，首次 UI 更新及首次可编辑帧来自应用日志。它们均不等同于 GPU 呈现时间，也不直接证明用户已实际完成编辑。不同观察使用的时钟与采样点不同，保留原始字段，不用相减补造缺测值。

| 保留组与轮次（原始 JSON） | 首个有标题窗口 | 首次 UI 更新 | 首次可编辑帧 |
|---|---:|---:|---:|
| baseline 1（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/baseline_final_FreshProcess_01/measurement.json`） | 13,679 / 15,204.1432 | 13,532 / 15,068 | 13,555 / 15,091 |
| baseline 2（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/baseline_final_FreshProcess_02/measurement.json`） | 缺测 / 缺测 | 13,984 / 13,994 | 14,006 / 14,016 |
| baseline 3（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/baseline_final_FreshProcess_03/measurement.json`） | 14,353 / 14,361.1382 | 14,123 / 14,137 | 14,151 / 14,165 |
| baseline 4（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/baseline_final_FreshProcess_04/measurement.json`） | 13,226 / 13,230.9588 | 13,012 / 13,028 | 13,056 / 13,072 |
| baseline 5（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/baseline_final_FreshProcess_05/measurement.json`） | 13,510 / 13,517.2594 | 13,369 / 13,382 | 13,384 / 13,397 |
| candidate 1（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/candidate_final_FreshProcess_01/measurement.json`） | 1,067 / **2,469.0760** | 891 / 2,305 | 8,713 / 10,127 |
| candidate 2（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/candidate_final_FreshProcess_02/measurement.json`） | 791 / 797.1475 | 665 / 677 | 8,483 / 8,495 |
| candidate 3（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/candidate_final_FreshProcess_03/measurement.json`） | 1,588 / 1,596.8991 | 1,364 / 1,375 | 9,394 / 9,405 |
| candidate 4（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/candidate_final_FreshProcess_04/measurement.json`） | **2,728 / 2,736.4630** | 2,529 / 2,542 | 12,219 / 12,232 |
| candidate 5（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/candidate_final_FreshProcess_05/measurement.json`） | **2,535 / 2,553.0656** | 2,206 / 2,235 | 10,940 / 10,969 |

候选第 4、5 轮从 ProcessStart 起首窗已超过 2 秒，第 1 轮从 launchCall 起为 **2,469.076 ms**。不能只报中位数或舍去启动调用等待后宣布首窗全部达标。baseline 第 2 轮在空闲采样时长为零时，脚本读到就绪标记便提前结束，没有记录首个有标题窗口；随后脚本修正了这一结束条件。该轮 UI 更新及可编辑帧日志有效，首窗仍保留为缺测，不用更早的空标题句柄代替。

按 ProcessStart 计算，首次可编辑帧的候选中位数为 **9,394 ms**，基线为 **13,555 ms**，本组中位数缩短约 **30.7%**；范围分别为 8,483～12,219 ms、13,056～14,151 ms。按 launchCall 计算，中位数变为候选 **10,127 ms**、基线 **14,016 ms**，缩短 **27.75%**，未达到全程缩短 30% 的目标。两种起点包含的等待不同，会改变样本排序与中位数，不能将 ProcessStart 改善比例当作全程改善。上述结果仅对应字体优化前保留组，不能用于评价后续字体改动，亦不能替代首窗逐轮验收。

### 5.3 第一组 CPU、内存、窗口及退出证据

只有两组各第 1 轮测量了就绪后约 60 秒 CPU，其余 8 轮的 CPU 字段为 **null，未测**，不能按零计入或形成 5 轮 CPU 平均值。各轮均使用隔离合成资料，本组未覆盖真实账户后台同步负载。

| 第 1 轮 | 实测区间 ms | 进程 CPU 时间 ms | 占单个逻辑处理器比例 | 占机器总量比例（8 个逻辑处理器） |
|---|---:|---:|---:|---:|
| baseline | 60,067.6355 | 984.375 | 1.639% | 0.205% |
| candidate | 60,077.5437 | 890.625 | 1.482% | 0.185% |

5 轮记录中的最高进程峰值工作集分别为基线 **183.36 MiB**、候选 **215.10 MiB**，最高采样私有内存分别为 **140.48 MiB**、**162.36 MiB**。这些是各轮不同运行区间内的峰值，不能将其写成内存降低；逐轮字节数与采样条件保留在原始 JSON 中。

10 轮均有匹配标题及程序路径的窗口观察、就绪与响应记录，并在正常 Alt+F4 关闭后留下同目录 `exit.json` 的 `cleanExit: true`。测量中的 `passed: true` 表示测量流程完成，不代表所有性能阈值通过。候选与基线的外部控件树均曾返回 `null`，窗口截图此前报 `0x80004002`；没有盲点或改用私建全局 UIAutomation。因此本组证明原生窗口运行及正常退出，不宣称已经通过外部界面完成切页、编辑保存和重开的人工式操作验证。第 6 节页面测试及业务保存测试仍按其合成测试范围解释。

### 5.4 第二组最终候选的逐轮数据

第二组同样在北京时间 2026 年 9 月 30 日完成，保留相同合成夹具、隔离目录、50 ms 外部采样和未清空操作系统磁盘缓存的条件。基线仍为第 2.1 节同一测量程序，候选为第 2.3 节异步字体 GUI 程序，10 轮程序身份均与相应哈希一致，均有正常退出收据。下表继续按 **ProcessStart / launchCall** 列出 ms，不与第一组混算。

| 第二组与轮次（原始 JSON） | 首个有标题窗口 | 首次 UI 更新 | 首次可编辑帧 |
|---|---:|---:|---:|
| baseline 1（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/baseline_fonts_FreshProcess_01/measurement.json`） | 12,423 / 12,445.7695 | 12,259 / 12,292 | 12,290 / 12,323 |
| baseline 2（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/baseline_fonts_FreshProcess_02/measurement.json`） | 10,546 / 10,551.6828 | 10,391 / 10,403 | 10,405 / 10,417 |
| baseline 3（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/baseline_fonts_FreshProcess_03/measurement.json`） | 10,370 / 10,376.4747 | 10,240 / 10,251 | 10,254 / 10,265 |
| baseline 4（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/baseline_fonts_FreshProcess_04/measurement.json`） | 10,910 / 10,918.3558 | 10,770 / 10,787 | 10,787 / 10,804 |
| baseline 5（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/baseline_fonts_FreshProcess_05/measurement.json`） | 10,335 / 10,340.8020 | 10,208 / 10,220 | 10,222 / 10,234 |
| candidate 1（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/candidate_fonts_FreshProcess_01/measurement.json`） | 867 / **2,378.8527** | 755 / 2,272 | 7,032 / 8,549 |
| candidate 2（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/candidate_fonts_FreshProcess_02/measurement.json`） | 843 / 848.3963 | 708 / 720 | 7,234 / 7,246 |
| candidate 3（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/candidate_fonts_FreshProcess_03/measurement.json`） | 834 / 839.5874 | 758 / 768 | 7,073 / 7,083 |
| candidate 4（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/candidate_fonts_FreshProcess_04/measurement.json`） | 690 / 695.2055 | 524 / 531 | 8,201 / 8,208 |
| candidate 5（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/candidate_fonts_FreshProcess_05/measurement.json`） | 1,075 / 1,082.0193 | 848 / 859 | 7,380 / 7,391 |

从 ProcessStart 起，候选首窗 **5/5 小于 2 秒**，范围 690～1,075 ms；从 launchCall 起，首轮为 **2,378.8527 ms**，所以包含启动调用的严格 **5/5 不超过 2 秒目标未全部满足**。没有证据将这次等待归因于 Defender 或其他特定后台程序。按 ProcessStart 计算，候选首次可编辑帧中位数为 **7,234 ms**，基线 **10,405 ms**，缩短 **30.48%**；范围分别为 7,032～8,201 ms、10,222～12,290 ms。按 launchCall 计算，中位数为候选 **7,391 ms**、基线 **10,417 ms**，缩短 **29.05%**，**全程缩短 30% 的目标未达到**。两种口径均保留，不能用进程创建后的比例代替全程比例。这些结果也不能把与第一组的差额单独归因于字体改动，或外推为真实资料和其他机器上的固定提速。

第二组仅各第 1 轮进行约 60 秒 CPU 测量，其余 8 轮仍为未测：

| 第 1 轮 | 实测区间 ms | 进程 CPU 时间 ms | 占机器总量比例（8 个逻辑处理器） |
|---|---:|---:|---:|
| baseline | 60,050.8417 | 8,625.000 | 1.7954% |
| candidate | 60,020.0949 | 8,781.250 | 1.8288% |

这两次 CPU 比例接近，候选略高，单次样本不能支持 CPU 改善结论，也不与第一组不同时间段的 CPU 数值混算。第二组候选进程峰值工作集为 **204.69～206.36 MiB**，基线 **182.01～184.55 MiB**；候选采样私有内存峰值为 **160.86～162.71 MiB**，基线 **138.10～142.01 MiB**。本组启动期间内存增加，不能用法律页面关闭后释放对象的单项结果抵消或隐藏这一事实。10 轮正常退出、原生视口图像检查与外部编辑链路未验证的边界分别按第 2.3、5.3 节记录。

## 6. 界面帧开销、切换与编辑（release 实测完成）

### 6.1 法律预览与报告对象持有方式对比

候选 release 测试程序按每种负载 **30 次**执行同一渲染路径。预览负载为 **1、8、32 MiB**，报告负载为 **1、16、32 MiB**，记录耗时和每次分配字节数。

这一对比在相同候选绘制函数中分别执行“显式深复制对象”和“共享不可变 `Arc` 对象”，用于量化取消深复制带来的开销变化。它不代表完整旧版 UI 与新版 UI 的端到端帧率对比，也不能将每次分配减少量当作进程常驻内存减少量。列表虚拟绘制、折叠状态及长文分段均保持相同测量条件。

时间单位为微秒（μs），分配单位为字节/帧。每行分别有 30 次深复制路径和 30 次共享路径测量。

| 场景 | 负载 | 深复制中位数 / P95 | 共享中位数 / P95 | 深复制分配 / 帧 | 共享分配 / 帧 |
|---|---:|---:|---:|---:|---:|
| 折叠扫描预览 | 1 MiB | 587.2 / 1,026.7 | 20.1 / 29.6 | 1,056,243 | 7,466 |
| 折叠扫描预览 | 8 MiB | 5,812.8 / 7,473.0 | 19.8 / 31.4 | 8,396,275 | 7,466 |
| 折叠扫描预览 | 32 MiB | 22,817.6 / 39,949.7 | 32.6 / 34.0 | 33,562,102 | 7,469 |
| 折叠报告线索 | 1 MiB | 581.0 / 1,854.2 | 30.2 / 46.2 | 1,054,603 | 5,596 |
| 折叠报告线索 | 16 MiB | 11,230.7 / 14,901.5 | 29.6 / 31.1 | 16,783,243 | 5,596 |
| 折叠报告线索 | 32 MiB | 22,369.7 / 30,093.6 | 26.8 / 29.8 | 33,560,459 | 5,596 |

原始数据见 legal_preview_release.json（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/legal_preview_release.json`） 与 legal_report_release.json（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/legal_report_release.json`）；对应通过日志为 legal_preview_release.log（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/legal_preview_release.log`）、legal_report_release.log（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/legal_report_release.log`）。六个场景的每帧分配均减少 **99% 以上**，该百分比只指此处的分配字节量。32 MiB 下，两类共享路径每帧分配分别约 7.3 KiB、5.5 KiB，未随全文大小产生一次全文复制。不能把这组结果写成“整个 Windows 客户端加速 99%”，也不能据此判断实际窗口完整帧率。

精确过滤器为 `tests::desktop_performance_probe::legal_preview_frame_allocation_benchmark` 和 `tests::desktop_performance_probe::legal_report_frame_allocation_benchmark`。它们是显式运行的 ignored 测试；普通状态测试将其列为 ignored，不能据此填入性能结果。

### 6.2 合成页面切换与编辑

desktop_pages_release.log（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/desktop_pages_release.log`） 记录显式测试通过，desktop_pages_release.json（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/desktop_pages_release.json`） 保存完整指标。常用规模为 **44 页、每页 2 块、每页 6 份历史版本，state 1,064,153 字节**；压力规模为 **129 页、每页 8 块、每页 6 份历史版本，state 18,539,223 字节**。

页面切换在同一测试框架内预热已有 `perf-0`、`perf-1` 两页，确认没有待保存草稿后，交替执行真实 `select_note_by_id` 和随后的绘制，共测量 40 次。编辑测量仍保留 40 次 `apply_note_block_text_edit`，它衡量文字变更与合并编辑状态的处理，不包含随后的整帧、落盘完成或操作系统输入传递。下表时间单位为毫秒（ms）：

| 测量项 | 次数 | 常用规模中位数 / P95 | 压力规模中位数 / P95 |
|---|---:|---:|---:|
| 已选页面绘制 `page_frame` | 40 | 0.3169 / 1.2478 | 0.3635 / 0.5927 |
| 数据库视图绘制 `database_frame` | 40 | 0.7978 / 4.3553 | 1.3852 / 5.0946 |
| 真实切页并绘制 `switch_page_and_frame` | 40 | 0.5438 / **2.5878** | 1.1674 / **3.0434** |
| 合并文字编辑 `coalesced_text_edit` | 40 | 0.0030 / **0.0353** | 0.0045 / **0.0059** |
| 显式重新解析计时投影 `timer_projection_parse` | 15 | 4.9516 / 29.7140 | 18.4649 / 37.7191 |

计时投影一项直接重新解析整个 state，用于记录该操作成本，不等于每帧都会重新解析。缓存中的 `knowledge_records` 另测 50 次，两种规模记录的中位数和 P95 均为 0.1 μs、计数线程分配为 0 字节。分配计数是测试主线程累计请求的堆字节，不是进程常驻内存。

这些数值描述同一合成测试框架中的主线程工作，不是操作系统从键盘或鼠标输入到屏幕显示的延迟，也没有对应的旧版同框架结果可用于计算版本间提速比例。精确过滤器为 `tests::desktop_performance_probe::desktop_performance_benchmark`。

## 7. 32 MiB 准备与回收内存探针（release 实测完成）

新增探针使用 **32 MiB 可读合成资料**，走实际 `prepare_legal_risk`、`advance_legal_preparation`、受管理准备线程和完成接收路径，不调用 AI 网络。它记录原始 state、evidence、batch 字节数及准备耗时，检查可读资料完整进入批次，再加入含 32 MiB 合成引文的报告，调用实际关闭页面函数。

探针通过 `Weak` 检查预览和报告的强引用是否消失；外部按阶段采集进程工作集、私有内存和峰值。阶段依次为 `process_ready`、`fixture_ready`、`worker_started`、`prepared`、`report_loaded`、`closed`、`client_dropped`。本次实际阶段停留为 **2,000 毫秒**，准备线程启动阶段不停留，正文不进入日志。

| 指标 | 结果 |
|---|---|
| 固定可读负载 | 33,554,432 字节 |
| 原始 state 字节数 | 33,569,393 |
| evidence / batch 字节数与完整性断言 | 两者文字均为 33,555,740 字节；38 个证据段、1,409 个批次，完整性断言通过；没有图片数据 |
| 准备完成耗时 | 234,053 μs，约 234.1 ms；完成接收的观察间隔为 5 ms |
| 报告引文字节数 | 33,554,432 |
| 预览与报告关闭后的强引用 | 两者均为 0，`previewReleasedOnClose`、`reportReleasedOnClose` 均为 true |
| 测试及进程退出 | 1 通过、0 失败；PID 82128，退出码 0 |
| 进程累计峰值工作集 | 216,653,824 字节，约 206.6 MiB |

外部采样设定间隔 **100 ms**，阶段按最近读到的已刷新日志标志归类；采样不是逐次分配追踪，短暂峰值可能出现在两个采样之间。下表给出各阶段采样中的最大值，单位 MiB（1 MiB = 1,048,576 字节）：

| 阶段 | 样本数 | 最大工作集 | 最大私有内存 |
|---|---:|---:|---:|
| 合成工作区就绪 `fixture_ready` | 16 | 77.48 | 67.33 |
| 准备线程运行 `worker_started` | 2 | 166.80 | 156.85 |
| 预览准备完成 `prepared` | 17 | 142.54 | 132.07 |
| 同时持有预览与报告 `report_loaded` | 16 | 174.54 | 164.13 |
| 关闭法律页面 `closed` | 17 | 78.16 | 68.60 |
| 释放测试客户端 `client_dropped` | 17 | 14.02 | 3.89 |

依据 legal_memory_ownership.json（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/legal_memory_ownership.json`）、legal_memory_process.json（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/legal_memory_process.json`） 和 legal_memory_measurement.log（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/legal_memory_measurement.log`），关闭后预览和报告大对象已不再被强引用，采样私有内存从同时持有阶段约 164.13 MiB 降至约 68.60 MiB。工作区自身仍存在，因此关闭页面后的值不应与整个客户端释放后的值混为一谈。此处仅有候选程序单次合成测量，没有对应原始正式版内存对照，不能填入版本间内存改善百分比。

精确过滤器为 `tests::desktop_performance_probe::legal_preparation_memory_reclamation_probe`，以 `--ignored --exact --nocapture --test-threads=1` 运行。`DESKTOP_LEGAL_PREPARATION_MEMORY_OUTPUT` 指定结果 JSON，`DESKTOP_LEGAL_PREPARATION_MEMORY_HOLD_MS` 调整阶段停留时间。进程峰值为累计峰值；关闭对象后峰值不会下降。强引用归零只证明对象持有结束，分配器仍可能保留空闲页，因此必须分别陈述所有权断言与 Windows 内存采样。

## 8. 异常与证据限制

### 8.1 原始 release 异常与完整证据保留

release 全套测试按两线程启动，日志声明 `running 504 tests`，在开始的几项法律存储与同步测试后原生异常终止，没有产生全套测试通过的结尾。client_release_tests.log（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/client_release_tests.log`） 保留了终止前输出；Windows 事件记录（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/crash_diagnostics/candidate_release_crash_events.json`） 记录进程 **57364（0xe014）**、程序 `candidate_tests_release.exe`、异常代码 **0xc0000374**。这次运行不能计为“504 项通过”。

随后使用 ProcDump 对同一 release 测试程序进行单线程法律同步测试复测，在首项 `legal_sync_body_repair_rejects_bad_hash_source_and_concurrent_delete` 执行时再次出现异常。调试器记录 **C0000005.ACCESS_VIOLATION**，捕获进程为 **87368**。首项测试的名称标明异常发生时正在执行的范围，尚不足以证明该项业务逻辑就是内存损坏根因；单线程仍复现也不能直接推导出具体缺陷位置。详情见 ProcDump 解码日志（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/crash_diagnostics/procdump_legal_single_decoded.txt`） 和 原始捕获日志（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/crash_diagnostics/procdump_legal_single.log`）。

已保存转储文件 candidate_tests_release.exe_260930_024631.dmp（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/crash_diagnostics/candidate_tests_release.exe_260930_024631.dmp`），大小 **80,748,430 字节**，SHA-256 为 `dce1edec605d40c1e11683cdd5d5889b4746dfd48dbc369f3b5e9d64e565664e`。事件、转储和原始日志的校验信息见 crash_capture_hashes.json（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/crash_diagnostics/crash_capture_hashes.json`）。后续定位、独立对照及修复后的结果另列如下，原失败运行不改记为通过。

### 8.2 独立复现、官方对应缺陷与借用修复

匹配符号的转储和调用方反汇编显示，第二次 manifest 请求拿到了首次调用已修改并释放的 JSON Object 存储，随后在 `BTreeMap<String, Value>::insert` 比较键时读取已释放节点。独立程序只保留 JSON 请求闭包和合成资料，使用默认分配器，不含应用 crate、DPAPI、数据库、网络和 UI；按值接收 `Value` 的优化构建仍可复现，排查已超过“仅因为 release 崩溃所以猜测编译器”的阶段。详细过程、转储与对照程序哈希见 crash_root_cause_evidence.md（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/crash_diagnostics/crash_root_cause_evidence.md`）。

官方 [Rust issue #155241](https://github.com/rust-lang/rust/issues/155241) 描述了相符的闭包参数问题：按值传入的大聚合参数在代码生成时通过指针传递，MIR 中仍会复用的参数缺少应有副本，闭包对参数的修改泄漏到调用者的复用存储。[PR #155343](https://github.com/rust-lang/rust/pull/155343) 于 2026 年 4 月 22 日合入，补齐闭包展开参数的复制处理。官方源码中，1.95.0、1.96.0 和 1.96.1 尚无该修复，1.97.0 已包含。[1.96.1 对应源码](https://github.com/rust-lang/rust/blob/1.96.1/compiler/rustc_codegen_ssa/src/mir/block.rs)、[1.97.0 对应源码](https://github.com/rust-lang/rust/blob/1.97.0/compiler/rustc_codegen_ssa/src/mir/block.rs)。本次构建仍使用实际核对的 Rust 1.95.0、LLVM 22.1.2；没有把更换工具链作为本次结果的隐含条件，也不将官方定位改写为已经证明的 LLVM 22 独有错误。

产品请求闭包改为接收 `&mut Value`，对象由每次调用者持有，发送前后的取消与账户状态检查保留。独立借用版本在优化级别 3 下通过 **10,000 次循环、179,996 次请求**；按值版本在优化级别 0 下也通过，并得到相同校验和。修复后的完整客户端 release 测试程序随后通过原失败测试、474 项自动状态测试以及 8 轮原生同步专项。该证据链支持本次请求参数复用缺陷的定位及规避处理，不代表应用所有潜在内存错误均已被证明不存在。

### 8.3 后续结论仍受测量范围约束

两组实际 GUI 样本均已完成，第一组首窗超时和缺测、第二组 launchCall 首轮超时均保留。发布已完成，独立 verify、正式 EXE 隔离业务自检及同字节副本的实际进程关闭重开验证通过，用户日常工作区也已启动。业务自检、原生视口图像、实际进程重开和候选性能测量分别对应不同范围；尚未通过外部 UIA 连续执行编辑、保存、关闭、重开，不能将已有证据写成该操作链路已经实测。

启动优化继续执行所有者、SQLite 完整性、恢复证据、未来格式及隐私清理核验；加载阶段的写入能力必须受核验结果约束。法律优化的发送授权、发送时摘要复核、账号/工作区取消、删除不复活及后台结果的时效边界分别由状态测试卡住。性能指标不能替代这些状态要求。

## 9. 正式发布验收与结论

此前 package 空间预检因仅 **7.70 GiB** 可用而停止，原始失败见 package_space_gate.log（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/package_space_gate.log`）。之后对本项目历史构建缓存执行保留文件的 LZX 压缩，4,182 个文件的收据（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/historical_cache_lzx_hashes.json`） 中压缩前后 SHA-256 全部一致；本次没有任何删除操作成功。正式 package 前收据（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/package_preflight_final.json`） 记录可用 **12,942,557,184 字节，约 12.05 GiB**，高于 12 GiB 门槛，并记录当时正式版仍为 1.1.0.1。磁盘空余随构建变化，此数值只对应收据时点。

| 项目 | 当前状态 | 证据与范围 |
|---|---|---|
| release 测试构建及状态验收 | 异步字体版本及 package 客户端测试均为 477 通过、0 失败、30 ignored；格式检查通过 | package 各组结果与正式程序身份见第 9.1 节 |
| 基线与候选实际 GUI 各 5 轮启动比较 | 两组均完成；第二组从 ProcessStart 首窗 5/5 小于 2 秒，从 launchCall 仅 4/5 满足 | 第 5 节保留全部原始值，不宣称所有性能目标达标 |
| 首次可编辑帧中位数缩短 30% | 第二组 ProcessStart 缩短 30.48%；launchCall 缩短 29.05%，全程目标未达到 | 第一组对应为 30.70% / 27.75%；两组及两种起点分别记录 |
| 原生视口与笔记导出 | 正式配置测试通过；32 张视口 PNG、1 张导出长图，四类页面图像已查看 | 外部 UIA 编辑、保存、重开链路仍未验证 |
| 30 次法律预览与报告帧测量 | 已完成，数据见第 6 节 | 仅表示相同绘制路径的对象持有方式对比 |
| 32 MiB 扫描准备、关闭回收和进程内存 | 已完成，所有权断言及进程退出通过 | 候选单次合成测量，见第 7 节 |
| 页面切换与编辑测量 | 已完成并通过，见第 6.2 节 | 包含真实切换和绘制的 P95；不等同于操作系统输入延迟 |
| Windows 独立发布与 verify | 发布收据 passed、publicationPerformed 均为 true；独立 verify 退出 0 | package shell 退出码未捕获，包装进程收尾异常单列于第 9.2 节 |
| 原始正式版、保留 Android 与最终发布文件核对 | 5 个发布文件、3 处 APK 哈希及修改时间、7 个旧版留档检查通过 | 第 9.1 节完整文件收据 |
| 正式 EXE 隔离业务自检 | 5 组通过，退出码 0 | 使用正式 EXE，合成业务保存与重读，不等同于外部 GUI 操作 |
| 正式程序实际窗口与退出验证 | 同字节副本两次原生进程启动与正常关闭通过，不重置资料重开前后两份主状态哈希一致 | 第 9.3 节；没有外部编辑操作 |
| 用户日常工作区最终启动 | 正式目录 EXE 已打开并激活，响应正常、资料可编辑，保持打开 | 第 9.4 节；单次全程 18.737 秒，无旧版对照 |

**结论：Windows 1.1.0.2 已正式发布，独立 verify、文件核对、正式 EXE 自检和隔离工作区实际进程重开验证通过，日常客户端已打开。** 发布前候选的可编辑帧中位数按 ProcessStart 缩短 30.48%、按 launchCall 缩短 29.05%，全程缩短 30% 目标未达到；包含启动调用的首窗 2 秒目标仍有一轮未满足。CPU 未见可宣称的改善，启动峰值内存增加。真实用户工作区这次全程加载到可编辑为 18.737 秒，没有旧版对照。正式 EXE 与性能候选的字节身份、合成与真实资料、外部 UIA 编辑链路未验证的限制，均保留各自口径。

### 9.1 独立发布、正式文件与业务自检

正式发布清单版本为 **1.1.0.2**，同步协议仍为 **1**，源码快照 SHA-256 为 `3e62121ff3a8b990b4426ecf90347dab1e991204aabb14f72831974c7ba151c0`。published_windows_build_verification.json（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/published_windows_build_verification.json`） 确认原生发布已执行且验证通过；独立 verify 日志（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/verify_final.log`） 及 退出收据（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/verify_final_result.json`） 确认独立命令退出 **0**。

package_final.log（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/package_final.log`） 中的必要测试结果逐组如下，ignored 不计为通过；33 项附件测试是核心测试中的重复过滤，不能再与核心 840 项相加宣称不重复总数。

| package 测试组 | 通过 | 失败 | ignored |
|---|---:|---:|---:|
| 核心库 | 840 | 0 | 8 |
| 桌面附件过滤 | 33 | 0 | 0 |
| Windows 客户端 | 477 | 0 | 30 |
| 同步启动器 | 52 | 0 | 0 |
| 发布器 | 62 | 0 | 2 |
| 稳定桌面入口 | 38 | 0 | 0 |

final_file_hash_verification.json（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/final_file_hash_verification.json`） 的总体结果为 `passed: true`：正式清单中的 **5 个发布文件**大小及 SHA-256 相符；Android **2.23.1.1 / versionCode 22266** 保留原版本，项目根目录、`APK`、`release_artifacts/current` 三处 APK 均为 `91271d4bb61982211d215a97e208a405ecce39a9b96e4b7cc943ff6107e567d2`，修改时间未变；原 1.1.0.1 基线及相关 **7 个留档文件**哈希核对通过。正式客户端的准确字节身份见第 2.1 节。

最终清单与稳定入口哈希（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/final_manifest_and_entry_hashes.json`） 另记录：正式 `release_manifest.json` 为 **2,527 字节**，SHA-256 `ff78e29e2e4806cdaae707e86075871aee04c63cbe3e4637c9e665074d646e26`；稳定入口 `TenRate_Desktop_Launcher.exe` 为 **2,572,800 字节**，SHA-256 `4f6441f36d58ae69ddbe64c48b821e5d81f10ca9996933c0ee40337d76a8b71b`。

直接执行正式 EXE 的 `--self-check` 后，formal_self_check_receipt.json（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/formal_self_check_receipt.json`） 记录 `passed: true`、退出码 **0**，程序哈希与正式清单相同。self_check_result.json（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/formal_self_check/self_check_result.json`） 记录 **5 组均通过**：计时原子保存与重读、日志库损坏恢复及所有者隔离、后台任务收据、中文富文本笔记更新与重读、财务与计时及笔记跨域保存。该自检使用隔离合成资料，不读取真实用户资料，也不等同于外部界面编辑或整个 GUI 进程重开验证。

### 9.2 package 输出包装进程收尾异常

原生发布器已经退出且发布收据通过后，原 PowerShell 输出包装进程 **PID 72444** 仍持续等待。独立 verify 已退出 0，核对程序身份后只结束本任务拥有的该包装进程，未触碰同步服务进程。package_wrapper_recovery.json（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/package_wrapper_recovery.json`） 记录 `nativePackagerAlreadyExited: true`、`packageShellExitCodeCaptured: false`、`syncProcessesUntouched: true`。外层工具返回 **-1**，不将其改记为 package 命令退出 0；发布成功依据是原生发布收据、正式清单、独立 verify 和最终文件核对。package 日志同时保留同步守护程序经稳定入口重新启动的记录，包装进程收尾没有再次停止这些进程。

### 9.3 正式程序同字节副本的实际进程重开验证

测量脚本禁止把正在使用的 `current` 发布槽直接作为测试程序，因此将正式 EXE 复制到隔离测试位置。published_gui_copy_receipt.json（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/published_gui_copy_receipt.json`） 确认 `probe_binaries/published_1.1.0.2.exe` 与正式客户端逐字节一致，SHA-256 同为 `86d9b637707cad9ea820a53ad3b3c234501296f16fcaf4f2367876ca448069cc`。该副本使用第 5 节隔离合成资料完成一次启动和一次不重置资料的重开，均为实际原生窗口进程。下表时间单位 ms，仍按 **ProcessStart / launchCall** 列出：

| 操作与原始记录 | PID | 首个有标题窗口 | 首次 UI 更新 | 首次可编辑帧 | 正常退出 |
|---|---:|---:|---:|---:|---|
| 首次启动（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/published_copy_FreshProcess_01/measurement.json`） | 5552 | 789 / 2,560.3057 | 717 / 2,513 | 7,073 / 8,869 | exit.json（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/published_copy_FreshProcess_01/exit.json`），true |
| 保留资料重开（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/published_copy_Reopen_01/measurement.json`） | 15108 | 1,005 / 1,012.2225 | 846 / 858 | 7,551 / 7,563 | exit.json（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/process_measurements/samples/published_copy_Reopen_01/exit.json`），true |

两次均观察到正确版本标题和可执行文件路径，响应与就绪状态正常，并通过正常 Alt+F4 关闭。首次启动从 launchCall 起首窗仍超过 2 秒，保留该实际值，不以进程起点值替代。formal_reopen_verification.json（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/formal_reopen_verification.json`） 记录 `passed: true`，不重置资料重开前后两份主状态文件的 SHA-256 分别保持一致，长度分别为 3,148,475 字节、7,346 字节。这证明该次关闭重开的资料保持与原生进程运行；本次没有外部编辑操作，未构成编辑、保存、关闭、重开的外部 UIA 全链路验证，也不加入候选 5 轮统计。

### 9.4 日常客户端最终启动与真实资料单次耗时

随后直接运行 `release_artifacts/current/grid_timer_windows_client_v1.1.0.2.exe`，使用用户日常工作区。formal_user_launch_verification.json（本机历史记录路径：`release_artifacts/verification/windows_v1.1.0.2/formal_user_launch_verification.json`） 记录 `passed: true`、**PID 67072**、正式 EXE 哈希、窗口标题 `十倍率 Windows v1.1.0.2`；正确路径和版本窗口已观察并激活，`responding: true`、`workspaceReady: true`，日志含 `writable=true`。完成验证后客户端保持打开。

此次从 launchCall 起首次 UI 更新为 **677 ms**，首次可编辑帧为 **18,737 ms，即 18.737 秒**。前者是应用更新标记，不是屏幕像素呈现时间；后者是**真实用户工作区单次**完整加载记录，未进行旧版对照，不能宣称该工作区提速了多少。第二组合成测量使用 128 篇笔记、约 3.15 MB 主状态与 69.49 MB 历史 SQLite，其候选五轮中位数约 7.2 秒是另一数据范围，不能写成此次实际资料的加载时间。该次日常启动只收集阶段时间、窗口与程序身份及就绪状态，没有编辑或导出用户内容，也未通过外部 UIA 执行编辑保存操作。
