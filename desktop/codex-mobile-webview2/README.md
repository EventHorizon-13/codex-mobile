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

## 依赖

- Windows 10 或 Windows 11。
- Microsoft Edge WebView2 Runtime。安装新版 Edge 的系统通常已经自带；缺失时从微软官方页面安装 Evergreen Runtime。

## 构建

```powershell
cargo build --release --manifest-path desktop/codex-mobile-webview2/Cargo.toml
```

产物为 `desktop/codex-mobile-webview2/target/release/codex-mobile-webview2.exe`。

