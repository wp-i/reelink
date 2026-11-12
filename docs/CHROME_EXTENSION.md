# Chrome 扩展

Reelink Chrome 扩展是一个独立的 Manifest V3 扩展，可在桌面应用未运行时从当前网页截图识别二维码。它最低要求 Chrome 116。扩展只处理用户主动触发时当前标签页的可见区域，不会截取整个桌面，也不访问其他标签页。

## 构建与加载

使用交付 ZIP 时，先解压 `Reelink-Chrome-0.2.2.zip`，再在 `chrome://extensions` 启用「开发者模式」，点击「加载已解压的扩展程序」，选择解压得到的 `dist-extension` 文件夹。无需安装桌面版或开发依赖。更新已加载的扩展后，需要在扩展管理页点击「重新加载」。

从源码构建：

```powershell
npm ci
npm run build:extension
```

在 Chrome 打开 `chrome://extensions`，启用「开发者模式」，点击「加载已解压的扩展程序」，选择仓库根目录的 `dist-extension`。不要选择 `extension`，那里是源码目录。打包可分发 ZIP：

```powershell
npm run package:extension
```

当前脚本生成 `artifacts/Reelink-Chrome-0.2.2.zip`，文件名自动读取包版本。

## 使用

扩展快捷键默认为 Alt+Q，可在 `chrome://extensions/shortcuts` 修改。桌面应用默认也使用 Alt+Q；若同时使用两者，请为其中一方设置不同快捷键。也可点击扩展工具栏图标开始截图。

浏览器会冻结当前可见页面并显示框选层。按 Esc 取消，选择区域后本机使用打包的 jsQR 解码。只识别到一个二维码时，扩展自动复制该文本；识别到多个时，点击对应结果旁的「复制」按钮。识别后仍可重新框选或再次复制。关闭选区页、按 Esc、源页面导航或会话到期时才会清理截图；会话最长保留 120 秒。浏览器内置页、Chrome 商店等不允许注入选区的页面会打开扩展自带截图页完成框选。要处理 `file://` 页面，需在扩展详情页启用「允许访问文件网址」。

## 权限与数据

扩展只声明 `activeTab`、`scripting`、`offscreen`、`clipboardWrite` 和 `storage` 权限，不声明 `host_permissions`。`activeTab` 在用户主动调用时临时授权当前活动标签页；`scripting` 用于放置选区界面；选区界面和 offscreen 文档在扩展会话内存中保存冻结截图，以供预览、裁剪、解码和复制；`storage.session` 保存临时会话元数据及解码文本。截图和预览不写入磁盘。扩展没有远程连接权限，扩展页 CSP 禁止网络连接；它不接入桌面版 Tauri 命令，也不包含重命名或收藏功能。

桌面版与扩展的复制行为不同：桌面版显示识别结果，由用户点击复制；扩展单码自动复制，多码需用户选择后复制。

Chrome API 约束参见官方文档：[activeTab](https://developer.chrome.com/docs/extensions/develop/concepts/activeTab)、[commands](https://developer.chrome.com/docs/extensions/reference/api/commands)、[offscreen](https://developer.chrome.com/docs/extensions/reference/api/offscreen) 和 [storage](https://developer.chrome.com/docs/extensions/reference/api/storage)。

该说明记录当前实现范围，不代表旧版 Chrome 或所有网站均已测试通过。
