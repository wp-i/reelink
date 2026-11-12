//! Installer-owned Windows Shell shortcut for capture while Reelink is closed.
//!
//! Ordinary runtime code never creates a link. A persisted hotkey is configuration, not
//! evidence that Explorer has registered it or that the combination is free.

use std::path::PathBuf;

/// Encode the Shell link HOTKEYF_* modifiers (not RegisterHotKey MOD_*).
pub fn hotkey_word(shortcut: &str) -> Result<u16, String> {
    let canonical = crate::preferences::normalize_shortcut(shortcut)?.0;
    let mut parts = canonical.split('+').collect::<Vec<_>>();
    let key = parts.pop().ok_or("快捷键缺少主键。")?;
    let modifiers = parts.iter().fold(0u16, |bits, modifier| {
        bits | match *modifier {
            "Shift" => 0x01,
            "Ctrl" => 0x02,
            "Alt" => 0x04,
            _ => 0,
        }
    });
    let key = if key.len() == 1 {
        key.as_bytes()[0] as u16
    } else {
        0x70 + key[1..].parse::<u16>().map_err(|_| "无效的功能键。")? - 1
    };
    Ok((modifiers << 8) | key)
}

#[cfg(windows)]
pub use platform::{configure_existing, inspect_existing, install_current, remove_current};

#[cfg(not(windows))]
pub fn inspect_existing() -> Result<bool, String> {
    Ok(false)
}

#[cfg(not(windows))]
pub fn install_current(shortcut: &str) -> Result<(), String> {
    hotkey_word(shortcut)?;
    Err("系统截图快捷方式仅支持 Windows。".into())
}

#[cfg(not(windows))]
pub fn remove_current() -> Result<bool, String> {
    Ok(false)
}

#[cfg(not(windows))]
pub fn configure_existing(shortcut: &str) -> Result<bool, String> {
    hotkey_word(shortcut)?;
    Ok(false)
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::ffi::OsString;
    use std::fs;
    use std::io::{Read, Seek, Write};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::path::{Component, Path, Prefix};
    use windows::core::{Interface, HSTRING, PCWSTR};
    use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IPersistFile,
        IPersistStream, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READ, STREAM_SEEK_CUR,
        STREAM_SEEK_SET,
    };
    use windows::Win32::UI::Shell::{
        FOLDERID_Programs, IShellLinkW, SHChangeNotify, SHCreateMemStream, SHGetKnownFolderPath,
        ShellLink, KF_FLAG_DONT_VERIFY, SHCNE_CREATE, SHCNE_DELETE, SHCNE_ID, SHCNE_UPDATEITEM,
        SHCNF_FLUSH, SHCNF_PATHW, SLGP_RAWPATH,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        GetDriveTypeW, MoveFileExW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_READ, FILE_SHARE_WRITE, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    struct ComApartment(bool);
    impl ComApartment {
        fn enter() -> Result<Self, String> {
            let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            if result == RPC_E_CHANGED_MODE {
                // The caller already initialized another apartment model.
                return Ok(Self(false));
            }
            result
                .ok()
                .map_err(|err| format!("无法初始化系统快捷方式：{err}"))?;
            Ok(Self(true))
        }
    }
    impl Drop for ComApartment {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    fn shortcut_path() -> Result<PathBuf, String> {
        // The default lookup verifies the directory, potentially accessing a
        // redirected network share before we can reject it.
        let raw = unsafe { SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_DONT_VERIFY, None) }
            .map_err(|err| format!("无法查找开始菜单：{err}"))?;
        let path = unsafe { OsString::from_wide(raw.as_wide()) };
        unsafe { CoTaskMemFree(Some(raw.0.cast())) };
        let path = PathBuf::from(path);
        validate_local_directory(&path)?;
        Ok(path.join("Reelink 截图.lnk"))
    }

    fn validate_local_directory(path: &Path) -> Result<(), String> {
        local_path_key(path).ok_or("截图快捷方式需要本地开始菜单目录。")?;
        let mut components = path.components();
        let mut ancestor = PathBuf::from(components.next().unwrap().as_os_str());
        ancestor.push(components.next().unwrap().as_os_str());
        let root = wide(&ancestor);
        // DRIVE_REMOVABLE=2, DRIVE_FIXED=3, DRIVE_CDROM=5, DRIVE_RAMDISK=6.
        // Reject remote, unknown and unavailable drives before filesystem access.
        if !matches!(unsafe { GetDriveTypeW(root.as_ptr()) }, 2 | 3 | 5 | 6) {
            return Err("截图快捷方式不能使用网络或不可用的开始菜单目录。".into());
        }
        // Walk from the root, inspecting each ancestor before traversing it.
        // Checking only the final .lnk does not catch a junction to a share.
        for component in components {
            ancestor.push(component.as_os_str());
            let metadata = fs::symlink_metadata(&ancestor)
                .map_err(|err| format!("无法检查开始菜单目录：{err}"))?;
            if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                return Err("截图快捷方式不能使用重解析或非目录的开始菜单路径。".into());
            }
        }
        Ok(())
    }

    fn load_link(path: &Path) -> Result<IShellLinkW, String> {
        let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
            .map_err(|err| format!("无法读取系统快捷方式：{err}"))?;
        let persist: IPersistFile = link.cast().map_err(|err| err.to_string())?;
        let path = wide(path);
        unsafe { persist.Load(PCWSTR(path.as_ptr()), STGM_READ) }
            .map_err(|err| format!("无法读取截图快捷方式：{err}"))?;
        Ok(link)
    }

    fn nul_slice(buffer: &[u16]) -> &[u16] {
        &buffer[..buffer
            .iter()
            .position(|&ch| ch == 0)
            .unwrap_or(buffer.len())]
    }

    // Inspect the spelling before any filesystem lookup. In particular, never
    // canonicalize a foreign UNC, device, relative, or redirected target.
    fn local_path_key(path: &Path) -> Option<String> {
        let mut components = path.components();
        if !matches!(components.next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_)))
            || !matches!(components.next(), Some(Component::RootDir))
            || !components.all(|part| matches!(part, Component::Normal(_)))
        {
            return None;
        }
        Some(path.to_str()?.replace('/', "\\").to_ascii_lowercase())
    }

    fn same_local_target(target: &Path, exe: &Path) -> bool {
        let Some(target) = local_path_key(target) else {
            return false;
        };
        // current_exe may use the extended-length spelling; links we create
        // use the ordinary local spelling so no device path is accepted from a link.
        let Some(exe) = exe.to_str() else {
            return false;
        };
        let exe = Path::new(exe.strip_prefix(r"\\?\").unwrap_or(exe));
        local_path_key(exe).is_some_and(|exe| exe == target)
    }

    fn local_exe_path(exe: &Path) -> Result<PathBuf, String> {
        let text = exe.to_str().ok_or("程序路径无法编码为系统快捷方式。")?;
        let path = PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(text));
        local_path_key(&path).ok_or("截图快捷方式需要本地绝对程序路径。")?;
        Ok(path)
    }

    fn owns_link(link: &IShellLinkW, exe: &Path) -> Result<bool, String> {
        let mut target = vec![0u16; 32768];
        let mut arguments = vec![0u16; 32768];
        unsafe {
            link.GetPath(&mut target, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)
                .map_err(|err| err.to_string())?;
            link.GetArguments(&mut arguments)
                .map_err(|err| err.to_string())?;
        }
        if nul_slice(&arguments) != "--capture".encode_utf16().collect::<Vec<_>>() {
            return Ok(false);
        }
        // Do not Resolve(): it may search for or change a missing target.
        let target = PathBuf::from(OsString::from_wide(nul_slice(&target)));
        if !same_local_target(&target, exe) {
            return Ok(false);
        }
        let Ok(target) = fs::canonicalize(target) else {
            return Ok(false);
        };
        let exe = fs::canonicalize(exe).map_err(|err| format!("无法确认当前程序路径：{err}"))?;
        Ok(target == exe)
    }

    fn owned_at(path: &Path, exe: &Path) -> Result<Option<(IShellLinkW, u16)>, String> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(format!("无法检查截图快捷方式：{err}")),
        };
        if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Ok(None);
        }
        let link = load_link(path)?;
        if !owns_link(&link, exe)? {
            return Ok(None);
        }
        let hotkey = unsafe { link.GetHotkey() }.map_err(|err| err.to_string())?;
        Ok(Some((link, hotkey)))
    }

    /// True means the owned link contains the requested hotkey; Explorer's
    /// eventual registration and conflicts cannot be established by GetHotkey.
    pub fn configure_existing(shortcut: &str) -> Result<bool, String> {
        let hotkey = hotkey_word(shortcut)?;
        let _com = ComApartment::enter()?;
        let path = shortcut_path()?;
        let exe = std::env::current_exe().map_err(|err| err.to_string())?;
        #[cfg(debug_assertions)]
        trace_shortcut("configure", &path, &exe);
        let configured = configure_at(&path, &exe, hotkey)?;
        if configured {
            notify(&path, SHCNE_UPDATEITEM);
        }
        Ok(configured)
    }

    /// Read only ownership check. A failed read must not be treated as a
    /// portable installation, because the Shell may still own the hotkey.
    pub fn inspect_existing() -> Result<bool, String> {
        let _com = ComApartment::enter()?;
        let path = shortcut_path()?;
        let exe = std::env::current_exe().map_err(|err| err.to_string())?;
        owned_at(&path, &exe).map(|link| link.is_some())
    }

    fn notify(path: &Path, event: SHCNE_ID) {
        let path = wide(path);
        unsafe {
            SHChangeNotify(
                event,
                SHCNF_PATHW | SHCNF_FLUSH,
                Some(path.as_ptr().cast()),
                None,
            );
        }
    }

    /// Called only by the explicit installer entry, after preferences load succeeds.
    pub fn install_current(shortcut: &str) -> Result<(), String> {
        let hotkey = hotkey_word(shortcut)?;
        let _com = ComApartment::enter()?;
        let path = shortcut_path()?;
        let exe = std::env::current_exe().map_err(|err| err.to_string())?;
        #[cfg(debug_assertions)]
        trace_shortcut("install", &path, &exe);
        let created = install_at(&path, &exe, hotkey)?;
        notify(
            &path,
            if created {
                SHCNE_CREATE
            } else {
                SHCNE_UPDATEITEM
            },
        );
        Ok(())
    }

    fn install_at(path: &Path, exe: &Path, hotkey: u16) -> Result<bool, String> {
        let exe = local_exe_path(exe)?;
        let (link, original) = match fs::symlink_metadata(path) {
            Ok(_) => {
                let Some((link, _)) = owned_at(path, &exe)? else {
                    return Err("同名截图快捷方式不属于当前 Reelink，已保留原文件。".into());
                };
                let original = fs::read(path).map_err(|err| err.to_string())?;
                (link, Some(original))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let link: IShellLinkW =
                    unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
                        .map_err(|err| err.to_string())?;
                let target = wide(&exe);
                unsafe {
                    link.SetPath(PCWSTR(target.as_ptr()))
                        .map_err(|err| err.to_string())?;
                    link.SetArguments(&HSTRING::from("--capture"))
                        .map_err(|err| err.to_string())?;
                }
                (link, None)
            }
            Err(err) => return Err(format!("无法检查截图快捷方式：{err}")),
        };
        let icon = exe
            .parent()
            .ok_or("程序路径缺少父目录。")?
            .join("icons")
            .join(format!("reelink-{}.ico", env!("CARGO_PKG_VERSION")));
        let icon = if icon.is_file() { &icon } else { &exe };
        let icon = wide(icon);
        unsafe { link.SetIconLocation(PCWSTR(icon.as_ptr()), 0) }.map_err(|err| err.to_string())?;
        let created = original.is_none();
        save_at(&link, path, &exe, hotkey, original.as_deref())?;
        Ok(created)
    }

    pub fn remove_current() -> Result<bool, String> {
        let _com = ComApartment::enter()?;
        let path = shortcut_path()?;
        let exe = std::env::current_exe().map_err(|err| err.to_string())?;
        #[cfg(debug_assertions)]
        trace_shortcut("remove", &path, &exe);
        let removed = remove_at(&path, &exe)?;
        if removed {
            notify(&path, SHCNE_DELETE);
        }
        Ok(removed)
    }

    #[cfg(debug_assertions)]
    fn trace_shortcut(operation: &str, path: &Path, exe: &Path) {
        if std::env::var_os("REELINK_QA_SHORTCUT_TRACE").is_some() {
            eprintln!(
                "capture-shortcut {operation}: exe={exe:?}, path={path:?}, physical={:?}, owned={:?}",
                path.canonicalize(),
                owned_at(path, exe).map(|value| value.map(|(_, hotkey)| hotkey))
            );
        }
    }

    fn remove_at(path: &Path, exe: &Path) -> Result<bool, String> {
        if owned_at(path, exe)?.is_none() {
            return Ok(false);
        }
        let original = fs::read(path).map_err(|err| err.to_string())?;
        if owned_at(path, exe)?.is_none()
            || fs::read(path).map_err(|err| err.to_string())? != original
        {
            return Err("截图快捷方式已变化，未删除。".into());
        }
        fs::remove_file(path).map_err(|err| format!("无法删除截图快捷方式：{err}"))?;
        Ok(true)
    }

    fn configure_at(path: &Path, exe: &Path, hotkey: u16) -> Result<bool, String> {
        configure_at_with(path, exe, hotkey, register_final)
    }

    fn configure_at_with(
        path: &Path,
        exe: &Path,
        hotkey: u16,
        register: impl FnOnce(&Path, u16, &[u8]) -> Result<(), String>,
    ) -> Result<bool, String> {
        let Some((link, _)) = owned_at(path, exe)? else {
            return Ok(false);
        };
        let original = fs::read(path).map_err(|err| format!("无法读取截图快捷方式：{err}"))?;
        save_at_with(&link, path, exe, hotkey, Some(&original), register)?;
        Ok(true)
    }

    fn save_at(
        link: &IShellLinkW,
        path: &Path,
        exe: &Path,
        hotkey: u16,
        original: Option<&[u8]>,
    ) -> Result<(), String> {
        save_at_with(link, path, exe, hotkey, original, register_final)
    }

    fn save_at_with(
        link: &IShellLinkW,
        path: &Path,
        exe: &Path,
        hotkey: u16,
        original: Option<&[u8]>,
        register: impl FnOnce(&Path, u16, &[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        let temporary =
            path.with_file_name(format!("reelink-capture-{}.tmp", uuid::Uuid::new_v4()));
        let mut temporary_created = false;
        let result = (|| {
            unsafe { link.SetHotkey(hotkey) }.map_err(|err| err.to_string())?;
            // IPersistFile::Save can register the shortcut's hotkey against its
            // filename immediately. Never pass the staging name to Shell COM:
            // Explorer could retain it after our atomic rename and open .tmp.
            let bytes = serialize_link(link)?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|err| format!("无法暂存截图快捷方式：{err}"))?;
            temporary_created = true;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|err| format!("无法保存截图快捷方式：{err}"))?;
            drop(file);
            let temporary_wide = wide(&temporary);
            let saved = deserialize_link(&fs::read(&temporary).map_err(|err| err.to_string())?)?;
            if !owns_link(&saved, exe)? {
                return Err("截图快捷方式写入校验失败，原设置未更改。".into());
            }
            let saved_hotkey = unsafe { saved.GetHotkey() }.map_err(|err| err.to_string())?;
            if saved_hotkey != hotkey {
                return Err("截图快捷键写入校验失败，原设置未更改。".into());
            }
            // Recheck an existing link immediately before replacement. New
            // installs use a no-replace move, so a concurrently created file wins.
            if let Some(original) = original {
                if owned_at(path, exe)?.is_none()
                    || fs::read(path).map_err(|err| err.to_string())? != original
                {
                    return Err("截图快捷方式已变化，请重新尝试。".into());
                }
            }
            let backup = original
                .map(|bytes| create_backup(path, bytes))
                .transpose()?;
            let target = wide(path);
            // Same-directory replacement is atomic. Until it succeeds the old
            // link stays intact, so any pre-commit failure needs no rollback.
            if unsafe {
                MoveFileExW(
                    temporary_wide.as_ptr(),
                    target.as_ptr(),
                    MOVEFILE_WRITE_THROUGH
                        | if original.is_some() {
                            MOVEFILE_REPLACE_EXISTING
                        } else {
                            0
                        },
                )
            } == 0
            {
                let error = std::io::Error::last_os_error();
                if let Some(backup) = &backup {
                    let _ = fs::remove_file(backup);
                }
                return Err(format!("无法更新截图快捷方式：{error}"));
            }
            finish_registration(path, exe, hotkey, &bytes, backup.as_deref(), register)
        })();
        if temporary_created {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    fn create_backup(path: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
        let backup = path.with_file_name(format!("reelink-capture-{}.bak", uuid::Uuid::new_v4()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup)
            .map_err(|err| format!("无法备份截图快捷方式：{err}"))?;
        if let Err(err) = file.write_all(bytes).and_then(|_| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(&backup);
            return Err(format!("无法备份截图快捷方式：{err}"));
        }
        Ok(backup)
    }

    fn matches_committed(path: &Path, bytes: &[u8]) -> bool {
        fs::symlink_metadata(path).is_ok_and(|metadata| {
            metadata.is_file() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
        }) && fs::read(path).is_ok_and(|current| current == bytes)
    }

    fn finish_registration(
        path: &Path,
        exe: &Path,
        hotkey: u16,
        committed: &[u8],
        backup: Option<&Path>,
        register: impl FnOnce(&Path, u16, &[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        // The final Shell save is necessary for hotkey registration and is not
        // atomic. Keep the previous bytes until registration and readback finish.
        let result = (|| {
            // Allow native Save to write, but prevent a replacement/reparse
            // substitution throughout validation, registration and readback.
            let _guard = fs::OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(path)
                .map_err(|err| format!("无法保护截图快捷方式：{err}"))?;
            if !matches_committed(path, committed) {
                return Err("截图快捷方式已变化，未注册快捷键。".into());
            }
            register(path, hotkey, committed)?;
            if !owned_at(path, exe)?.is_some_and(|(_, value)| value == hotkey) {
                return Err("系统快捷方式注册后的校验失败。".into());
            }
            Ok(())
        })();
        if result.is_ok() {
            if let Some(backup) = backup {
                if let Err(err) = fs::remove_file(backup) {
                    // Registration succeeded: housekeeping must not leave the
                    // caller's preferences behind the new Shell configuration.
                    eprintln!("快捷方式已保存，但无法清理备份 {}：{err}", backup.display());
                }
            }
            return result;
        }
        let error = result.unwrap_err();
        // Never repair a file whose current contents could belong to another
        // writer. A partial native Save therefore leaves a recoverable backup.
        if let Some(backup) = backup {
            if restore_previous(path, committed, backup).is_ok() {
                let _ = fs::remove_file(backup);
                return Err(format!(
                    "系统截图快捷键注册失败：{error}；已恢复原文件，系统按键注册状态尚未确认。"
                ));
            }
        }
        match backup {
            Some(backup) => Err(format!(
                "系统截图快捷键注册失败：{error}；当前文件已保留，原快捷方式备份位于 {}。",
                backup.display()
            )),
            None => Err(format!(
                "系统截图快捷键注册失败：{error}；当前文件已保留在 {}，请检查后重试。",
                path.display()
            )),
        }
    }

    fn restore_previous(path: &Path, committed: &[u8], backup: &Path) -> Result<(), String> {
        let previous = fs::read(backup).map_err(|err| err.to_string())?;
        // Exclusive sharing prevents another writer/replacement between the
        // byte comparison and restoration. OPEN_REPARSE_POINT avoids following
        // a concurrently substituted symlink. If any check fails, retain backup.
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|err| err.to_string())?;
        let metadata = file.metadata().map_err(|err| err.to_string())?;
        if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("截图快捷方式已变化。".into());
        }
        let mut current = Vec::new();
        file.read_to_end(&mut current)
            .map_err(|err| err.to_string())?;
        if current != committed {
            return Err("截图快捷方式已变化。".into());
        }
        file.rewind()
            .and_then(|_| file.write_all(&previous))
            .and_then(|_| file.set_len(previous.len() as u64))
            .and_then(|_| file.sync_all())
            .map_err(|err| err.to_string())
    }

    #[cfg(not(test))]
    fn register_final(path: &Path, hotkey: u16, committed: &[u8]) -> Result<(), String> {
        // Never reload mutable disk bytes into COM before registration.
        let link = deserialize_link(committed)?;
        let persist: IPersistFile = link.cast().map_err(|err| err.to_string())?;
        // An unchanged Save does not make Explorer register a new link. Dirty
        // the property in memory, but never save an intermediate combination.
        unsafe {
            link.SetHotkey(0).map_err(|err| err.to_string())?;
            link.SetHotkey(hotkey).map_err(|err| err.to_string())?;
            let final_path = wide(path);
            persist
                .Save(PCWSTR(final_path.as_ptr()), true)
                .map_err(|err| err.to_string())?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn register_final(_path: &Path, _hotkey: u16, _committed: &[u8]) -> Result<(), String> {
        // Unit fixtures must never register keys in the user's desktop Shell.
        Ok(())
    }

    fn serialize_link(link: &IShellLinkW) -> Result<Vec<u8>, String> {
        let stream = unsafe { SHCreateMemStream(None) }.ok_or("无法创建快捷方式内存流。")?;
        let persist: IPersistStream = link.cast().map_err(|err| err.to_string())?;
        unsafe { persist.Save(&stream, false) }.map_err(|err| err.to_string())?;
        let mut length = 0;
        unsafe { stream.Seek(0, STREAM_SEEK_CUR, Some(&mut length)) }
            .map_err(|err| err.to_string())?;
        // Keep malformed/unexpected COM output from causing an unbounded allocation.
        if length == 0 || length > 16 * 1024 * 1024 {
            return Err("截图快捷方式序列化大小无效。".into());
        }
        let mut bytes = vec![0u8; length as usize];
        let mut read = 0;
        unsafe {
            stream
                .Seek(0, STREAM_SEEK_SET, None)
                .map_err(|err| err.to_string())?;
            stream
                .Read(
                    bytes.as_mut_ptr().cast(),
                    bytes.len() as u32,
                    Some(&mut read),
                )
                .ok()
                .map_err(|err| err.to_string())?;
        }
        if read as usize != bytes.len() {
            return Err("截图快捷方式内存流不完整。".into());
        }
        Ok(bytes)
    }

    fn deserialize_link(bytes: &[u8]) -> Result<IShellLinkW, String> {
        let stream = unsafe { SHCreateMemStream(Some(bytes)) }.ok_or("无法读取快捷方式内存流。")?;
        let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
            .map_err(|err| err.to_string())?;
        let persist: IPersistStream = link.cast().map_err(|err| err.to_string())?;
        unsafe { persist.Load(&stream) }.map_err(|err| err.to_string())?;
        Ok(link)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn start_menu_directory_must_be_local_and_regular() {
            for path in [
                r"\\server\share\Programs",
                r"\\?\C:\Programs",
                r"\\.\C:\Programs",
                r"C:Programs",
                r"Programs",
            ] {
                assert!(validate_local_directory(Path::new(path)).is_err());
            }
            let root = tempfile::tempdir().unwrap();
            // TempDir may return an extended-length path; use the same ordinary
            // spelling as a Windows Known Folder result.
            let root_path = local_exe_path(root.path()).unwrap();
            let directory = root_path.join("Programs");
            fs::create_dir(&directory).unwrap();
            validate_local_directory(&directory).unwrap();
            let file = root_path.join("not-a-directory");
            fs::write(&file, b"preserve").unwrap();
            assert!(validate_local_directory(&file.join("Programs")).is_err());
            assert_eq!(fs::read(file).unwrap(), b"preserve");
        }

        #[test]
        fn rejects_foreign_or_nonlocal_target_before_filesystem_access() {
            let exe = Path::new(r"C:\Apps\Reelink\reelink.exe");
            assert!(same_local_target(
                Path::new(r"c:\apps\reelink\REELINK.exe"),
                exe
            ));
            for path in [
                r"\\server\share\reelink.exe",
                r"\\?\C:\Apps\Reelink\reelink.exe",
                r"\\.\C:\Apps\Reelink\reelink.exe",
                r"C:reelink.exe",
                r"reelink.exe",
                r"C:\Apps\Other\reelink.exe",
                r"C:\Apps\Other\..\Reelink\reelink.exe",
            ] {
                assert!(!same_local_target(Path::new(path), exe), "accepted {path}");
            }
        }

        fn write_link(path: &Path, exe: &Path, arguments: &str, hotkey: u16) {
            let link: IShellLinkW =
                unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }.unwrap();
            let exe = wide(exe);
            unsafe {
                link.SetPath(PCWSTR(exe.as_ptr())).unwrap();
                link.SetArguments(&HSTRING::from(arguments)).unwrap();
                link.SetHotkey(hotkey).unwrap();
            }
            // Tests must not register temporary hotkeys in the user's Shell.
            fs::write(path, serialize_link(&link).unwrap()).unwrap();
        }

        #[test]
        fn stream_serialization_is_a_readable_shell_link_file() {
            let _com = ComApartment::enter().unwrap();
            let root = tempfile::tempdir().unwrap();
            let exe = root.path().join("reelink.exe");
            fs::write(&exe, b"test executable").unwrap();
            let path = root.path().join("capture.lnk");
            write_link(&path, &exe, "--capture", 0x0451);
            let bytes = fs::read(&path).unwrap();
            // MS-SHLLINK ShellLinkHeader: HeaderSize and Shell Link CLSID.
            assert_eq!(&bytes[..4], &[0x4c, 0, 0, 0]);
            assert_eq!(
                &bytes[4..20],
                &[1, 0x14, 2, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46]
            );
            let memory_link = deserialize_link(&bytes).unwrap();
            assert!(owns_link(&memory_link, &exe).unwrap());
            assert_eq!(unsafe { memory_link.GetHotkey() }.unwrap(), 0x0451);
            // The final on-disk file must remain readable via normal Shell APIs.
            let disk_link = load_link(&path).unwrap();
            assert!(owns_link(&disk_link, &exe).unwrap());
            assert_eq!(unsafe { disk_link.GetHotkey() }.unwrap(), 0x0451);
        }

        #[test]
        fn updates_only_exact_owned_capture_link() {
            let _com = ComApartment::enter().unwrap();
            let root = tempfile::tempdir().unwrap();
            let exe = root.path().join("reelink.exe");
            fs::write(&exe, b"test executable").unwrap();
            let other = root.path().join("other.exe");
            fs::write(&other, b"another executable").unwrap();
            let link = root.path().join("capture.lnk");
            assert!(!configure_at(&link, &exe, 0x0451).unwrap());
            assert!(!link.exists());
            for (target, args) in [(&exe, ""), (&exe, "--capture extra"), (&other, "--capture")] {
                write_link(&link, target, args, 0);
                let before = fs::read(&link).unwrap();
                assert!(!configure_at(&link, &exe, 0x0451).unwrap());
                assert_eq!(fs::read(&link).unwrap(), before);
            }
            write_link(&link, &exe, "--capture", 0x0451);
            assert!(configure_at(&link, &exe, 0x067B).unwrap());
            assert_eq!(owned_at(&link, &exe).unwrap().unwrap().1, 0x067B);
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 3);
        }

        #[test]
        fn installer_creates_updates_and_removes_only_owned_link() {
            let _com = ComApartment::enter().unwrap();
            let root = tempfile::tempdir().unwrap();
            let exe = root.path().join("reelink.exe");
            fs::write(&exe, b"test executable").unwrap();
            let link = root.path().join("capture.lnk");
            assert!(install_at(&link, &exe, 0x067B).unwrap());
            assert_eq!(owned_at(&link, &exe).unwrap().unwrap().1, 0x067B);
            assert!(!install_at(&link, &exe, 0x0451).unwrap());
            assert_eq!(owned_at(&link, &exe).unwrap().unwrap().1, 0x0451);
            assert!(remove_at(&link, &exe).unwrap());
            assert!(!remove_at(&link, &exe).unwrap());
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
        }

        #[test]
        fn installer_preserves_foreign_links_and_invalid_files() {
            let _com = ComApartment::enter().unwrap();
            let root = tempfile::tempdir().unwrap();
            let exe = root.path().join("reelink.exe");
            let other = root.path().join("other.exe");
            fs::write(&exe, b"test executable").unwrap();
            fs::write(&other, b"another executable").unwrap();
            let link = root.path().join("capture.lnk");
            for (target, args) in [(&exe, ""), (&exe, "--capture extra"), (&other, "--capture")] {
                write_link(&link, target, args, 0x0451);
                let before = fs::read(&link).unwrap();
                assert!(install_at(&link, &exe, 0x067B).is_err());
                assert!(!remove_at(&link, &exe).unwrap());
                assert_eq!(fs::read(&link).unwrap(), before);
            }
            fs::write(&link, b"not a shell link").unwrap();
            assert!(install_at(&link, &exe, 0x067B).is_err());
            assert!(remove_at(&link, &exe).is_err());
            assert_eq!(fs::read(&link).unwrap(), b"not a shell link");
        }

        #[test]
        fn new_link_commit_never_overwrites_a_concurrently_created_file() {
            let _com = ComApartment::enter().unwrap();
            let root = tempfile::tempdir().unwrap();
            let exe = root.path().join("reelink.exe");
            fs::write(&exe, b"test executable").unwrap();
            let source = root.path().join("source.lnk");
            write_link(&source, &exe, "--capture", 0x0451);
            let shell_link = load_link(&source).unwrap();
            let destination = root.path().join("capture.lnk");
            fs::write(&destination, b"concurrent user file").unwrap();
            assert!(save_at(&shell_link, &destination, &exe, 0x067B, None).is_err());
            assert_eq!(fs::read(&destination).unwrap(), b"concurrent user file");
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 3);
        }

        #[test]
        fn unchanged_hotkey_still_requests_final_path_registration() {
            let _com = ComApartment::enter().unwrap();
            let root = tempfile::tempdir().unwrap();
            let exe = root.path().join("reelink.exe");
            fs::write(&exe, b"test").unwrap();
            let path = root.path().join("capture.lnk");
            write_link(&path, &exe, "--capture", 0x0451);
            let mut called = false;
            assert!(
                configure_at_with(&path, &exe, 0x0451, |final_path, hotkey, _| {
                    assert_eq!(final_path, path);
                    assert_eq!(hotkey, 0x0451);
                    called = true;
                    Ok(())
                })
                .unwrap()
            );
            assert!(called);
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
        }

        #[test]
        fn registration_guard_blocks_concurrent_replacement() {
            let _com = ComApartment::enter().unwrap();
            let root = tempfile::tempdir().unwrap();
            let exe = root.path().join("reelink.exe");
            fs::write(&exe, b"test").unwrap();
            let path = root.path().join("capture.lnk");
            write_link(&path, &exe, "--capture", 0x0451);
            let foreign = root.path().join("foreign.tmp");
            fs::write(&foreign, b"foreign").unwrap();
            assert!(configure_at_with(&path, &exe, 0x0451, |final_path, _, _| {
                let source = wide(&foreign);
                let target = wide(final_path);
                assert_eq!(
                    unsafe {
                        MoveFileExW(source.as_ptr(), target.as_ptr(), MOVEFILE_REPLACE_EXISTING)
                    },
                    0
                );
                assert!(fs::remove_file(final_path).is_err());
                Ok(())
            })
            .unwrap());
            assert_eq!(fs::read(foreign).unwrap(), b"foreign");
        }

        #[test]
        fn registration_failure_restores_previous_bytes() {
            let _com = ComApartment::enter().unwrap();
            let root = tempfile::tempdir().unwrap();
            let exe = root.path().join("reelink.exe");
            fs::write(&exe, b"test").unwrap();
            let path = root.path().join("capture.lnk");
            write_link(&path, &exe, "--capture", 0x0451);
            let before = fs::read(&path).unwrap();
            let link = load_link(&path).unwrap();
            let result = save_at_with(
                &link,
                &path,
                &exe,
                0x067B,
                Some(&before),
                |final_path, hotkey, _| {
                    assert_eq!(final_path, path);
                    assert_eq!(owned_at(final_path, &exe).unwrap().unwrap().1, hotkey);
                    Err("injected registration failure".into())
                },
            );
            assert!(result.is_err());
            assert_eq!(fs::read(&path).unwrap(), before);
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
        }

        #[test]
        fn registration_failure_preserves_concurrent_file_and_recovery_backup() {
            let _com = ComApartment::enter().unwrap();
            let root = tempfile::tempdir().unwrap();
            let exe = root.path().join("reelink.exe");
            fs::write(&exe, b"test").unwrap();
            let path = root.path().join("capture.lnk");
            write_link(&path, &exe, "--capture", 0x0451);
            let before = fs::read(&path).unwrap();
            let link = load_link(&path).unwrap();
            assert!(save_at_with(
                &link,
                &path,
                &exe,
                0x067B,
                Some(&before),
                |final_path, _, _| {
                    fs::write(final_path, b"concurrent foreign data").unwrap();
                    Err("injected registration failure".into())
                }
            )
            .is_err());
            assert_eq!(fs::read(&path).unwrap(), b"concurrent foreign data");
            let backups: Vec<_> = fs::read_dir(root.path())
                .unwrap()
                .map(|item| item.unwrap().path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "bak"))
                .collect();
            assert_eq!(backups.len(), 1);
            assert_eq!(fs::read(&backups[0]).unwrap(), before);
        }

        #[test]
        fn failed_atomic_replacement_keeps_previous_link() {
            let _com = ComApartment::enter().unwrap();
            let root = tempfile::tempdir().unwrap();
            let exe = root.path().join("reelink.exe");
            fs::write(&exe, b"test executable").unwrap();
            let link = root.path().join("capture.lnk");
            write_link(&link, &exe, "--capture", 0x0451);
            let before = fs::read(&link).unwrap();
            let mut permissions = fs::metadata(&link).unwrap().permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&link, permissions.clone()).unwrap();
            let result = configure_at(&link, &exe, 0x067B);
            permissions.set_readonly(false);
            fs::set_permissions(&link, permissions).unwrap();
            assert!(result.is_err());
            assert_eq!(fs::read(&link).unwrap(), before);
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_modifier_encoding_is_distinct_from_register_hotkey() {
        assert_eq!(hotkey_word("Alt+Q").unwrap(), 0x0451);
        assert_eq!(hotkey_word("Ctrl+Alt+F12").unwrap(), 0x067B);
        assert_eq!(hotkey_word("Shift+Ctrl+7").unwrap(), 0x0337);
        assert!(hotkey_word("Shift+Q").is_err());
        assert!(hotkey_word("Meta+Q").is_err());
    }
}
