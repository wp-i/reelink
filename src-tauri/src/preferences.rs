use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use tauri_plugin_global_shortcut::Shortcut;
use uuid::Uuid;

pub const DEFAULT_SHORTCUT: &str = "Ctrl+Shift+Q";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SavedPreferences {
    pub shortcut: String,
}

impl Default for SavedPreferences {
    fn default() -> Self {
        Self {
            shortcut: DEFAULT_SHORTCUT.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreferencesView {
    pub shortcut: String,
    pub shortcut_available: bool,
}

impl SavedPreferences {
    pub fn view(&self, shortcut_available: bool) -> PreferencesView {
        PreferencesView {
            shortcut: self.shortcut.clone(),
            shortcut_available,
        }
    }
}

pub fn normalize_shortcut(input: &str) -> Result<(String, Shortcut), String> {
    let parts = input.split('+').map(str::trim).collect::<Vec<_>>();
    if !(2..=4).contains(&parts.len()) || parts.iter().any(|part| part.is_empty()) {
        return Err("快捷键需要修饰键和一个字母、数字或 F1–F12。".into());
    }
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    for modifier in &parts[..parts.len() - 1] {
        match modifier.to_ascii_uppercase().as_str() {
            "CTRL" | "CONTROL" if !ctrl => ctrl = true,
            "ALT" if !alt => alt = true,
            "SHIFT" if !shift => shift = true,
            _ => return Err("快捷键修饰键只能使用 Ctrl、Alt、Shift，且不可重复。".into()),
        }
    }
    if !ctrl && !alt {
        return Err("快捷键至少需要 Ctrl 或 Alt。".into());
    }
    let key = parts.last().unwrap().to_ascii_uppercase();
    let valid = (key.len() == 1
        && key
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit()))
        || key
            .strip_prefix('F')
            .and_then(|number| number.parse::<u8>().ok())
            .is_some_and(|number| (1..=12).contains(&number) && format!("F{number}") == key);
    if !valid {
        return Err("快捷键主键只能使用字母、数字或 F1–F12。".into());
    }
    let mut tokens = Vec::new();
    if ctrl {
        tokens.push("Ctrl");
    }
    if alt {
        tokens.push("Alt");
    }
    if shift {
        tokens.push("Shift");
    }
    tokens.push(&key);
    let canonical = tokens.join("+");
    let shortcut = canonical
        .parse::<Shortcut>()
        .map_err(|_| "系统不支持这个快捷键组合。".to_string())?;
    Ok((canonical, shortcut))
}

pub fn load(path: &Path) -> Result<SavedPreferences, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SavedPreferences::default())
        }
        Err(err) => return Err(format!("无法读取设置：{err}")),
    };
    let mut saved: SavedPreferences =
        serde_json::from_slice(&bytes).map_err(|err| format!("设置文件格式错误：{err}"))?;
    saved.shortcut = normalize_shortcut(&saved.shortcut)?.0;
    Ok(saved)
}

pub fn save(path: &Path, saved: &SavedPreferences) -> Result<(), String> {
    let parent = path.parent().ok_or("设置路径缺少父目录")?;
    fs::create_dir_all(parent).map_err(|err| format!("无法创建设置目录：{err}"))?;
    let temporary = temp_path(path);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|err| format!("无法写入设置：{err}"))?;
    let write_result = (|| -> Result<(), String> {
        serde_json::to_writer_pretty(&mut file, saved)
            .map_err(|err| format!("无法写入设置：{err}"))?;
        file.write_all(b"\n")
            .and_then(|_| file.sync_all())
            .map_err(|err| format!("无法保存设置：{err}"))
    })();
    drop(file);
    if let Err(err) = write_result {
        let _ = fs::remove_file(&temporary);
        return Err(err);
    }
    if let Err(err) = replace_file(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(err);
    }
    Ok(())
}

fn temp_path(path: &Path) -> PathBuf {
    path.with_file_name(format!("preferences-{}.tmp", Uuid::new_v4()))
}

#[cfg(windows)]
fn replace_file(source: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let src = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let dst = target
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let flags = MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH;
    if unsafe { MoveFileExW(src.as_ptr(), dst.as_ptr(), flags) } == 0 {
        return Err(format!(
            "无法替换设置文件：{}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, target: &Path) -> Result<(), String> {
    fs::rename(source, target).map_err(|err| format!("无法替换设置文件：{err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn canonicalizes_only_supported_shortcuts() {
        assert_eq!(
            normalize_shortcut(" shift + ctrl + q ").unwrap().0,
            "Ctrl+Shift+Q"
        );
        assert_eq!(normalize_shortcut("alt+f12").unwrap().0, "Alt+F12");
        assert_eq!(normalize_shortcut("CONTROL+alt+7").unwrap().0, "Ctrl+Alt+7");
        for input in [
            "Q",
            "Shift+Q",
            "Ctrl+Ctrl+Q",
            "Ctrl+Alt+F13",
            "Ctrl+Escape",
            "Ctrl+F01",
            "Meta+Q",
            "Ctrl+Q+R",
        ] {
            assert!(
                normalize_shortcut(input).is_err(),
                "unexpectedly accepted: {input}"
            );
        }
    }

    #[test]
    fn persists_and_loads_preferences_without_exposing_availability() {
        let root = tempdir().unwrap();
        let path = root.path().join("nested").join("preferences.json");
        assert_eq!(load(&path).unwrap(), SavedPreferences::default());
        let saved = SavedPreferences {
            shortcut: "Alt+F12".into(),
        };
        save(&path, &saved).unwrap();
        assert_eq!(load(&path).unwrap(), saved);
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("autoCopy"));
        assert!(!text.contains("shortcutAvailable"));
        let next = SavedPreferences {
            shortcut: DEFAULT_SHORTCUT.into(),
        };
        save(&path, &next).unwrap();
        assert_eq!(load(&path).unwrap(), next);
    }

    #[test]
    fn ignores_legacy_auto_copy_preference() {
        let root = tempdir().unwrap();
        let path = root.path().join("preferences.json");
        fs::write(&path, r#"{"autoCopy":true,"shortcut":"Alt+F12"}"#).unwrap();

        let saved = load(&path).unwrap();
        assert_eq!(
            saved,
            SavedPreferences {
                shortcut: "Alt+F12".into()
            }
        );
        save(&path, &saved).unwrap();
        let text = fs::read_to_string(path).unwrap();
        assert!(!text.contains("autoCopy"));
    }
}
