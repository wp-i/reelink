# Reelink 开发交接

适用源码版本为 0.2.2，交接日期为 2026-11-13。仓库包含 Windows 桌面应用和独立的 Chrome 扩展。桌面端提供二维码识别、影视文件重命名和用户自定义网站入口；扩展负责从活动标签页的可见区域框选识别二维码。

## 接手顺序

1. 运行 `git status --short --branch`，确认分支和已有工作区改动。先读 [AGENTS.md](AGENTS.md) 和 [贡献指南](CONTRIBUTING.md)。
2. 按修改范围阅读 [接口与行为约定](docs/CONTRACT.md)、[隐私说明](docs/PRIVACY.md) 或 [Chrome 扩展说明](docs/CHROME_EXTENSION.md)。[验证记录](docs/VALIDATION.md) 记录各阶段实际测到的行为。
3. 用下面的命令复现本轮涉及的入口。改动 Windows 截图、资源窗口权限或安装钩子时，在真实 Windows 桌面和 WebView2 环境复测。

## 开发与构建

桌面端要求 Windows 10/11 x64、Node.js 22.12+、Rust 1.88+、Visual Studio C++ Build Tools 和 Microsoft Edge WebView2 Runtime。扩展使用相同的 npm 依赖，运行环境为 Chrome 116+。

```powershell
npm ci
npm run desktop
```

`npm run desktop` 由 Tauri 启动 Vite（`127.0.0.1:1420`）和桌面窗口。`npm run dev` 可预览 React 界面；截图、WebView2 权限、文件改名和撤销请从桌面入口验证。

```powershell
npm run build
npm run format:check
cargo test --locked --manifest-path src-tauri/Cargo.toml
npm run build:extension
npm run format:extension
npm run test:extension
```

扩展通过 `chrome://extensions` 的开发者模式加载 `dist-extension/`。`npm run package:extension` 生成 `artifacts/Reelink-Chrome-0.2.2.zip`；`npm run package` 生成桌面程序和 `src-tauri/target/release/bundle/nsis/` 下的安装包。Windows GitHub Actions 也会执行检查并上传未签名的构建产物，工作流位于 [windows.yml](.github/workflows/windows.yml)。

## 修改入口

| 范围 | 起点 |
| --- | --- |
| 主界面与 IPC | [src/main.tsx](src/main.tsx) 选择主界面或本地资源工具栏，[src/App.tsx](src/App.tsx) 管理主窗口交互，[src-tauri/src/lib.rs](src-tauri/src/lib.rs) 注册命令与校验调用来源。 |
| 桌面二维码与截图 | [qr.rs](src-tauri/src/qr.rs) 处理剪贴板、图片和解码，[screen_capture.rs](src-tauri/src/screen_capture.rs) 管理 Win32 截图选区，[capture_launch.rs](src-tauri/src/capture_launch.rs) 协调截图请求和前端就绪。 |
| Alt+Q 与安装 | [preferences.rs](src-tauri/src/preferences.rs) 保存组合键，[shell_hotkey.rs](src-tauri/src/shell_hotkey.rs) 管理 Windows 快捷方式，[shell_activation.rs](src-tauri/src/shell_activation.rs) 接收运行中窗口消息，[installer-hooks.nsh](src-tauri/installer-hooks.nsh) 调用快捷方式安装和移除入口。 |
| 改名、撤销与作品查询 | [rename.rs](src-tauri/src/rename.rs) 负责文件预览、冲突校验、执行和恢复，[rename_nested_tests.rs](src-tauri/src/rename_nested_tests.rs) 覆盖嵌套目录，[tmdb.rs](src-tauri/src/tmdb.rs) 解析用户主动查询的公开搜索页。 |
| 网站窗口 | [ResourceSites.tsx](src/ResourceSites.tsx) 管理收藏，[ResourceBrowser.tsx](src/ResourceBrowser.tsx) 提供工具栏，[resources.rs](src-tauri/src/resources.rs) 管理站点和窗口，[browser.rs](src-tauri/src/resources/browser.rs) 与 [native.rs](src-tauri/src/resources/browser/native.rs) 同步导航状态并验证工具栏来源。 |
| Chrome MV3 扩展 | [manifest.json](extension/manifest.json) 声明入口和权限，[background.ts](extension/background.ts) 管理触发与会话，[selection.ts](extension/selection.ts) 管理选区，[offscreen.ts](extension/offscreen.ts) 处理截图与解码，[build-extension.mjs](scripts/build-extension.mjs) 构建扩展。 |

## 验收要点

- Windows 截图的链路为 `--capture` 或运行中快捷键 → 请求队列 → 前端就绪握手 → 原生选区 → 本地解码。当前实现使用内存中的桌面快照。最近的真实桌面记录仍待确认物理 Alt+Q、框选识别和复制。优先用临时二维码复测完全退出、运行中、取消/失焦/超时恢复以及非主显示器和不同缩放。安装版以用户实际开始菜单快捷方式和程序路径为准。
- 资源窗口分为可信本地工具栏和第三方网页。导航命令校验来源、窗口和站点会话，网页没有本地文件、剪贴板或改名命令权限。修改窗口逻辑后，从第三方测试页核对 IPC 拒绝，检查返回、前进、地址同步和最小窗口尺寸。主窗口默认 520×240，两个工具共用布局。
- 改名需要预览、冲突校验和持久化撤销。用临时文件验证目标冲突、中断恢复以及重启后撤销。桌面本地数据目录为 `%LOCALAPPDATA%\app.reelink.desktop`，包含偏好、收藏、撤销记录和独立站点的 WebView2 数据。撤销日志含文件路径。
- 扩展的 Alt+Q 可能与桌面快捷键冲突。测试普通网页及内置页的截图回退、Esc 或导航后的清理、单码自动复制和多码手动选择。扩展截图留在会话内存中；识别结果和会话元数据保存在 `storage.session`。
- 构建与用户数据各自在本地生成。保持 `node_modules/`、`dist/`、`dist-extension/`、`artifacts/`、Rust 编译目录和 `.env` 等文件在 Git 忽略范围内。

## 本次验证

- 在仅包含待提交源码的临时副本中运行 `npm ci`，确认 Vite 首页、React 入口和预构建依赖可访问。
- TypeScript/Vite 构建、前端与 Rust 格式检查、Chrome 扩展构建与格式检查均通过。扩展 26 项测试通过；Rust 64 项通过、3 项按默认设置跳过，跳过项涉及手动改名验收和在线 TMDb 查询。
- 物理 Alt+Q、原生框选与复制、安装/卸载和真实 Chrome 扩展交互仍需桌面实测。已有工作区如有旧 `artifacts/` 测试页面，Vite 可能报告其依赖扫描错误；本次干净副本启动未出现该错误。
