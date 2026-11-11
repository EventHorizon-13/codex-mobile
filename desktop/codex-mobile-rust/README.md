# Codex Mobile · Windows

Rust + Slint 原生桌面客户端，不使用浏览器控件或 .NET 运行时。下载 `CodexMobile-Windows-x64.zip`、解压后运行 `CodexMobile.exe`。

首次连接填写服务器地址与访问口令。启用“保存此设备登录”时，服务器签发的设备凭证保存在 Windows Credential Manager；普通配置仅保存在 `%LOCALAPPDATA%/CodexMobile.Native/settings.json`，不含口令。可在界面内忘记设备凭证。

外观默认跟随 Windows 应用主题，也可在“设置与工具”中固定深色或浅色。标题栏由 Slint 绘制。界面使用按需渲染和可见行列表；网络运行在独立单线程事件循环中，流式文本按批次更新。

本客户端使用 Slint，界面页脚保留 “Powered by Slint” 署名。
