# Codex Mobile WebView2

这是一个很小的 Windows 桌面外壳，不重新实现界面。它使用 Edge WebView2 直接打开现有 Codex Mobile Web UI，因此布局、主题和功能与浏览器版本一致。

## 运行

直接启动 `CodexWebView2.exe`。默认打开：

```text
https://codex-go.arcsiny.com/
```

也可以传入另一个 HTTP(S) 地址：

```powershell
.\CodexWebView2.exe http://127.0.0.1:3232/
```

登录状态、Cookie 和站点数据保存在：

```text
%LOCALAPPDATA%\CodexMobileWebView2\Profile
```

删除该目录即可清除这个外壳的登录状态。

窗口最小尺寸为 360x480，可以缩到常见手机宽度；Windows 标题栏和任务栏会显示 Codex 图标。

窗口宽高、最大化状态和文字缩放会自动保存到：

```text
%LOCALAPPDATA%\CodexMobileWebView2\window.json
```

文字大小快捷键：

- `Ctrl +`：放大 10%
- `Ctrl -`：缩小 10%
- `Ctrl 0`：恢复 100%
- `Ctrl + 鼠标滚轮`：连续缩放

可以直接从资源管理器或其他应用把文件拖进窗口。外壳会把文件交给网页现有的附件栏，图片预览和上传继续由 Web UI 处理。单次拖入的文件总大小上限为 24MB，避免桥接数据占用过多内存。

侧边栏会话条目支持右键：在具体会话行上右键，会立即打开与长按相同的会话菜单（重命名、压缩上下文、回退、未读、归档、删除）。其他区域保留 WebView2 默认右键菜单。

## 依赖

- Windows 10 或 Windows 11。
- Microsoft Edge WebView2 Runtime。安装新版 Edge 的系统通常已经自带；缺失时从微软官方页面安装 Evergreen Runtime。

## 构建

```powershell
cargo build --release --manifest-path desktop/codex-mobile-webview2/Cargo.toml
```

产物为 `desktop/codex-mobile-webview2/target/release/codex-mobile-webview2.exe`。

