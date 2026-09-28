use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::PathBuf, sync::Mutex};
use tauri::{Manager, State, Url, WebviewUrl, WebviewWindowBuilder};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ResourceSite {
    pub id: String,
    pub name: String,
    pub url: String,
}

pub struct ResourceStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl ResourceStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    fn load(&self) -> Result<Vec<ResourceSite>, String> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(format!("无法读取资源网站：{e}")),
        };
        let sites: Vec<ResourceSite> = serde_json::from_slice(&bytes)
            .map_err(|_| "资源网站文件损坏，请保留文件并恢复备份。".to_string())?;
        let mut ids = std::collections::HashSet::new();
        for site in &sites {
            if Uuid::parse_str(&site.id).is_err() || !ids.insert(&site.id) {
                return Err("资源网站文件包含无效或重复标识。".into());
            }
            validate_name(&site.name)?;
            validate_url(&site.url)?;
        }
        Ok(sites)
    }

    fn save(&self, sites: &[ResourceSite]) -> Result<(), String> {
        let parent = self.path.parent().ok_or("资源网站路径无效")?;
        fs::create_dir_all(parent).map_err(|e| format!("无法创建资源网站目录：{e}"))?;
        let temporary = parent.join(format!("resource-sites-{}.tmp", Uuid::new_v4()));
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|e| format!("无法保存资源网站：{e}"))?;
            serde_json::to_writer_pretty(&mut file, sites).map_err(|e| e.to_string())?;
            file.write_all(b"\n")
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
            drop(file);
            replace_file(&temporary, &self.path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
}

fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err("网站名称需要 1–80 个字符，不能含控制字符。".into());
    }
    Ok(name.into())
}

fn validate_url(input: &str) -> Result<Url, String> {
    let input = input.trim();
    if input.len() > 4096
        || input.chars().any(|c| c.is_control() || c.is_whitespace())
        || input.contains('\\')
    {
        return Err("请输入有效的网站地址，不要包含空格或控制字符。".into());
    }
    let url = Url::parse(input).map_err(|_| "请输入完整的 http:// 或 https:// 网站地址。")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !input
            .to_ascii_lowercase()
            .starts_with(&format!("{}://", url.scheme()))
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("仅支持不含用户名和密码的 HTTP(S) 网站地址。".into());
    }
    // WebView2 represents Tauri custom protocols using *.localhost. Keep these,
    // and the dev server/loopback, out of the untrusted browsing surface.
    let host = url
        .host_str()
        .unwrap()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let ip = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<std::net::IpAddr>()
        .ok();
    let local_ip = ip.is_some_and(|ip| match ip {
        std::net::IpAddr::V4(ip) => ip.is_loopback() || ip.is_unspecified(),
        std::net::IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip
                    .to_ipv4_mapped()
                    .is_some_and(|ip| ip.is_loopback() || ip.is_unspecified())
        }
    });
    if host == "localhost" || host.ends_with(".localhost") || local_ip {
        return Err("资源网站不能使用应用内部或本机回环地址。".into());
    }
    Ok(url)
}

pub fn is_main_window(label: &str) -> bool {
    label == "main"
}

#[tauri::command]
pub fn list_resource_sites(state: State<ResourceStore>) -> Result<Vec<ResourceSite>, String> {
    let _guard = state.lock.lock().map_err(|_| "资源网站状态异常")?;
    state.load()
}

#[tauri::command]
pub fn save_resource_site(
    app: tauri::AppHandle,
    state: State<ResourceStore>,
    id: Option<String>,
    name: String,
    url: String,
) -> Result<Vec<ResourceSite>, String> {
    let name = validate_name(&name)?;
    let url = validate_url(&url)?.to_string();
    let _guard = state.lock.lock().map_err(|_| "资源网站状态异常")?;
    let mut sites = state.load()?;
    let mut changed_id = None;
    if let Some(id) = id {
        let site = sites
            .iter_mut()
            .find(|site| site.id == id)
            .ok_or("网站已不存在，请刷新列表。")?;
        if site.url != url || site.name != name {
            changed_id = Some(id);
        }
        site.name = name;
        site.url = url;
    } else {
        sites.push(ResourceSite {
            id: Uuid::new_v4().to_string(),
            name,
            url,
        });
    }
    state.save(&sites)?;
    if let Some(id) = changed_id {
        if let Some(window) = app.get_webview_window(&format!("resource-{id}")) {
            let _ = window.close();
        }
    }
    Ok(sites)
}

#[tauri::command]
pub fn delete_resource_site(
    app: tauri::AppHandle,
    state: State<ResourceStore>,
    id: String,
) -> Result<Vec<ResourceSite>, String> {
    let _guard = state.lock.lock().map_err(|_| "资源网站状态异常")?;
    let mut sites = state.load()?;
    let index = sites
        .iter()
        .position(|site| site.id == id)
        .ok_or("网站已不存在，请刷新列表。")?;
    sites.remove(index);
    state.save(&sites)?;
    if let Some(window) = app.get_webview_window(&format!("resource-{id}")) {
        let _ = window.close();
    }
    Ok(sites)
}

#[tauri::command]
pub async fn open_resource_site(
    app: tauri::AppHandle,
    state: State<'_, ResourceStore>,
    id: String,
) -> Result<(), String> {
    let site = {
        let _guard = state.lock.lock().map_err(|_| "资源网站状态异常")?;
        state
            .load()?
            .into_iter()
            .find(|site| site.id == id)
            .ok_or("网站已不存在，请刷新列表。")?
    };
    let url = validate_url(&site.url)?;
    let label = format!("resource-{}", site.id);
    if let Some(window) = app.get_webview_window(&label) {
        window
            .set_title(&format!("{} · 资源网站", site.name))
            .map_err(|e| e.to_string())?;
        window.unminimize().map_err(|e| e.to_string())?;
        window.show().map_err(|e| e.to_string())?;
        return window.set_focus().map_err(|e| e.to_string());
    }
    let browsing_data = app
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?
        .join("resource-webviews")
        .join(&site.id);
    fs::create_dir_all(&browsing_data).map_err(|e| format!("无法创建网站数据目录：{e}"))?;
    let navigation_app = app.clone();
    let navigation_label = label.clone();
    WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(url))
        .title(format!("{} · 资源网站", site.name))
        .inner_size(1100.0, 760.0)
        .min_inner_size(640.0, 480.0)
        .data_directory(browsing_data)
        .on_navigation(|url| validate_url(url.as_str()).is_ok())
        .on_new_window(move |url, _| {
            if validate_url(url.as_str()).is_ok() {
                if let Some(window) = navigation_app.get_webview_window(&navigation_label) {
                    let _ = window.navigate(url);
                }
            }
            tauri::webview::NewWindowResponse::Deny
        })
        .on_download(|_, _| false)
        .build()
        .map_err(|e| format!("无法打开网站窗口：{e}"))?;
    Ok(())
}

#[cfg(windows)]
fn replace_file(source: &std::path::Path, target: &std::path::Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(format!(
            "无法保存资源网站：{}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &std::path::Path, target: &std::path::Path) -> Result<(), String> {
    fs::rename(source, target).map_err(|e| format!("无法保存资源网站：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_boundary_rejects_privileged_and_credential_urls() {
        for url in [
            "javascript:alert(1)",
            "file:///C:/secret",
            "tauri://localhost",
            "http://tauri.localhost",
            "https://ipc.localhost/x",
            "http://127.0.0.1:1420",
            "http://[::1]",
            "http://[::]",
            "http://[::ffff:7f00:1]",
            "http://[::ffff:127.0.0.1]",
            "http://2130706433",
            "https://name:secret@example.com",
            "https://example.com/\nfoo",
            "https:example.com",
            "https://example.com\\foo",
            "https://localhost.",
        ] {
            assert!(validate_url(url).is_err(), "accepted {url}");
        }
        assert_eq!(
            validate_url(" https://example.com/path?q=1#video ")
                .unwrap()
                .as_str(),
            "https://example.com/path?q=1#video"
        );
    }

    #[test]
    fn empty_store_round_trip_update_delete_and_corruption() {
        let root = tempfile::tempdir().unwrap();
        let store = ResourceStore::new(root.path().join("resource-sites.json"));
        assert!(store.load().unwrap().is_empty());
        let mut sites = vec![ResourceSite {
            id: Uuid::new_v4().to_string(),
            name: "我的网站".into(),
            url: "https://example.com/".into(),
        }];
        store.save(&sites).unwrap();
        assert_eq!(store.load().unwrap(), sites);
        sites[0].name = "新名称".into();
        store.save(&sites).unwrap();
        assert_eq!(store.load().unwrap(), sites);
        store.save(&[]).unwrap();
        assert!(store.load().unwrap().is_empty());
        fs::write(&store.path, b"broken").unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read(&store.path).unwrap(), b"broken");
    }

    #[test]
    fn only_main_window_can_dispatch_app_commands() {
        assert!(is_main_window("main"));
        for label in ["resource-123", "main-child", "", "MAIN"] {
            assert!(!is_main_window(label));
        }
        assert!(validate_name(" ").is_err());
        assert!(validate_name("a\nb").is_err());
    }

    #[test]
    fn invalid_persisted_entries_are_never_accepted() {
        let root = tempfile::tempdir().unwrap();
        let store = ResourceStore::new(root.path().join("resource-sites.json"));
        let site = ResourceSite {
            id: Uuid::new_v4().to_string(),
            name: "站点".into(),
            url: "https://example.com/".into(),
        };
        store.save(&[site.clone(), site.clone()]).unwrap();
        assert!(store.load().is_err());
        let mut invalid = site;
        invalid.url = "file:///C:/secret".into();
        store.save(&[invalid]).unwrap();
        assert!(store.load().is_err());
    }

    #[test]
    fn failed_replacement_preserves_existing_destination_and_cleans_temporary_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("resource-sites.json");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("keep"), "existing data").unwrap();
        let store = ResourceStore::new(path.clone());
        assert!(store.save(&[]).is_err());
        assert_eq!(
            fs::read_to_string(path.join("keep")).unwrap(),
            "existing data"
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
