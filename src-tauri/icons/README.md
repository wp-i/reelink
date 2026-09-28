# Reelink icon

The artwork source is `public/reelink-logo.svg`: white scanning corners and two connected nodes on a solid dark rounded square. No letters, gradients, blur or fine decorative details are used. Each Windows size has its own optical master with pixel-aligned filled edges; the smallest marks are simplified to preserve separation. This is original Reelink artwork.

From the repository root, after `npm ci`, run:

```powershell
node scripts/generate-icons.mjs
```

The script uses the Tauri CLI from the locked npm dependencies to rasterize the
SVG directly at each target size. It writes PNGs and one Windows ICO containing
16, 20, 24, 32, 40, 48 and 64 px uncompressed 32-bit DIB frames with AND masks,
plus 128 and 256 px RGBA PNG frames. No image
editor, Python dependency or network request is required. Its temporary output
is a unique `reelink-icons-*` directory beneath the system temporary directory;
only that directory is removed when the script completes.

On Windows, Tauri reads the ICO for both the embedded executable resource and
the default window icon. The desktop shortcut explicitly uses the standalone
versioned ICO shipped through `bundle.resources`; update that resource destination
when bumping the package version. The Start menu inherits the executable icon.
The install hook refreshes only the desktop shortcut and preserves its icon when
Tauri's finish-page callback runs. It does not clear the global icon cache.

Verify installation from Windows Explorer and launch the desktop shortcut.
MSIX-hosted development tools can redirect LocalAppData into their own LocalCache;
seeing a new version inside that context is not evidence that the desktop target
was updated. Check the actual running process path and the icon shown by Explorer.
