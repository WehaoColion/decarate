v0.0.3 - 更新文档文本块居中修订的版本与验收说明。

## 目录导航

| 内容 | 位置 |
| --- | --- |
| Android 与 Windows 更新说明 | [全部版本说明](documents/releases/README.md) |
| 使用说明、验收与性能记录 | [文档索引](documents/README.md) |
| Rust 源码与 Android 源码生成器 | `native/gridtimer_native/` |
| Android 工程与必要资源 | `app/` |
| 构建、发布及验证工具 | [工具索引](tools/README.md) |
| 公开发布元数据 | [正式发布清单](release_artifacts/current/release_manifest.json) |
| 本机安装包交付 | 原电脑的项目根目录、`APK/` 和正式构建输出 |
| 本机历史安装包与私人归档 | `old_apks/`、`old_exes/`、`local_archive/`，不随源码上传 |

此仓库保留完整应用源码、必要资源和可共享说明。安装文件、原始设备证据及私人材料按既有规则保留在本机；历史记录中的本机路径用文字标注。版本、功能、界面和数据格式保持原状。

v2.23.2.13（Android候选）- 标题和正文在文档文本块内上下居中，让键盘避让跟随实际文本坐标；保留多行自然增高、非文本块位置及此前段落预览修复。沿用正式签名与原APK流程，Windows保持1.1.0.7，手机验收待完成。详见[本版说明](documents/releases/android/release_notes_v2.23.2.13.md)。
v2.23.2.12（Android已验收候选）- 修复知识文档预览合并独立正文段落的问题，保留代码、公式和列表的连续边界；沿用正式签名与原APK流程，Windows保持1.1.0.7，2026-10-05 22:20 用户已确认本次问题修复。详见[本版说明](documents/releases/android/release_notes_v2.23.2.12.md)。
v2.23.2.11（Android候选）- 风控按总览、核对、预测分区，长清单在独立详情中处理；保留原核对、预测和法律发送确认，沿用正式签名与原APK流程，Windows保持1.1.0.7。详见[本版说明](documents/releases/android/release_notes_v2.23.2.11.md)。
v2.23.2.10（Android已验收候选）- 精简知识列表顶部，修复文档换行后的光标与键盘避让；沿用正式签名和原APK流程，Windows保持1.1.0.7；用户已确认真机测试通过并授权合并PR #4与#5。详见[本版说明](documents/releases/android/release_notes_v2.23.2.10.md)。
v1.1.0.7（Windows正式版）- 补齐直接问 AI、知识库问答与法律资料发送确认，改进离线 Markdown 和公式阅读；正式程序已在本机发布，公开仓库同步源码、发布元数据与验收范围。详见[本版说明](documents/releases/windows/release_notes_windows_v1.1.0.7.md)。
v2.23.2.9（Android候选）- 接入PR #3，让法律分析入口与主操作常显，增加与扫描和AI配置绑定的一次性发送确认；沿用正式签名与原APK交付位置，手机及真实模型验收待完成，Windows保持1.1.0.6。详见[本版说明](documents/releases/android/release_notes_v2.23.2.9.md)。
v2.23.2.8（Android候选）- 接入PR #2，修复法律风险扫描失效后持续忙碌与无法重试的问题；使用原正式签名生成release APK，手机验收待完成，Windows保持1.1.0.6。详见[本版说明](documents/releases/android/release_notes_v2.23.2.8.md)。
v2.23.2.7（Android）- 修复回答本地排版加载失败，补齐知识页公式预览与原文切换；已通过连接手机的真实 DeepSeek 调用、公式画面和原文完整性检查，正式签名 APK 已发布，Windows 保持 1.1.0.6。
v2.23.2.6（Android）- AI 回答支持离线公式和 Markdown 排版，新增复制原文，保存后的知识页可预览并编辑原文；正式签名 APK 已发布，Windows 保持 1.1.0.6。本轮未进行真机复测。
v2.23.2.5（Android）- 新增默认的直接问 AI，仅发送问题；知识库模式继续核对来源，修复模式切换和取消后的状态，明确上次同步状态与 AI 独立接口；正式签名 APK 已发布，Windows 保持 1.1.0.6。本轮未进行真机复测。
v2.23.2.4（Android）- 修复 AI 知识问答输入框的键盘唤起及再次点按恢复，补齐窗口焦点、生命周期和延迟请求状态保护；正式签名 APK 已发布，Windows 保持 1.1.0.6。本轮未进行真机键盘复测。
v2.23.2.3（Android）- AI 功能入口常显，知识问答展示实际正文和请求依据，修复空回答与未完成响应误报成功、取消后待发请求和同步内部英文错误；正式签名 APK 已发布，Windows 保持 1.1.0.6。
v2.23.2.2（Android）- 新增 DeepSeek 快捷配置、密钥粘贴与明确的连接测试，测试只发送合成资料，阻止空密钥、重复请求和过期回执；正式签名 APK 已发布，Windows 保持 1.1.0.6。
v1.1.0.6（Windows）- 修复无效金额仍能按旧数保存或核对的问题，补齐快捷保存和退出保护，知识内容可独立保存；已完成实际窗口复测、正式发布和固定桌面入口验收，Android 保持原版。
v2.23.2.1（Android）- 合并连续自动保存的待写草稿，将退出恢复副本写入移到后台；财务金额支持两位小数并保护保存、备份和同步精度，已完成正式签名 APK 与发布复验。
v1.1.0.5（Windows）- 客户端与同步服务兼容整数分字段，精确编辑及计算两位小数；继续通过一个 TenRate 桌面入口打开最新正式版，已核对实际运行身份。
v1.1.0.4（Windows）- 完成本轮性能优化和固定桌面入口发布，修复跨 PowerShell 环境的入口维护与中文输出；桌面使用同一个 TenRate 图标打开最新正式客户端。
v1.1.0.3（Windows）- 减少工作区启动校验、隐私准备、知识导航和列表的重复计算，按计时内容复用读模型；新增安全版本交接与统一的数字10上升图标。
v2.23.2（Android）- 知识筛选改为文档库、类型和排序三个入口；新增显示全部与零结果恢复，按工作区隔离筛选状态，并对原生分组及排序增加完整覆盖检查。
v2.23.1.3（Android）- 当前版本单独显示，旧版说明放入“历史版本”入口；更新时间按北京时间精确到分，保留点按复制和整版复制。
v2.23.1.2（Android）- 更新说明支持点按摘要、标题或正文复制，并新增“复制本版”按钮；保留此前版本记录。
v1.1.0.1（Windows）- 修复已登录空报告工作区启动时，法律报告同步重复请求清单导致的客户端退出。
v1.1（Windows）- 风控改用经核对的财务判断，新增法律风险线索与加密报告仓库；法律报告可与 Android 在同一账户和工作区同步，服务数据库升至 schema 17；“我的”页面新增仅记录本次内容的可折叠更新说明。
v2.23.1.1（Android）- 法律报告新增跨端同步、待同步重试和删除记录同步；保留 2.23.1 的应用内更新记录。
v2.23.1（Android）- “我的”页面新增可折叠的更新记录；本次只写 2.23.1 的内容，此后每次安卓更新新增当次说明并保留已写入的版本。
v2.23（Android）- 风控按月份核对收支、现金、资产和负债，资料不足时显示待核对；新增法律风险线索入口，可在发送前查看范围与内容，并将证据报告加密保存在本机。

v2.22.49.13（Android）- 键盘弹出时保留便签纸张顶部、标题与正文首行，输入期间将版本操作收成细分隔线。

v2.22.49.12（Android）- 调整便签键盘弹出时的首行留白，六个格式操作以紧凑单排显示。

v2.22.49.11（Android）- 将便签输入时的草稿落盘移出界面线程，减少本地快照保存的重复解析。

v1.0.3.17（Windows）- 计时启停交由后台保存，点击时立即反馈并阻止重复提交；修复同步保存重试时钟和页面切换的无谓缓存失效。

v1.0.3.16（Windows）- 降低运行中计时、知识列表和编辑器的重绘开销，复用画廊预览与托盘提示计算。

v1.0.3.15（Windows）- 修复短暂离线后已登录会话过期导致附件清单报错、同步中断的问题。

v1.0.3.14（Windows）- 减少知识画布和内容块重绘的正文复制，复用资料库画廊预览解析；保持编辑、账户和加密边界。

v1.0.3.13（Windows）- 画布显示当前名称与全部画布入口，支持复制、确认删除和卡片导航；修复空白标题兼容并降低连线绘制与自动排布开销。

v2.22.49.10（Android）- 多语言覆盖计时、便签、知识画布、结构化知识页、风控和诊断界面；十种语言资源随正式 APK 离线提供，并保护用户输入内容不被翻译。

v1.0.3.12（Windows）- 减少便签摘要与清单统计的整篇正文临时复制，并加快资料库文件夹计数。

v1.0.3.11（Windows）- 持续使用的同步会话在到期前续期；近期活跃的手机令牌刚过期时，通过原令牌证明安全恢复同步。

v2.22.49.9（Android）- 在“我的”页面加入语言切换器，提供十种总使用人数最多的语言；补齐设置页与底部导航的对应译文。

v2.22.49.8（Android）- 压缩并认证本机历史快照，复用相同数据的启动解析；真机连续五次冷启动读取等待均低于一秒。

v2.22.49.7（Android）- 缓存未变化的连线并批量绘制，缩放文字按需更新，减少拖动时的重复计算与临时对象。

v2.22.49.6（Android）- 在画布顶部固定显示新建和全部画布入口，标出当前画布，完善创建上限与切换失败后的重试。

v2.22.49.5（Android）- 减少笔记筛选、文档排序和知识页摘要的重复计算；画布平移、缩放复用卡片与连线，降低内存分配和回复数据量。

v2.22.49.4（Android）- 增加触屏知识画布、卡片与连线，完善保存和账户隔离，减少拖动时的复制与绘制开销。

v1.0.3.10（Windows）- 修复知识属性输入丢失、表单校验遗漏和画布键盘删除越过编辑限制，完成电脑端测试与正式发布核验。

v1.0.3.7（Windows）- 清理删除时间之前的历史笔记附件引用，解除遗留删除冲突并保留当前引用保护。

v2.22.49.3（Android）- 修复知识文档编辑时键盘弹出后格式栏遮挡正文。

v1.0.3.6（Windows）- 忽略与附件无关的财务档案冲突，恢复媒体引用核对并完成删除同步。

v1.0.3.2（Windows）- 隔离文档撤销记录，回收编辑缓存，修复明确恢复附件的同步预检。

v1.0.3.1（Windows）- 修复主题保存恢复、账户切换后的保存失败继承和加密会话隔离，完善独立发布校验。

v2.22.49.2（Android）- 修复计时账户绑定、便签解锁失效、风控保存回执和历史列表刷新回跳，完成真机覆盖安装与验收。

v1.0.1（Windows）- Windows 从 1.0.1 开始独立编号，客户端、同步服务和后台守护程序统一版本；安卓更新不递增 Windows 版本。

## 当前版本与编号规则

当前正式 Android 版本为 **2.23.2.7**，versionCode 为 **22276**，安装包为 tenfold_v2.23.2.7.apk（签名安装文件仅在本地保留）。详见 [Android 发布说明](documents/releases/android/release_notes_v2.23.2.7.md)。本版已在 Vivo V2352A 真机核验普通 Markdown、保存后的知识页预览、上标与公式、原文比对和重新打开；最终 APK 的实时 DeepSeek 提问及官方用量增长见 真机验收（原始验收记录仅在本地保留）。

此前 **2.23.2.3** 于 2026-10-03 完成 DeepSeek 真机知识问答验收（原始验收记录仅在本地保留）：实际随机资料问答、官方用量增量、回答保存及重新打开均已核对。仅发送一个合成知识来源，未读取密钥或改动 VPN。该记录属于 2.23.2.3，不替代后续版本的真机复测。

当前正式 Windows 版本为 **1.1.0.7**，独立版本序列从 **1.0.1** 开始。客户端、配套同步服务、启动器、界面显示、发布文件及清单使用同一个 Windows 版本号。安卓保留自己的版本序列和 versionCode，两端分别发布。桌面固定入口为 `TenRate.lnk`，指向无版本的稳定启动器，按已提交的正式清单打开最新客户端；本版已通过该入口启动实际窗口并核对显示版本。

后续每次发布只递增 **0.0.0.1**，固定前三段、递增第四段；已有三段版本的第四段按 0 处理。Windows **1.1** 是用户指定的例外，当前版本 **1.1.0.7**，下一版为 **1.1.0.8**；Android **2.23.2** 为用户指定的编号，当前为 **2.23.2.7**，下一版为 **2.23.2.8**。第四段持续递增，不进位到第三段；现有正式版本和历史记录保持原号。

实际安装版本以 `release_artifacts/current/release_manifest.json` 为准，候选构建不代表正式发布。应用版本号独立于同步协议和数据格式；重新编号保留账户、数据目录、手机配对及登录自启动任务。详见 [Windows 1.1.0.7 发布说明与验收范围](documents/releases/windows/release_notes_windows_v1.1.0.7.md)、[Windows 1.1.0.6 发布说明与实际交互验收](documents/releases/windows/release_notes_windows_v1.1.0.6.md)、[Windows 1.1.0.5 发布说明](documents/releases/windows/release_notes_windows_v1.1.0.5.md)、[此前性能发布验收](documents/windows/windows_deep_optimization_v1.1.0.4.md) 和 [此前性能与边界验证](documents/windows/windows_deep_optimization_v1.1.0.3.md)。

## 历史更新记录

以下记录保留当时的版本号；Windows 的旧 2.x 编号不再用于后续发布。

v2.22.49.1 - 修复安卓计时阶段交接时漏响，铃声跟随系统闹钟音量，保留暂停停止和单次提醒保护。
v2.22.49 - 逐行读取历史快照并复用校验缓冲区，记录主界面完整呈现时间。
- v2.22.48.1（Android）：以 Rust 流式验证本机快照，减少启动时历史数据的大字符串复制与重复读取；校验回执仅在同一事务与工作区内生效，异常数据仍走完整恢复流程。
- v2.22.48-rest-bell（Android）：暂停时停止铃声并作废待播放请求；提醒绑定本机计时批次与当前阶段，丢弃过期提示；修复阶段更新取消自身提醒的问题，15 秒休息采用精确闹钟。
- v2.22.51-windows-audit（Windows）：复用相同历史内容的解析结果，减少启动与自动保存的重复计算；同步累计时长时保留已经到期的本机提醒，补齐状态边界、篡改回退和性能对比。
- v2.22.50-windows-audit（Windows）：修复无关同步清空撤销、退出旧版本查看和重锁加密文档的问题；修复远端累计时长误触发电脑间隔提醒，补齐状态边界回归及正式客户端验收。
- v2.22.49-device-timers（Windows）/ v2.22.46-device-timers（Android）：同步时保留本机计时启停、当前批次与休息提醒进度；手动下载和账户恢复采用同一规则，同时开始的计时记录使用独立批次。
- v2.22.48-risk-navigation（Windows）：主导航按安卓统一为计时、便签、知识、风控、我的；计时记录与归档收回计时模块，补齐返回详情和 Esc 返回；风控默认打开总览，概览页名称与安卓一致。
- v2.22.47-android-parity（Windows）：以安卓为功能原型，补齐计时详情、每日计次、拖动排序，便签文字与长图分享、格式和纸张设置，知识目录，完整财务概览、语言选择、通知区后台运行及诊断收集。
- v2.22.46-ui-refinement（Windows）：统一侧栏、配色、字体和控件状态，重排计时摘要与等高卡片，优化便签、知识、历史、财务及小窗口布局。
- v2.22.45-local-startup（Android）：启动复用已验证且未变化的本机数据，同一事务内避免重复校验，移除启动时全库附件清理，保留恢复与防覆盖保护。
- v2.22.41-knowledge（Windows）：修复文档斜杠输入、重复选择丢失撤销、新建文件夹归属、回收站筛选与恢复位置；保存前校验问答来源，压缩编辑界面并补齐关键状态回归。
- v2.22.44-knowledge-resume：知识文档与便签切换后台应用后保留原页面；只对已解锁的加密文档执行后台锁定，回到前台或离开页面时取消过期的延迟任务。
- v2.22.43-pause-layout：固定计时卡状态区和详情页操作栏的尺寸；运行时归档按钮保持占位并禁用，暂停后按钮不再上移。表盘保留暂停时的实际进度，计时状态变化不再触发列表位置弹动。
- v2.22.42-timer-response：计时启停只处理当前运行批次及其记录，保留其他历史对象；铃声与闹钟调用移出主线程。历史快照在事务中逐条读取校验，避免游标反复回扫；微休息广播超过 8 秒结束本次回执并安排重试。新增状态边界回归和真机点击到显示的测量。
- v2.22.39-windows-polish：计时卡增加明确启停按钮，归档显示备注与恢复位置，重置绑定当前任务；修复便签回车与尾部空格丢失、回收站恢复导航和富文本保存误报，保留手机端格式与元数据；财务按用途分页并显示即时合计，完成 Windows 独立发布验收。
- v2.22.38-windows-refinement：补齐历史记录删除、格子筛选与直达编辑，增加季度和年度财务汇总；小窗口便签直接进入正文、跨模块记住文档；修复编辑弹窗误触、任务回执串单、旧清单导致快捷入口启动失败，新增隔离数据自检。
- v2.22.37-note-save：便签保存和版本创建只计算当前便签，保存前检查改为读取历史快照的版本头，减少全量编解码与备份重复读取。
- v2.22.36-android-timer：优化安卓计时启停与保存，复用已验证的存储检查，后台计算详情和历史搜索，暂停被覆盖卡片的逐秒刷新。
- v2.22.35-archive-restore：安卓归档恢复保存成功后直接打开目标计时格子，修正排序后的恢复编号，阻止重复提交，保留累计时长。
- v2.22.35：重做 Windows 计时台和历史筛选，对齐微休息、分类、排序与归档恢复；补齐财务编辑及备份、富文本编辑、完整 HTML 导出与本机音视频转写；修复草稿并发覆盖、中文提交、长路径保存和点击命中，降低空闲占用。
- v2.22.34：修复旧附件缺失阻断计时与文字同步的问题，保留附件恢复提示，并准确返回同步失败原因。
- v2.22.33：修复真机历史快照恢复内存耗尽，导出时保留现有数据并明确标记缺失附件。
- 2.22.32-diagnostics-collection：增加开始收集、停止收集和持久收集状态，按会话保留操作日志，收集与导出分别控制。
- 2.22.31-diagnostics-export：修复数据库备份查询方式，诊断日志可直接导出并保留上次崩溃信息，增加独立文件保存、分享重试与导出状态恢复。
- 2.22.30-android-responsiveness：移除计时运行态循环装饰动画，缓存表盘静态绘制，后台恢复账户与统计历史，按数据域复用状态，诊断日志改为有界异步写入，并启用 Compose 强跳过以减少无关界面重组。
- 2.22.29-persistence-recovery：拆分旧数据迁移与快照读取异常边界，迁移落盘失败时继续读取已验证的本地数据，并在主存储异常时使用只读恢复副本，避免默认空数据覆盖持久化证据。
- 2.22.28-android-rollback-performance：Android 功能基线回退到 v2.22.25，仅保留绘制阶段脉冲、后台纯计算、微休息补算上限和 release 诊断门控，修复新版功能偏移带来的卡顿与 native 协议错位风险。
- 2.22.27-android-stability：把计时器 CPU 工作移出界面线程，限制微休息补算并支持精确续算，严格校验 native 数组边界，同时缓存表盘静态几何以降低长期运行卡顿与闪退风险。
- 2.22.26-render-hotpath：把运行态脉冲刷新限制在绘制与图层阶段，固定状态灯和底栏标记的布局尺寸，并为诊断日志和微休息监控增加有界异步与停滞保护。
- 2.22.25-windows-runtime-safety：电脑端建立服务端到启动器再到客户端的三级完整性绑定、同步服务身份校验、后台任务恢复日志、安全退出状态机和正式发布闸门。
- 2.22.24-windows-notes-fix：电脑端便签改回简洁记录界面，修复便签卡片无法点击、旧版本搜索后打不开、便签回收站缺失，以及历史版本与恢复记录混淆的问题。
- 2.22.23-sync-resource-guard：把当前快照媒体身份、全局资源配额和迁移修复豁免统一纳入原子事务；修复历史绕过、容量放大、清理自锁及备份失效后误放行风险。
- 2.22.22-sync-atomicity：修复附件缺失误判成功、预上传期间并发编辑越过媒体闸门，以及媒体身份与保留数据可被放大的问题。
- 2.22.21-sync-integrity：修复重复启动无反馈、恢复后的媒体读取屏障、附件先后顺序、离线删除因果与 Android 公网发现回滚问题，并更新简约图标。
- 2.22.20-windows-stability：修复 Windows 草稿丢失、媒体事务与恢复、旧版并发写入、同步备份和开机入口问题，补齐附件加密保护、小屏布局、运行日志、极简图标与正式发布测试闸门。
- 2.22.19-windows-knowledge：电脑端品牌统一为“十倍率”，新增独立知识模块并对齐资料库、文件夹、筛选、回收站、版本、附件、加密和知识问答能力。
- 2.22.18-database-sync：统一跨端数据合并与恢复规则，修复附件历史、并发覆盖、账户隔离和数据库升级完整性。
- 2.22.16-risk-semantics：风控结论加入未知状态与原因码，缺少支出基线时不再显示已覆盖，并限制异常金额数量级、固定财务枚举协议码。
- 2.22.15-sheet-swipe-dismiss：格子详情顶部拖拽区支持跟手下滑关闭，超过距离阈值或快速下甩即退出，未达到阈值自动回弹。
- 2.22.14-knowledge-canvas：知识文档改为单画布块编辑，插入、属性、文字和文档操作按当前上下文出现，并补齐保存状态、斜杠命令与输入撤销合并。
- 2.22.13-my-page-lite：精简“我的”页，首屏只保留账户、外观和 AI 设置；同步地址、设备名、手动迁移及模型参数改为按需展开。
- 2.22.12-auth-response-binding：待激活登录令牌获取恢复基线时，服务端把响应身份稳定绑定到请求令牌，避免首次公网同步被令牌编号校验拦截。
- 2.22.11-public-sync-fallback：自动同步优先采用最新签名公网入口，弱网公告读取放宽，并把打包时已验证的公网地址写入正式 APK 作为兜底。
- 2.22.10-baseline-sync-recovery：公网大快照读取容限提升至 90 秒；恢复基线只有在手机安全落盘后才确认，并在传输或落盘失败时按 30 秒冷却有界重试。
- 2.22.9-auto-sync-recovery：手机启动、回到前台或停留在“我的”页时有界重试普通同步，自动避开强制下载无法恢复旧会话的死路；保留完整令牌原子激活与公网隧道自愈。
- 2.22.8-legacy-session-recovery：旧版手机无需更新或重新登录，普通同步凭完整令牌在成功提交事务内安全恢复待激活会话；保留公网重试和 Quick Tunnel 分块请求修复。
- 2.22.5-cross-network-sync：电脑端自动签名发布当前 HTTPS 同步入口，手机使用 Wi-Fi、5G 或 VPN 时均可跨网发现，无需连接同一局域网。
- 2.22.4-sync-discovery-bootstrap：修复 VPN 环境下桌面公网入口不发布和未登录状态无法自动发现同步服务的问题。
- 2.22.3-timer-dial-control：移除计时卡片底部的开始、继续和暂停按钮，点击卡片时钟即可直接启停计时。
- 2.22.2-timer-copy-trim：删除计时卡片的重复说明，只保留用户备注和计次、时长等事实信息。
- 2.22.1-timer-card-text-fit：计时卡片改为内容驱动高度并重排窄屏信息区，完整显示标题、分类、状态和操作按钮。
- 2.22-night-mode：重做深色与 OLED 纯黑主题，统一语义色、表面层级和文字对比度。
- 2.21.36-timer-index-horizontal：计时模块编号保持单行横向显示，避免“01”等两位编号在窄卡片内上下换行。
- 2.21.35-release-security-hardening：修复 AI 地址混淆导致的凭据外传、富文本持久型脚本注入与带毒导出，并收紧 WebView 网络、媒体导入、云备份、权限和正式发布信息。
- 2.21.34-protection-state-safety：补齐便签加密状态的单调版本、改密事务、退出写入屏障、恢复语义校验与正式产物一致性门禁。
- 2.21.33-note-save-recovery：修复便签落盘任务被自动保存取消的问题；主数据处于只读保护或暂时无法写入时，使用已校验的本地恢复日志安全保留编辑内容，并记录明确失败类型。
- 2.21.32-note-version-stack：为同一便签增加显式创建、横向堆叠、历史只读和旧数据无损迁移的完整版本系统。
- 2.21.31-simple-note-editor：按纸张编辑思路简化便签页，收拢顶栏、元信息与键盘格式工具，次要设置移入弹层。

- 2.21.30-note-save-recovery：修复便签主数据库已提交却被恢复镜像误报为保存失败的问题，并让已验证的恢复快照完成自愈后恢复写入。

- 2.21.29-risk-cockpit：重做风控驾驶舱，新增可支配余量、固定账单预留、预算余量、现金流预测、压力测试、异常与重复支出扫描。

# TenRate（十倍率）

TenRate is a personal time, notes, and life-record system built with Rust for Android and Windows.

It is designed for people who want one place to hold the shape of a day: focused work sessions, quick notes, longer records, useful documents, sync state, and the small traces that are easy to lose when they are scattered across separate apps. The timer is the entry point, but the larger goal is a private operating desk for personal rhythm, memory, and review.

The Android app is generated from the Rust source tree, uses native Rust logic through JNI, and includes focus timing, note management, finance records, sync support, notification effects, and Xiaomi HyperOS notification experiments.

## Project layout

- `app/` contains the Android Gradle project and static Android resources.
- `native/gridtimer_native/` contains the Rust crate, JNI bridge, generated Android source emitters, desktop client, sync server, launcher, packaging helpers, and source audit tools.
- `release_artifacts/current/` publishes the release manifest and the Cloudflared runtime dependency. Signed application installers remain local pending a decision about embedded workstation build paths. `release_artifacts/desktop_entry/` publishes the icon generator and icon assets; all stable launcher sources are included under `native/`.
- `old_apks/` and `old_exes/` are local archives; they are not included in this baseline.
- `documents/` contains indexed release, Android, Windows, sync, acceptance, and design documents. Private historical materials remain in ignored `local_archive/` on the original workstation.

## Build requirements

- JDK 17
- Android SDK with API 35 (compile SDK); the main app targets API 34
- Android NDK 27.1.12297006
- Rust 1.95 or newer with the `stable-x86_64-pc-windows-msvc` toolchain
- `cargo-ndk`
- Git LFS for binary release artifacts

The current Gradle script expects the local Rust and Android toolchain paths used by the original workstation. If your paths differ, update `app/build.gradle` or provide equivalent local tooling paths before building.

## Local configuration

The repository intentionally does not include local signing credentials or machine-specific configuration. Create these files locally when needed:

- `local.properties` with `sdk.dir=...`
- `release_signing.properties` with `storeFile`, `storePassword`, `keyAlias`, and `keyPassword`
- a release keystore matching your own signing setup

Copy `local.properties.example` and `release_signing.properties.example` to their local configuration filenames, then fill in your own paths and credentials. The examples contain placeholders only. Do not commit keystores, passwords, API keys, personal machine paths, or generated setup reports.

## Build commands

All Windows checks and releases use one entrypoint. The script resolves the installed MSVC toolchain, Rust linker, and xwin libraries without changing `RUSTUP_HOME`. From the repository root, run:

```powershell
.\tools\windows.ps1 doctor
.\tools\windows.ps1 check
.\tools\windows.ps1 test
.\tools\windows.ps1 package
.\tools\windows.ps1 verify
```

The Windows entrypoint packages Windows only. It verifies the retained signed Android APK and runs every required Windows test suite, then builds the sync server, embeds that server's size and SHA-256 into the launcher, and embeds the completed launcher's size and SHA-256 into the Windows client. The stable desktop launcher and source-bound release manifest are published atomically. Android version, APK bytes and modification times remain unchanged; no Gradle or Android build runs. The manifest explicitly records independent Windows and Android versions, while unexpected mixed versions and extra executables are rejected. Use `GRIDTIMER_WINDOWS_TEST_TARGET_DIR` and `GRIDTIMER_WINDOWS_RELEASE_TARGET_DIR` to reuse separate existing test and release caches when needed.

For focused development from `native/gridtimer_native/`, use `rustup run stable-x86_64-pc-windows-msvc cargo ...`; the formal delivery path remains `tools/windows.ps1 package`.

```powershell
rustup run stable-x86_64-pc-windows-msvc cargo test --locked --offline --lib
rustup run stable-x86_64-pc-windows-msvc cargo run --locked --offline --bin timer_sync_server
rustup run stable-x86_64-pc-windows-msvc cargo run --locked --offline --features desktop --bin timer_windows_client
```

## Public artifacts

Current release metadata and the vendor runtime dependency are stored in `release_artifacts/current/`. Signed app installers are currently retained only on the original workstation. Historical APKs and Windows executables remain in local archives. Current public APK/EXE binaries use Git LFS. Personal Word/PDF documents and raw device evidence are excluded from this baseline. Some binary filenames retain the earlier `grid_timer` prefix for release compatibility.

## Files intentionally not published

The repository excludes generated build outputs, Gradle and Cargo caches, local temporary folders, signing credentials, keystores, local SDK paths, generated setup reports with personal paths, and tool-specific traces.

## Baseline publication scope

The 2026-10-04 baseline includes all current Rust application sources, Rust-generated Android UI sources, required static resources, dependency locks, reusable build and verification tools, shared documentation, and current release metadata. Signed app APK/EXE files remain local pending the separate build-path privacy decision. Independent clock/calendar applications are separate projects and remain local, together with their generated APKs. No application logic or data format was changed for this synchronization.

Signing credentials, API keys, personal records, thesis documents, raw device diagnostics, local backups, temporary edits, caches and historical installers stay local. The public release manifest retains its schema and hashes but omits the workstation-specific linker home path. Earlier published Git history is retained; removing a personal file from the current tree does not erase its historical copies.

For a fresh checkout, configure the local SDK and signing files, build the signed Android release using `tools/android.ps1 -Mode build`, then use the Windows packaging entrypoint. Verification and packaging require locally built formal artifacts; application unit tests and source generation tests do not require private signing credentials.

## Windows 1.1.0.7 source synchronization

The Windows 1.1.0.7 source and shared release metadata match the locally published build. See [release verification and limits](documents/windows/windows_release_verification_v1.1.0.7.md). Signed application installers remain local because their compiled dependency metadata contains workstation-specific user paths; no private signing material or new personal files are published. Android application sources retain the already merged PR #2 and PR #3 changes; this synchronization does not rebuild or promote an Android APK.
