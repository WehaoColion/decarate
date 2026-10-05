版本号更新为：1.1.0.4，完成本轮性能优化、固定桌面入口与跨PowerShell环境发布修复。

# Windows 1.1.0.4 最终验收记录

日期：2026-10-02。当前正式版本为1.1.0.4，已提交 `release_artifacts/current/release_manifest.json`。实际桌面与运行验收于北京时间11:46完成。

## 版本过程

本轮性能、图标和桌面入口改动首先通过1.1.0.3完整发布测试，正式文件与清单成功提交；最后的桌面维护遇到子进程模块路径兼容问题，发布命令返回失败。直接运行经过核对的维护脚本后，真实 `TenRate.lnk` 已修正，1.1.0.3服务健康身份也已核对。当前没有把最初的非零发布退出码记录为成功。

问题可稳定复现：Rust进程继承PowerShell7的模块路径，Windows PowerShell5优先加载不兼容的Utility模块，导致 `Get-FileHash` 不存在。由PowerShell7直接启动Windows PowerShell时会自动净化模块路径，因此先前普通调用没有触发该边界。

修复仅对维护子进程移除 `PSModulePath`，让Windows PowerShell恢复自身模块路径，并明确UTF-8 stdout/stderr及失败退出码。完整路径使用转义后的字面量，未更改用户的全局环境。1.1.0.3已经提交，后续修复按规则递增第四段为1.1.0.4。

## 实际边界变异

使用完整 `windows_desktop_entry.rs`、Rust1.95.0、rust-lld、xwin及标准库，在独立目录编译三个副本，没有改动Cargo target。每组编译成功。

|源码副本|Rust测试退出码|结果|
|---|---:|---|
|未修改|0|1项通过|
|仅说明文字修改|0|1项通过|
|删除子进程模块路径隔离|101|关键业务断言失败，实际Get-FileHash不可用|

正常副本还验证含单引号的真实目录、实际文件哈希、中文与emoji的UTF-8输出及异常非零退出码。生产helper源码前后SHA-256一致：`e4cdf423b9ce665e4732e9d5bd9b65738964c5aa483f861ebc328de8b24e1411`。回执位于 `release_artifacts/verification/windows_v1.1.0.4/desktop_entry/rust_process_boundary_mutation_result.json`。

## 性能证据的范围

本版保留全部性能优化。1.1.0.4相对于1.1.0.3仅修复桌面维护子进程、调整Windows版本身份及新增当版历史。六份UI热点、两份存储热点和四项图标资产已与1.1.0.3冻结记录核对SHA-256一致，额外计时内容版本和相关测试源也一致；记录位于 `release_artifacts/verification/windows_v1.1.0.4/hotspot_source_identity.json`。

局部测量详见 [1.1.0.3性能与验证](windows_deep_optimization_v1.1.0.3.md)。该记录包含实际release测试程序身份、24次知识读取样本、完整冷重建成本及12次存储API打开样本。较大资料P50有改善、P95回退6.40%的事实继续保留。不能将这些数据宣称为1.1.0.4整个客户端的冷启动测量。

## 正式发布

本版规定格式检查已通过，覆盖218份共用及Windows Rust源文件。全部发布测试实际通过：

|测试命令|通过|失败|忽略|运行时间|
|---|---:|---:|---:|---:|
|核心库|865|0|8|201.73 s|
|桌面附件及同步子集|33|0|0|0.22 s|
|Windows客户端|516|0|34|92.35 s|
|同步启动器|52|0|0|2.02 s|
|发布器及入口维护子进程|66|0|2|2.35 s|
|稳定桌面入口|38|0|0|0.35 s|

子集与核心库有重叠，不相加为独立测试数量；忽略项没有计为通过。完整日志位于 `release_artifacts/verification/windows_v1.1.0.4/windows_package_final.log`。

release服务、启动器和客户端已构建并原子提交，桌面维护在正式流程中自动完成。独立执行正式发布器的 `finish --windows-only --validate-only` 实际退出码为0，完整性链及保留APK的签名验证通过；日志位于 `independent_release_verification.log`。

外层PowerShell/Tee输出包装会话在发布器退出、自动桌面维护完成后仍未自然结束，没有取得该包装会话的成功退出码。核对其PID37500、创建时间、可执行路径、完整命令及已无编译/发布进程后，11:49仅定点结束该任务包装进程，工具会话最终返回-1。具体输出管道原因尚未确定，不将其记为package退出0。同步启动器、服务及客户端仍保持原PID运行。正式成功依据是上述独立发布器复验退出0、发布清单与真实桌面运行验收；处理回执位于 `package_wrapper_completion.json`。

正式源码快照SHA-256：`b54f949cb49129d61959a9e060fa36c7572e8973d73c8be6f92bc37d0121b6fd`。客户端SHA-256：`8501c1c8dc0c94cae7192d60e39c2e3539546d65707c558b9dd460054318dffa`。服务、启动器、客户端、APK和cloudflared五项文件的实际大小与哈希均匹配正式清单。

真实桌面链接为 `<local-desktop-path>`，唯一目标为无版本的 `release_artifacts/desktop_entry/TenRate_Desktop_Launcher.exe`，图标从该程序的原生图标资源读取。维护检查解析到current中的1.1.0.4客户端。本次原来只有一个已归属的TenRate链接，已经原位更新；没有额外的重复旧客户端链接可删，其他22条桌面链接保留。

从该真实链接首次启动后，唯一客户端PID为85944，日志明确记录 `START version=1.1.0.4`、首帧和 `STARTUP_READY_FRAME writable=true`。再次点击同一个链接，最终仍是PID85944，临时第二进程记录 `duplicate_instance running_version=1.1.0.4 reactivated` 后正常退出。客户端已保留打开。此次核对的是实际进程与日志，未目视确认整个客户端界面外观。

该次真实工作区就绪日志记录8590 ms；只有一次观察，没有旧版本配对样本，不作为性能基准，也没有以局部首帧2 ms宣称完整启动只需2 ms。本轮已测量的存储和知识读取优化范围见前文，实际工作区启动仍有进一步减少耗时的空间。

同步启动器PID35172与服务PID68048均来自current的1.1.0.4文件，服务父进程为该启动器，8917监听所属PID为68048。`Invoke-RestMethod -NoProxy` 和直接curl读取的健康结果包含1.1.0.4、正确产品与服务角色、Git身份及上述源码快照指纹，备份状态为ok。首次健康请求8秒超时，后续直接读取和重复读取通过，保留这次观测；这不是长期网络稳定性结论。

Android保持2.23.2、versionCode22269。项目根目录、APK目录及current三处正式APK均为23,121,253 B，SHA-256为 `c7adead385d2bc79dfe6d593a0edcfd9834fb93b7d58c7c41f995c62417aab84`，UTC修改时间ticks `639264991125882326` 完全保留。详细回执位于 `published_acceptance.json`，字段 `passed` 为true。

正式流程启动时Windows doctor检查约12.3 GiB可用空间，通过12 GiB构建门槛。为保留资料并恢复空间，本版之前对66个已确认的构建缓存文件执行NTFS LZX压缩，每个文件压缩前后SHA-256一致，没有删除；回执位于 `retained_cache_compression.json`。11:46复核C盘约12.22 GiB可用，11:51最终核对为11.24 GiB，空间数值会随本机其他写入变化。最后一次服务健康身份及交付链接存在性核对通过，回执位于 `final_closure.json`。最初两处已授权的incremental缓存在用户提出回收站方案前已经删除，此后没有再永久删除缓存；后续清理遵循用户的回收站要求。
