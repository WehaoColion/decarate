# 文档页合并头部验收

## 变更范围

以 main 的 c8dac75acb0d5f8f8f2a487c5d9df22c16a614ca 为基线，保留已经合并的风控修改。本次只调整 Android 文档列表头部，不修改账目、同步、模型配置、数据格式、签名或正式发布清单。

把 FlowusWorkspaceHeader 和 NoteSummaryCard 合为同一个 FlowusPanel。删除重复的静态空间路径、副标题、宣传语及页面总数。标题与页数放在第一行，三个原有操作集中在自适应换行区域。回收站、文档库的数量直接显示在对应入口上；置顶、任务块、图片数量仍保留。最近一页的时间与预览保留为最多一行，继续使用原有 previewBody()，不直接读取加密正文。

无筛选时显示“23 页”；筛选得到 5 页时显示“显示 5 / 共 23 页”。回收站使用回收站的当前结果和总量，不混用正常文档的总数。零结果不意味着原文档已经丢失。标题、操作区和统计区使用 FlowRow，不设置头部固定高度；三个原有操作的点击区域设置至少 48 dp 高。

变更仍由 Rust 生成 Kotlin。新模块 android_knowledge_header.rs 在已有 android_note_list_performance::render 的末尾执行，处于界面本地化之前，不手改生成目录。原有缓存统计、FlowusPanel、FlowusIconAction 的主题和交互实现不变。模板锚点缺失、重复或旧汇总卡被上游修改时停止生成，避免默默删掉新功能。

## 已做的检查

重新构造的原 android_note_list_performance.rs 与 GitHub blob d1d43656aeeeaeb057cf58ee01d1b4174bc75d4d 字节哈希一致。接入只增加模块声明，并把返回值交给新转换。

从本次 Rust 常量直接抽取新的 Kotlin 组件，在独立 Kotlin/JVM 运行器中执行了 9 个契约检查：普通列表、筛选计数、零匹配、空文档库、回收站计数与返回、三个回调及最小高度配置、长预览行数配置、预览访问器调用、较大计数与 FlowRow 注册。9 项通过。

上述运行器使用轻量替身记录 Compose 组件调用，不包含真正的 Android/Compose 测量、绘制、点击命中或生命周期，也不能证明像素高度、实际字体效果及设备兼容性。加密检查只确认继续调用旧访问器，不替代项目原有密码和隐私测试。

源码附带 7 个 Rust 回归测试，读取仓库真正的 NoteStudioSheet 模板，检查合并、缓存统计接入、计数范围、操作与预览边界、共享组件和后续便签/AI 代码不变、模板漂移拒绝及其他文件不变。本环境无 Rust/Android 工具链，这 7 个测试、rustfmt、完整生成链和 Gradle 编译尚未执行。

## 本地 Codex 验收和 APK 交付

保留本地所有未提交修改，在独立工作区接入本 PR，不回退已经验收的风控修改。先运行格式检查与对应的源码生成器回归测试，再按项目原有流程完整生成 Kotlin、运行已有领域/JVM 测试、lint 和正式签名 APK 构建。格式检查如有差异，应先格式化本次 Rust 修改并提交，不改动无关模块。

相关测试过滤命令：

```powershell
cargo test --manifest-path native/gridtimer_native/Cargo.toml --bin gridtimer_sourcegen android_knowledge_header::tests
```

实际生成的 NoteStudioSheet.kt 必须只有一张 knowledge_compact_header 卡，不再含 FlowusWorkspaceHeader；必须包含 knowledge_header_create、knowledge_header_trash 和 knowledge_header_folders。不能只检查 Rust 中存在这些文字，也不能把旧 APK 作为新构建结果。

在 320、360、412 dp 宽度以及 1.0、1.3、2.0 字体缩放下检查：标题、数量和按钮不重叠，窄屏可换行，卡片不截断，最近预览只占一行。记录同一数据集、同一窗口与字体缩放下修改前后的截图和头部实测高度，确认节省空间；未测量之前不宣称具体节省百分比。

用非敏感资料核对新建仍进入空白页面、回收站可进入且可返回、文档库入口仍打开管理界面、删除与恢复后的计数刷新正确、搜索和文档库筛选显示当前结果与总量、切换账户不串用统计。空列表、零计数、大计数、长标题、锁定笔记都应检查。知识问答、页面模板、筛选视图、正文编辑、便签以及刚验收的风控入口必须保持原状。

沿用原应用包名和正式签名，按 AGENTS.md 对当前实际版本递增，不能生成新证书要求用户卸载。将本次成功构建的 APK 放入 Windows 实际桌面的 TenRate_APK 文件夹，使用英文文件名，提供完整路径、版本、大小、SHA-256 和对应提交，并在资源管理器中选中。没有连接手机时可以交付候选 APK，但必须明确真机布局和操作尚未验收。

交付后按 AGENTS.md 核对本项目同步服务和自启动健康状态，保留原凭据、VPN 和其他项目隧道。由本地 Codex 在验收后合并 PR，本次提交不直接合并或发布。
