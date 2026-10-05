v0.0.1 - 增加构建与验证入口索引，历史一次性补丁归入本机档案。

# 工具入口

从项目根目录运行工具，正式发布继续使用现有入口和正式签名。

| 用途 | 入口 |
| --- | --- |
| 安卓环境检查、测试与签名正式构建 | [android.ps1](android.ps1) |
| 安卓发布前检查、交付与交付复验 | [publish_android_note.ps1](publish_android_note.ps1) |
| Windows 环境检查、测试、发布与校验 | [windows.ps1](windows.ps1) |
| 稳定桌面入口与快捷方式维护 | [windows_desktop_entry.ps1](windows_desktop_entry.ps1) |
| Windows 发布前领域测试 | [windows_preflight_tests.ps1](windows_preflight_tests.ps1) |

## 其他工具

`verify_*.ps1` 是专项验证与变异验证，`measure_*.ps1` 是性能测量，`prepare_*`、`record_*`、`summarize_*` 是测试资料准备与结果整理。各工具参数沿用文件中的声明。

历史一次性补丁保留在 `local_archive/one_time_scripts/`，供追溯旧修改，不属于日常构建或发布入口。
