# Codex Mobile Windows 原生客户端

这是独立的 WPF/.NET 8 Windows 桌面客户端。会话、输入框、消息列表和审批控件均为 Windows 原生控件，不使用 WebView 或网页 UI。它连接现有 Codex Mobile 网关，不在 Windows 本机启动 Codex CLI，也不改动服务器部署。

## 使用

从 GitHub Actions 的 **Windows native client** 构建记录下载 `CodexMobile-Windows-x64`，解压并运行 `CodexMobile.Windows.exe`。程序自带 .NET 运行时，面向 Windows 10/11 x64。

默认服务器为 `https://codex-go.arcsiny.com/`。输入现有 Codex Mobile 的访问口令后点击“连接”。首次接入仍按网关的设备批准规则处理。程序只在当前进程内保存访问口令派生的会话令牌；稳定设备 ID 存在 `%LOCALAPPDATA%\CodexMobile.Windows\device-id.txt`。如删除这个文件，网关会把下次连接视为新设备。

支持会话列表、选择与新建、文本历史、文本发送与流式回复、停止当前任务，以及操作审批。Enter 发送，Shift+Enter 换行。当前版本只显示历史中的部分工具摘要，不支持图片附件、文件浏览、离线队列或完整 Markdown 渲染；这些功能仍可通过现有移动网页使用。

远程连接只允许 HTTPS；`http://localhost`/`http://127.0.0.1` 用于本机开发。客户端使用网关现有的 `/auth/session` 和 Socket.IO 4 协议，审批仍由网关与 Codex app-server 执行。

## 构建

Windows 上安装 .NET 8 SDK 后：

```powershell
dotnet publish desktop/CodexMobile.Windows/CodexMobile.Windows.csproj --configuration Release --runtime win-x64 --self-contained true -p:PublishSingleFile=true -p:IncludeNativeLibrariesForSelfExtract=true
```

GitHub Actions 工作流位于 `.github/workflows/windows-native.yml`，在功能分支和 `dev` 上修改客户端时自动编译，并上传 ZIP 产物。
