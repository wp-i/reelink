# Internal implementation contract

Reelink: Windows-only Tauri 2 + React/TypeScript. All UI Chinese. Local first.

Frontend invokes these Tauri commands (camelCase argument keys):

- `decode_clipboard() -> QrResult`
- `decode_image({ data: number[] }) -> QrResult` (pasted/browser-selected image bytes)
- `capture_qr() -> QrResult` (Windows screen clipping then local clipboard decode)
- `copy_text({ text: string }) -> void`
- `get_preferences() -> Preferences`
- `update_preferences({ shortcut: string }) -> Preferences`
- `preview_renames({ paths: string[] }) -> RenameItem[]`
- `apply_renames({ items: RenameEdit[] }) -> RenameOutcome`
- `undo_rename() -> RenameOutcome`
- `rename_history() -> HistorySummary | null`
- `search_tmdb({ query: string, kind: 'movie'|'tv' }) -> TmdbMatch[]` (public TMDb search page, no API credential)
- `list_resource_sites() -> ResourceSite[]`
- `save_resource_site({ id: string|null, name: string, url: string }) -> ResourceSite[]`
- `delete_resource_site({ id: string }) -> ResourceSite[]`
- `open_resource_site({ id: string }) -> void`

Types (all serialized field names camelCase):

```ts
interface QrResult { texts: string[]; imageDataUrl: string }
interface Preferences { shortcut: string; shortcutAvailable: boolean }
interface ResourceSite { id: string; name: string; url: string }
interface RenameItem { id: string; sourcePath: string; originalName: string; suggestedName: string; title: string; year: string | null; extension: string; kind: 'movie'|'tv'; confidence: 'high'|'review'; notes: string[]; isDirectory: boolean; groupId: string | null; episode: string | null }
interface RenameEdit { id: string; sourcePath: string; targetName: string }
interface RenameOutcome { count: number; message: string }
interface HistorySummary { count: number; createdAt: string }
interface TmdbMatch { id: number; title: string; originalTitle: string; year: string | null; overview: string; kind: 'movie'|'tv' }
```

`id` is opaque backend authorization token associated with previewed path; backend never accepts unpreviewed arbitrary operations. Backend preserves all video extensions and the directory hierarchy. Selected directories are scanned for video files; nested name changes are previewed explicitly, children execute before parent folders, and undo proceeds in reverse order. Directory identity uses Windows volume/file IDs, with compatibility for legacy journals. Failed parent restoration blocks descendant operations. Preview registers sources. Execution enforces non-collision, no overwrite, valid Windows names, source consistency, and persistent undo journal.

Rust renamer module: expose serializable types above and `pub struct RenameEngine`; `RenameEngine::new(data_dir: PathBuf) -> Self`; methods `preview(&mut self, paths: Vec<String>) -> Result<Vec<RenameItem>, String>`, `apply(&mut self, items: Vec<RenameEdit>) -> Result<RenameOutcome,String>`, `undo(&mut self) -> Result<RenameOutcome,String>`, `history(&self) -> Result<Option<HistorySummary>,String>`. Parent wraps Mutex engine in Tauri state and commands. Module can use serde, serde_json, regex, uuid (v4), chrono, tempfile (dev). Keep journals recoverable, partial failure honest.

Rust qr module: async Tauri `#[tauri::command] pub async fn decode_clipboard()`, `decode_image(data: Vec<u8>)`, `capture_qr(window: tauri::WebviewWindow)` and sync `copy_text(text: String)`. Helpers should unit test decoding. Dependencies parent includes: image 0.25 (png,jpeg,gif,bmp,webp only), rqrr 0.10, arboard 3, base64 0.22, tauri 2, windows-sys 0.61 (Win32_UI_Shell, Win32_UI_WindowsAndMessaging, Win32_Foundation). Commands return Result<QrResult,String>. Coordinate dependency changes with parent.

Frontend file ownership: src/App.tsx, src/styles.css, src/types.ts. Parent owns src/main.tsx, config and backend other than qr.rs/rename.rs. Use lucide-react, @tauri-apps/api, @tauri-apps/plugin-dialog. File picker open({multiple:true,directory:false,filters:[...]}) for movie; directory:true for folders. Tauri native onDragDropEvent for path drops. Browser preview allowed but show desktop-only limitation honestly; can provide clearly labeled examples. Do not invoke real filesystem from browser fallback. Accept image drops via DOM, native path drops may need read through QR command (coordinate). QR input hidden type=file bytes.

Style revised in 0.1.8: a compact Apple-inspired pearl surface, floating pill selection, rounded controls, subtle elevation and visible keyboard focus, native Windows title bar, and an internally scrolling original/new-name table. QR and rename share the default 520×240 window with minimum 480×220. Mode/content changes never resize the window; workspace content scrolls inside the user-sized window. Both modes use the same fixed-grid empty-state component, identical action widths/order and fixed footer height. Stable scrollbar gutters prevent horizontal shifts; result actions and summary/card origins align between modes. Resource shortcuts use neutral solid-gray surfaces with no positional hover transform. Only QR and rename are tabs. Resource links are two compact toolbar shortcuts, with URL-only editing in an anchored viewport-bounded popover; additional existing sites remain reachable in a compact menu. No independent resource page, automatic copy, settings page, or About UI. The screenshot shortcut editor is inline at the bottom. TMDb attribution is inline in the search panel.

Selection has no movie/TV switch. Movie containers emit individual video rows without renaming the container. TV folders and episode files share a preview-scoped groupId. Selecting a TMDb match or editing the TV folder name updates sibling episode suggestions. A season folder chosen alone never authorizes renaming its ancestor. Unknown episode names remain unchanged. Subtitles and directory structure remain untouched. Directory scan limit: 8 levels, 5000 videos, links/reparse points rejected. A global shortcut (default Alt+Q) starts screen capture; there is no capture button. Saved preferences contain only the shortcut, and legacy autoCopy keys are ignored.

Resource sites: list starts empty; names (1–80 characters), canonical HTTP(S) URLs and UUIDs persist in resource-sites.json using atomic replacement. Corrupt stores are reported and never silently replaced. Reject credentials, controls, malformed schemes, loopback/unspecified IPs (including mapped IPv6), localhost and *.localhost. Each site uses a stable resource-<UUID> window with its own persistent WebView2 data directory. Reopening focuses the existing page; editing closes its old window after persistence, so the next click opens the changed address. Safe popup links navigate the existing window; all new windows and downloads are denied. Navigation repeats URL validation. Deleting a bookmark closes its window but retains its browser profile.

Security boundary: capabilities/default.json grants only the main window local permissions, with no remote capability. Tauri 2.11.6 webview/mod.rs explicitly checks ACL for every remote request, including custom app commands without an app ACL manifest. The centralized invoke handler additionally rejects every non-main webview before dispatching any application command. This applies to all existing file/clipboard commands and future commands. Resource windows cannot navigate to app/custom protocol origins; on-page-load focus is restricted to main. Main-window operations remain available while browsing. These boundaries require desktop WebView2 verification in addition to URL and persistence unit tests.
