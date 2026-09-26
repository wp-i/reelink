use chrono::{Datelike, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;
use uuid::Uuid;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameItem {
    pub id: String,
    pub source_path: String,
    pub original_name: String,
    pub suggested_name: String,
    pub title: String,
    pub year: Option<String>,
    pub extension: String,
    pub kind: String,
    pub is_directory: bool,
    pub group_id: Option<String>,
    pub episode: Option<String>,
    pub confidence: String,
    pub notes: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameEdit {
    pub id: String,
    pub source_path: String,
    pub target_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameOutcome {
    pub count: usize,
    pub message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySummary {
    pub count: usize,
    pub created_at: String,
}

#[derive(Clone)]
struct Previewed {
    path: PathBuf,
    extension: String,
    fingerprint: Fingerprint,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Fingerprint {
    is_dir: bool,
    size: u64,
    modified_ns: Option<u128>,
    created_ns: Option<u128>,
    #[serde(default)]
    file_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct JournalEntry {
    source: PathBuf,
    target: PathBuf,
    fingerprint: Fingerprint,
}

#[derive(Serialize, Deserialize)]
struct JournalHeader {
    version: u32,
    created_at: String,
    entries: Vec<JournalEntry>,
}

pub struct RenameEngine {
    data_dir: PathBuf,
    previewed: HashMap<String, Previewed>,
}

impl RenameEngine {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            previewed: HashMap::new(),
        }
    }

    pub fn preview(&mut self, paths: Vec<String>) -> Result<Vec<RenameItem>, String> {
        self.previewed.clear();
        let mut selected = Vec::new();
        let mut seen = HashSet::new();
        for supplied in paths {
            let path = checked_path(Path::new(&supplied))?;
            if path.is_file() && !is_video(&path) {
                return Err(format!("不支持的视频格式： {}", path.display()));
            }
            if !path.is_file() && !path.is_dir() {
                return Err(format!("请选择视频文件或文件夹： {}", path.display()));
            }
            if seen.insert(path.clone()) {
                selected.push(path);
            }
        }
        selected.sort();
        selected.retain(|p| {
            !seen
                .iter()
                .any(|other| other != p && other.is_dir() && p.starts_with(other))
        });
        let mut videos = Vec::new();
        for path in &selected {
            if path.is_file() {
                videos.push(path.clone());
            } else {
                collect_videos(path, 0, &mut videos)?;
            }
        }
        videos.sort();
        videos.dedup();
        let mut groups: HashMap<PathBuf, (String, Parsed)> = HashMap::new();
        for path in &videos {
            let name = file_name(path)?;
            let ext = extension(path)?;
            if episode(&name[..name.len() - ext.len()]).is_some()
                && selected
                    .iter()
                    .any(|root| root.is_dir() && path.starts_with(root))
            {
                let folder = series_folder(path, &selected)?;
                groups.entry(folder.clone()).or_insert_with(|| {
                    let title_folder = if season_number(&folder).is_some() {
                        folder.parent().unwrap_or(&folder)
                    } else {
                        &folder
                    };
                    let parsed =
                        parse_movie(title_folder.file_name().unwrap().to_string_lossy().as_ref());
                    (Uuid::new_v4().to_string(), parsed)
                });
            }
        }
        let mut result = Vec::new();
        let mut folders: Vec<_> = groups.iter().collect();
        folders.sort_by(|a, b| a.0.cmp(b.0));
        for (path, (group_id, parsed)) in folders {
            let title = safe_suggestion_title(&parsed.title);
            let suggested = match &parsed.year {
                Some(year) => format!("{title} ({year})"),
                None => title.clone(),
            };
            if season_number(path).is_none() {
                self.push_item(
                    &mut result,
                    path,
                    suggested,
                    title,
                    parsed,
                    "tv",
                    Some(group_id.clone()),
                    None,
                )?;
            }
        }
        for path in &videos {
            let name = file_name(path)?;
            let ext = extension(path)?;
            let stem = &name[..name.len() - ext.len()];
            if let Some((position, _, _, _)) = episode(stem) {
                let folder = series_folder(path, &selected)?;
                let standalone = parse_movie(if position > 0 {
                    &stem[..position]
                } else {
                    folder.file_name().and_then(|s| s.to_str()).unwrap_or("")
                });
                let (group_id, parsed) = groups
                    .get(&folder)
                    .map(|(id, parsed)| (Some(id.clone()), parsed))
                    .unwrap_or((None, &standalone));
                let title = safe_suggestion_title(&parsed.title);
                let code = episode_code(stem, season_number(path.parent().unwrap()));
                let suggested = code
                    .as_ref()
                    .map(|code| format!("{title} - {code}{ext}"))
                    .unwrap_or_else(|| name.clone());
                self.push_item(
                    &mut result,
                    path,
                    suggested,
                    title,
                    parsed,
                    "tv",
                    group_id,
                    code,
                )?;
            } else if let Some((_, (group_id, parsed))) = groups
                .iter()
                .filter(|(root, _)| path.starts_with(root))
                .max_by_key(|(root, _)| root.components().count())
            {
                self.push_item(
                    &mut result,
                    path,
                    name.clone(),
                    parsed.title.clone(),
                    parsed,
                    "tv",
                    Some(group_id.clone()),
                    None,
                )?;
                if let Some(item) = result.last_mut() {
                    item.confidence = "review".into();
                    item.notes = vec!["未识别到集数，保留原名称".into()];
                }
            } else {
                let parsed = parse_movie(stem);
                let title = safe_suggestion_title(&parsed.title);
                let suggested = match &parsed.year {
                    Some(year) => format!("{title} ({year}){ext}"),
                    None => format!("{title}{ext}"),
                };
                self.push_item(
                    &mut result,
                    path,
                    suggested,
                    title,
                    &parsed,
                    "movie",
                    None,
                    None,
                )?;
            }
        }
        result.sort_by(|a, b| {
            let key = |item: &RenameItem| {
                item.group_id
                    .as_ref()
                    .and_then(|id| {
                        groups
                            .iter()
                            .find(|(_, (gid, _))| gid == id)
                            .map(|(path, _)| path.to_string_lossy().into_owned())
                    })
                    .unwrap_or_else(|| item.source_path.clone())
            };
            key(a)
                .cmp(&key(b))
                .then_with(|| b.is_directory.cmp(&a.is_directory))
                .then_with(|| a.source_path.cmp(&b.source_path))
        });
        Ok(result)
    }

    fn push_item(
        &mut self,
        result: &mut Vec<RenameItem>,
        path: &Path,
        suggested_name: String,
        title: String,
        parsed: &Parsed,
        kind: &str,
        group_id: Option<String>,
        episode: Option<String>,
    ) -> Result<(), String> {
        let is_directory = path.is_dir();
        let extension = if is_directory {
            String::new()
        } else {
            extension(path)?
        };
        let id = Uuid::new_v4().to_string();
        self.previewed.insert(
            id.clone(),
            Previewed {
                path: path.to_path_buf(),
                extension: extension.clone(),
                fingerprint: fingerprint(path)?,
            },
        );
        result.push(RenameItem {
            id,
            source_path: path.to_string_lossy().into_owned(),
            original_name: file_name(path)?,
            suggested_name,
            title,
            year: parsed.year.clone(),
            extension,
            kind: kind.into(),
            is_directory,
            group_id,
            episode,
            confidence: if parsed.review { "review" } else { "high" }.into(),
            notes: parsed.notes.clone(),
        });
        Ok(())
    }

    pub fn apply(&mut self, items: Vec<RenameEdit>) -> Result<RenameOutcome, String> {
        if items.is_empty() {
            return Ok(RenameOutcome {
                count: 0,
                message: "没有需要更名的项目".into(),
            });
        }
        let mut used_tokens = HashSet::new();
        let mut targets = HashSet::new();
        let mut entries = Vec::new();
        for edit in &items {
            if !used_tokens.insert(edit.id.as_str()) {
                return Err("同一项目不能重复提交".into());
            }
            let source = self
                .previewed
                .get(&edit.id)
                .ok_or("预览已过期，请重新预览")?;
            if edit.source_path != source.path.to_string_lossy() {
                return Err("文件路径与预览不一致".into());
            }
            validate_target_name(&edit.target_name, &source.extension)?;
            checked_path(&source.path)?;
            if fingerprint(&source.path)? != source.fingerprint {
                return Err("文件自预览后已变化，请重新预览".into());
            }
            let target = source.path.with_file_name(&edit.target_name);
            if target == source.path {
                continue;
            }
            // Windows compares ordinary file names without regard to case.
            if !targets.insert(target.to_string_lossy().to_lowercase()) {
                return Err("多个文件使用了相同的目标名称".into());
            }
            if target.exists() || fs::symlink_metadata(&target).is_ok() {
                return Err(format!("目标文件已存在：{}", target.display()));
            }
            entries.push(JournalEntry {
                source: source.path.clone(),
                target,
                fingerprint: source.fingerprint.clone(),
            });
        }
        if entries.is_empty() {
            return Ok(RenameOutcome {
                count: 0,
                message: "名称没有变化".into(),
            });
        }
        // A target matching any source would require a multi-step swap and could overwrite data.
        let source_set: HashSet<_> = entries.iter().map(|e| e.source.as_path()).collect();
        if entries
            .iter()
            .any(|e| source_set.contains(e.target.as_path()))
        {
            return Err("目标名称与选中的现有文件冲突".into());
        }
        // Rename nested children before their parent folder.
        entries.sort_by(|a, b| {
            b.source
                .components()
                .count()
                .cmp(&a.source.components().count())
                .then_with(|| a.source.cmp(&b.source))
        });
        let created_at = Utc::now().to_rfc3339();
        let header = JournalHeader {
            version: 1,
            created_at,
            entries,
        };
        let journal = self.create_journal(&header)?;
        let mut completed = 0;
        for (index, entry) in header.entries.iter().enumerate() {
            if let Err(err) = checked_path(&entry.source).and_then(|_| {
                if !fingerprint_matches_undo(&entry.source, &entry.fingerprint) {
                    return Err("源文件在执行期间发生变化".into());
                }
                move_no_replace(&entry.source, &entry.target)
            }) {
                self.previewed.clear();
                return Err(format!(
                    "已重命名 {completed} 项，随后失败：{err}。可使用撤销恢复已完成的操作"
                ));
            }
            completed += 1;
            if let Err(err) = append_event(
                &journal,
                &format!("{{\"event\":\"done\",\"index\":{index}}}"),
            ) {
                self.previewed.clear();
                return Err(format!(
                    "已重命名 {completed} 项，但更新恢复记录失败：{err}。请使用撤销恢复"
                ));
            }
        }
        self.previewed.clear();
        Ok(RenameOutcome {
            count: completed,
            message: format!("已重命名 {completed} 项，可撤销"),
        })
    }

    pub fn undo(&mut self) -> Result<RenameOutcome, String> {
        let Some((path, header)) = self.latest_active_journal()? else {
            return Ok(RenameOutcome {
                count: 0,
                message: "没有可撤销的重命名".into(),
            });
        };
        let mut count = 0;
        let mut issues = Vec::new();
        let mut blocked_parents: Vec<PathBuf> = Vec::new();
        for entry in header.entries.iter().rev() {
            if blocked_parents
                .iter()
                .any(|parent| entry.source.starts_with(parent))
            {
                continue;
            }
            let issues_before = issues.len();
            if let Err(err) =
                checked_parent(&entry.source).and_then(|_| checked_parent(&entry.target))
            {
                issues.push(err);
                if entry.fingerprint.is_dir {
                    blocked_parents.push(entry.source.clone());
                }
                continue;
            }
            let source_meta = fs::symlink_metadata(&entry.source);
            let target_meta = fs::symlink_metadata(&entry.target);
            match (source_meta, target_meta) {
                (Ok(_), Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                    if !fingerprint_matches_undo(&entry.source, &entry.fingerprint) {
                        issues.push(format!("源文件已变化：{}", entry.source.display()));
                    }
                }
                (Err(e), Ok(_)) if e.kind() == std::io::ErrorKind::NotFound => {
                    if checked_path(&entry.target).is_err()
                        || !fingerprint_matches_undo(&entry.target, &entry.fingerprint)
                    {
                        issues.push(format!("目标文件已变化：{}", entry.target.display()));
                    } else if let Err(e) = move_no_replace(&entry.target, &entry.source) {
                        issues.push(format!("无法恢复 {}：{e}", entry.source.display()));
                    } else {
                        count += 1;
                    }
                }
                (Ok(_), Ok(_)) => {
                    issues.push(format!("源和目标同时存在：{}", entry.source.display()))
                }
                (Err(a), Err(b))
                    if a.kind() == std::io::ErrorKind::NotFound
                        && b.kind() == std::io::ErrorKind::NotFound =>
                {
                    issues.push(format!("源和目标均不存在：{}", entry.source.display()));
                }
                (Err(a), _) => issues.push(format!("无法检查源文件：{a}")),
                (_, Err(b)) => issues.push(format!("无法检查目标文件：{b}")),
            }
            if entry.fingerprint.is_dir && issues.len() > issues_before {
                blocked_parents.push(entry.source.clone());
            }
        }
        self.previewed.clear();
        if !issues.is_empty() {
            return Err(format!(
                "已恢复 {count} 项；仍有 {} 个问题。恢复记录已保留：{}",
                issues.len(),
                issues.join("；")
            ));
        }
        append_event(&path, "{\"event\":\"closed\"}")?;
        Ok(RenameOutcome {
            count,
            message: format!("已撤销 {count} 项的重命名"),
        })
    }

    pub fn history(&self) -> Result<Option<HistorySummary>, String> {
        let Some((_, header)) = self.latest_active_journal()? else {
            return Ok(None);
        };
        let count = header
            .entries
            .iter()
            .filter(|entry| {
                let mut parents: Vec<_> = header
                    .entries
                    .iter()
                    .filter(|parent| {
                        parent.fingerprint.is_dir
                            && parent.source != entry.source
                            && entry.source.starts_with(&parent.source)
                    })
                    .collect();
                parents.sort_by_key(|parent| parent.source.components().count());
                let mut mappings: Vec<(PathBuf, PathBuf)> = Vec::new();
                for parent in parents {
                    let source = mapped_path(&parent.source, &mappings);
                    let target = source.with_file_name(parent.target.file_name().unwrap());
                    if !source.exists() && target.exists() {
                        mappings.push((parent.source.clone(), target));
                    }
                }
                mapped_path(&entry.target, &mappings).exists()
                    && !mapped_path(&entry.source, &mappings).exists()
            })
            .count();
        Ok(Some(HistorySummary {
            count,
            created_at: header.created_at,
        }))
    }

    fn journal_dir(&self) -> PathBuf {
        self.data_dir.join("rename-journals")
    }

    fn create_journal(&self, header: &JournalHeader) -> Result<PathBuf, String> {
        let dir = self.journal_dir();
        fs::create_dir_all(&dir).map_err(|e| format!("无法创建恢复目录：{e}"))?;
        let name = format!(
            "{}-{}.jsonl",
            Utc::now().format("%Y%m%dT%H%M%S%.9fZ"),
            Uuid::new_v4()
        );
        let path = dir.join(name);
        let temporary = path.with_extension("jsonl.tmp");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| format!("无法创建恢复记录：{e}"))?;
        let write_result = (|| -> Result<(), String> {
            serde_json::to_writer(&mut file, header)
                .map_err(|e| format!("无法写入恢复记录：{e}"))?;
            file.write_all(b"\n")
                .map_err(|e| format!("无法写入恢复记录：{e}"))?;
            file.sync_all()
                .map_err(|e| format!("无法保存恢复记录：{e}"))
        })();
        if let Err(err) = write_result {
            drop(file);
            let _ = fs::remove_file(&temporary);
            return Err(err);
        }
        drop(file);
        if let Err(err) = move_no_replace(&temporary, &path) {
            let _ = fs::remove_file(&temporary);
            return Err(format!("无法发布恢复记录：{err}"));
        }
        Ok(path)
    }

    fn latest_active_journal(&self) -> Result<Option<(PathBuf, JournalHeader)>, String> {
        let dir = self.journal_dir();
        if !dir.exists() {
            return Ok(None);
        }
        let mut files = fs::read_dir(&dir)
            .map_err(|e| format!("无法读取恢复目录：{e}"))?
            .map(|entry| entry.map(|e| e.path()).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        files.retain(|p| p.extension().is_some_and(|ext| ext == "jsonl"));
        files.sort();
        for path in files.into_iter().rev() {
            let contents = fs::read(&path).map_err(|e| format!("无法读取恢复记录：{e}"))?;
            let Some(newline) = contents.iter().position(|byte| *byte == b'\n') else {
                // An incomplete old header could never authorize a move.
                continue;
            };
            let header: JournalHeader = serde_json::from_slice(&contents[..newline])
                .map_err(|e| format!("恢复记录损坏：{e}"))?;
            if header.version != 1 {
                return Err("恢复记录版本不受支持".into());
            }
            let closed = contents[newline + 1..]
                .split(|byte| *byte == b'\n')
                .filter_map(|line| serde_json::from_slice::<serde_json::Value>(line).ok())
                .any(|value| value.get("event").and_then(|v| v.as_str()) == Some("closed"));
            if !closed {
                return Ok(Some((path, header)));
            }
        }
        Ok(None)
    }
}

fn append_event(path: &Path, event: &str) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > 0 {
        file.seek(SeekFrom::End(-1)).map_err(|e| e.to_string())?;
        let mut last = [0u8];
        file.read_exact(&mut last).map_err(|e| e.to_string())?;
        if last[0] != b'\n' {
            file.write_all(b"\n").map_err(|e| e.to_string())?;
        }
    }
    file.write_all(event.as_bytes())
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}

fn checked_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(format!("需要完整且规范的路径：{}", path.display()));
    }
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)
            .map_err(|e| format!("无法检查路径 {}：{e}", ancestor.display()))?;
        if metadata.file_type().is_symlink() || is_reparse(&metadata) {
            return Err(format!("不允许符号链接或重解析点：{}", ancestor.display()));
        }
    }
    let canonical = fs::canonicalize(path).map_err(|e| format!("无法读取路径：{e}"))?;
    Ok(canonical)
}

fn checked_parent(path: &Path) -> Result<(), String> {
    checked_path(path.parent().ok_or("文件缺少父目录")?).map(|_| ())
}

#[cfg(windows)]
fn is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}
#[cfg(not(windows))]
fn is_reparse(_: &fs::Metadata) -> bool {
    false
}

fn fingerprint(path: &Path) -> Result<Fingerprint, String> {
    let m = fs::metadata(path).map_err(|e| format!("无法检查文件：{e}"))?;
    if !m.is_file() && !m.is_dir() {
        return Err("所选项目不是文件或文件夹".into());
    }
    let time_ns = |value: Result<std::time::SystemTime, std::io::Error>| {
        value
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
    };
    Ok(Fingerprint {
        is_dir: m.is_dir(),
        size: m.len(),
        modified_ns: time_ns(m.modified()),
        created_ns: time_ns(m.created()),
        file_id: file_identity(path)?,
    })
}

fn fingerprint_matches_undo(path: &Path, expected: &Fingerprint) -> bool {
    match fingerprint(path) {
        Ok(actual) if expected.is_dir => {
            actual.is_dir
                && actual.created_ns == expected.created_ns
                && expected
                    .file_id
                    .as_ref()
                    .is_none_or(|id| actual.file_id.as_ref() == Some(id))
        }
        Ok(mut actual) => {
            if expected.file_id.is_none() {
                actual.file_id = None;
            }
            actual == *expected
        }
        Err(_) => false,
    }
}

fn file_name(path: &Path) -> Result<String, String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
        .ok_or("文件名不是有效文本".into())
}

fn extension(path: &Path) -> Result<String, String> {
    let ext = path
        .extension()
        .and_then(|n| n.to_str())
        .ok_or("缺少视频扩展名")?;
    Ok(format!(".{ext}"))
}

fn is_video(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        matches!(
            s.to_ascii_lowercase().as_str(),
            "mkv" | "mp4" | "avi" | "mov" | "m4v" | "wmv" | "ts" | "webm" | "flv"
        )
    })
}

fn collect_videos(folder: &Path, depth: usize, videos: &mut Vec<PathBuf>) -> Result<(), String> {
    if depth > 8 || videos.len() >= 5_000 {
        return Err("所选文件夹超出扫描范围（最多 8 层、5000 个视频）".into());
    }
    let mut children = fs::read_dir(folder)
        .map_err(|e| format!("无法读取 {}：{e}", folder.display()))?
        .map(|e| e.map(|x| x.path()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    children.sort();
    for path in children {
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() || is_reparse(&metadata) {
            return Err(format!("所选文件夹包含链接或重解析点： {}", path.display()));
        }
        if metadata.is_dir() {
            collect_videos(&path, depth + 1, videos)?;
        } else if metadata.is_file() && is_video(&path) {
            videos.push(checked_path(&path)?);
        }
    }
    Ok(())
}

fn season_number(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    let pattern =
        Regex::new(r"(?i)^(?:season[ ._-]*(\d{1,2})|s(\d{1,2})|第\s*(\d{1,2})\s*季)$").unwrap();
    let caps = pattern.captures(name)?;
    (1..=3).find_map(|index| caps.get(index)?.as_str().parse().ok())
}

fn series_folder(episode_file: &Path, selected: &[PathBuf]) -> Result<PathBuf, String> {
    let parent = episode_file.parent().ok_or("视频缺少父目录")?;
    if season_number(parent).is_some() {
        if let Some(grandparent) = parent.parent() {
            if selected
                .iter()
                .any(|root| root.is_dir() && grandparent.starts_with(root))
            {
                return Ok(grandparent.to_path_buf());
            }
        }
    }
    Ok(parent.to_path_buf())
}

fn episode_code(stem: &str, folder_season: Option<u32>) -> Option<String> {
    let (_, season, number, explicit) = episode(stem)?;
    let season = if explicit {
        season
    } else {
        folder_season.unwrap_or(season)
    };
    // Retain multi-episode files as S01E01-E02, never silently drop the second episode.
    let standard =
        Regex::new(r"(?i)S\d{1,2}[ ._-]*E\d{1,3}(?P<tail>(?:[ ._-]*E\d{1,3}|-\d{1,3})*)").unwrap();
    let mut code = format!("S{season:02}E{number:02}");
    if let Some(caps) = standard.captures(stem) {
        if stem[caps.get(0).unwrap().end()..]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_digit())
        {
            return None;
        }
        let tail = caps.name("tail").unwrap().as_str();
        for digits in Regex::new(r"\d{1,3}").unwrap().find_iter(tail) {
            let next: u32 = digits.as_str().parse().ok()?;
            if next <= number {
                return None;
            }
            code.push_str(&format!("-E{next:02}"));
        }
    }
    Some(code)
}

fn mapped_path(path: &Path, mappings: &[(PathBuf, PathBuf)]) -> PathBuf {
    for (source, target) in mappings.iter().rev() {
        if let Ok(relative) = path.strip_prefix(source) {
            return target.join(relative);
        }
    }
    path.to_path_buf()
}

#[cfg(windows)]
fn file_identity(path: &Path) -> Result<Option<String>, String> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
    };
    let file = OpenOptions::new()
        .access_mode(0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map_err(|e| format!("无法检查文件标识：{e}"))?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(format!(
            "无法读取文件标识：{}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(Some(format!(
        "{}:{}:{}",
        info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
    )))
}
#[cfg(not(windows))]
fn file_identity(_: &Path) -> Result<Option<String>, String> {
    Ok(None)
}

fn validate_target_name(name: &str, extension: &str) -> Result<(), String> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.ends_with([' ', '.'])
        || name.chars().any(|c| c < ' ' || "<>:\"/\\|?*".contains(c))
    {
        return Err("目标文件名含有 Windows 不允许的字符".into());
    }
    if name.encode_utf16().count() > 255 {
        return Err("目标文件名过长".into());
    }
    if !name.ends_with(extension) || name.len() <= extension.len() {
        return Err("必须保留原始扩展名".into());
    }
    let base = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    if matches!(
        base.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "COM¹"
            | "COM²"
            | "COM³"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
            | "LPT¹"
            | "LPT²"
            | "LPT³"
    ) {
        return Err("目标文件名是 Windows 保留名称".into());
    }
    Ok(())
}

#[cfg(windows)]
fn move_no_replace(source: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileW;
    let src: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let dst: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    // MoveFileW fails when the destination exists; unlike replacement-capable rename APIs.
    if unsafe { MoveFileW(src.as_ptr(), dst.as_ptr()) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

#[cfg(not(windows))]
fn move_no_replace(source: &Path, target: &Path) -> Result<(), String> {
    // The application ships on Windows. Keep test behavior conservative on Unix.
    if target.exists() {
        return Err("目标已存在".into());
    }
    if source.is_dir() {
        return fs::rename(source, target).map_err(|e| e.to_string());
    }
    fs::hard_link(source, target).map_err(|e| e.to_string())?;
    if let Err(e) = fs::remove_file(source) {
        let _ = fs::remove_file(target);
        return Err(e.to_string());
    }
    Ok(())
}

struct Parsed {
    title: String,
    year: Option<String>,
    review: bool,
    notes: Vec<String>,
}

fn episode(stem: &str) -> Option<(usize, u32, u32, bool)> {
    let patterns = [
        (r"(?i)S(\d{1,2})[ ._-]*E(\d{1,3})(?:$|[^0-9])", true),
        (r"(?i)(\d{1,2})x(\d{1,3})(?:$|[^0-9])", true),
        (r"第\s*(\d{1,2})\s*季\s*第\s*(\d{1,3})\s*[集话]", true),
        (r"(?i)\b(?:EP?|Episode)[ ._-]*(\d{1,3})\b", false),
        (r"第\s*(\d{1,3})\s*[集话]", false),
    ];
    for (pattern, has_season) in patterns {
        let re = Regex::new(pattern).unwrap();
        if let Some(caps) = re.captures(stem) {
            let position = caps.get(0).unwrap().start();
            let first = caps.get(1)?.as_str().parse::<u32>().ok()?;
            let (season, ep) = if has_season {
                (first, caps.get(2)?.as_str().parse::<u32>().ok()?)
            } else {
                (1, first)
            };
            if season <= 99 && ep > 0 && ep <= 999 {
                return Some((position, season, ep, has_season));
            }
        }
    }
    None
}

fn parse_movie(stem: &str) -> Parsed {
    let clean = remove_leading_group(stem);
    let current = Utc::now().year() + 1;
    let tag_position = first_release_tag(&clean);
    let year_match = year_candidates(&clean)
        .into_iter()
        .filter(|(position, year)| {
            year.parse::<i32>().is_ok_and(|n| n <= current)
                && tag_position.is_none_or(|tag| *position < tag)
                && !normalize_title(&clean[..*position]).is_empty()
        })
        .last();
    let cutoff = match (&year_match, tag_position) {
        (Some((p, _)), Some(tag)) if tag < *p => tag,
        (Some((p, _)), _) => *p,
        (_, Some(tag)) => tag,
        _ => clean.len(),
    };
    let title = normalize_title(&clean[..cutoff]);
    let mut notes = Vec::new();
    if year_match.is_none() {
        notes.push("未识别到年份，请核对片名".into());
    }
    if title.is_empty() {
        notes.push("未能可靠识别片名".into());
    }
    let year = year_match.map(|(_, y)| y);
    Parsed {
        title: if title.is_empty() {
            normalize_title(&clean)
        } else {
            title
        },
        year,
        review: !notes.is_empty(),
        notes,
    }
}

fn year_candidates(input: &str) -> Vec<(usize, String)> {
    let re = Regex::new(r"(?:19|20)\d{2}").unwrap();
    re.find_iter(input)
        .filter(|m| {
            let before = input[..m.start()].chars().next_back();
            let after = input[m.end()..].chars().next();
            // A Chinese title can run directly into its year. ASCII title words and
            // longer digit sequences are too ambiguous to infer automatically.
            !before.is_some_and(|c| c.is_ascii_alphanumeric())
                && !after.is_some_and(|c| c.is_ascii_alphanumeric())
        })
        .map(|m| (m.start(), m.as_str().to_owned()))
        .collect()
}

fn remove_leading_group(input: &str) -> String {
    let re = Regex::new(r"^\s*(?:\[[^\]]{1,40}\]|【[^】]{1,40}】)[\s._-]*").unwrap();
    re.replace(input, "").into_owned()
}

fn first_release_tag(input: &str) -> Option<usize> {
    let re = Regex::new(r"(?i)(?:^|[\s._\-\[【(])(?:2160p|1080p|720p|480p|4k|uhd|bluray|blu-ray|bdrip|brrip|webrip|web[ ._-]?dl|hdtv|dvdrip|remux|x264|x265|h264|h265|hevc|av1|aac|dts|hdr10?|dolby|中字|字幕|双语|国英)(?:$|[\s._\-\]】)])").unwrap();
    re.find(input).map(|m| m.start())
}

fn normalize_title(input: &str) -> String {
    let separators = Regex::new(r"[._]+|\s+-\s+|\s{2,}").unwrap();
    separators
        .replace_all(input, " ")
        .trim_matches(|c: char| c.is_whitespace() || "-–—[]【】()（）".contains(c))
        .trim()
        .to_string()
}

fn safe_suggestion_title(input: &str) -> String {
    let cleaned: String = input
        .chars()
        .map(|c| {
            if c < ' ' || "<>:\"/\\|?*".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches([' ', '.']);
    let mut shortened = String::new();
    for ch in cleaned.chars() {
        if shortened.encode_utf16().count() + ch.len_utf16() > 180 {
            break;
        }
        shortened.push(ch);
    }
    let shortened = shortened.trim_end_matches([' ', '.']);
    if shortened.is_empty() {
        return "未命名".into();
    }
    let base = shortened
        .split('.')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    if matches!(
        base.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "COM¹"
            | "COM²"
            | "COM³"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
            | "LPT¹"
            | "LPT²"
            | "LPT³"
    ) {
        format!("_{shortened}")
    } else {
        shortened.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn movie_parse_handles_chinese_english_and_tags() {
        let a = parse_movie("流浪地球2.2023.2160p.WEB-DL.x265");
        assert_eq!(
            (&a.title, a.year.as_deref(), a.review),
            (&"流浪地球2".to_string(), Some("2023"), false)
        );
        let b = parse_movie("[Group]The.Matrix.1999.BluRay.1080p");
        assert_eq!(
            (&b.title, b.year.as_deref()),
            (&"The Matrix".to_string(), Some("1999"))
        );
        assert!(parse_movie("Unknown.Release.1080p").review);
        let chinese = parse_movie("你好2023.1080p");
        assert_eq!(
            (chinese.title.as_str(), chinese.year.as_deref()),
            ("你好", Some("2023"))
        );
        let numeric = parse_movie("1917");
        assert_eq!(
            (numeric.title.as_str(), numeric.year.as_deref()),
            ("1917", None)
        );
        let numeric2 = parse_movie("2001.A.Space.Odyssey.1968");
        assert_eq!(
            (numeric2.title.as_str(), numeric2.year.as_deref()),
            ("2001 A Space Odyssey", Some("1968"))
        );
    }

    #[test]
    fn deselecting_episodes_allows_folder_only_rename() {
        let root = tempdir().unwrap();
        let folder = root.path().join("三体.2023.1080p");
        fs::create_dir(&folder).unwrap();
        let episode_name = "三体S01E01.mkv";
        fs::write(folder.join(episode_name), b"episode bytes").unwrap();
        fs::create_dir(folder.join("Extras")).unwrap();
        fs::write(folder.join("Extras").join("bonus.mp4"), b"bonus bytes").unwrap();
        let mut engine = RenameEngine::new(root.path().join("data"));
        let item = engine
            .preview(vec![folder.to_string_lossy().into_owned()])
            .unwrap()
            .remove(0);
        assert_eq!(item.extension, "");
        assert_eq!(item.suggested_name, "三体 (2023)");
        assert_eq!(item.kind, "tv");
        let count = engine
            .apply(vec![RenameEdit {
                id: item.id,
                source_path: item.source_path,
                target_name: item.suggested_name,
            }])
            .unwrap()
            .count;
        assert_eq!(count, 1);
        let renamed = root.path().join("三体 (2023)");
        assert_eq!(
            fs::read(renamed.join(episode_name)).unwrap(),
            b"episode bytes"
        );
        assert_eq!(
            fs::read(renamed.join("Extras").join("bonus.mp4")).unwrap(),
            b"bonus bytes"
        );
        assert_eq!(engine.undo().unwrap().count, 1);
        assert_eq!(
            fs::read(folder.join(episode_name)).unwrap(),
            b"episode bytes"
        );
        assert_eq!(
            fs::read(folder.join("Extras").join("bonus.mp4")).unwrap(),
            b"bonus bytes"
        );
    }

    #[test]
    fn preview_auto_detects_mixed_movie_file_and_series_folder() {
        let root = tempdir().unwrap();
        let movie = root.path().join("The.Matrix.1999.mkv");
        let series = root.path().join("三体.2023");
        fs::write(&movie, b"movie").unwrap();
        fs::create_dir(&series).unwrap();
        fs::write(series.join("三体S01E01.mkv"), b"episode").unwrap();
        let mut engine = RenameEngine::new(root.path().join("data"));
        let items = engine
            .preview(vec![
                series.to_string_lossy().into_owned(),
                movie.to_string_lossy().into_owned(),
            ])
            .unwrap();
        assert_eq!(items.len(), 3);
        let movie_item = items.iter().find(|item| item.kind == "movie").unwrap();
        assert_eq!(movie_item.suggested_name, "The Matrix (1999).mkv");
        let series_item = items.iter().find(|item| item.kind == "tv").unwrap();
        assert_eq!(series_item.suggested_name, "三体 (2023)");
        let edits = items
            .into_iter()
            .map(|item| RenameEdit {
                id: item.id,
                source_path: item.source_path,
                target_name: item.suggested_name,
            })
            .collect();
        assert_eq!(engine.apply(edits).unwrap().count, 3);
        assert_eq!(
            fs::read(root.path().join("三体 (2023)").join("三体 - S01E01.mkv")).unwrap(),
            b"episode"
        );
        assert_eq!(engine.undo().unwrap().count, 3);
        assert!(movie.exists() && series.exists());
    }

    #[test]
    fn preview_apply_and_persistent_undo() {
        let root = tempdir().unwrap();
        let source = root.path().join("The.Matrix.1999.1080p.mkv");
        fs::write(&source, b"video").unwrap();
        let data = root.path().join("data");
        let mut engine = RenameEngine::new(data.clone());
        let items = engine
            .preview(vec![source.to_string_lossy().into_owned()])
            .unwrap();
        assert_eq!(items[0].suggested_name, "The Matrix (1999).mkv");
        let result = engine
            .apply(vec![RenameEdit {
                id: items[0].id.clone(),
                source_path: items[0].source_path.clone(),
                target_name: items[0].suggested_name.clone(),
            }])
            .unwrap();
        assert_eq!(result.count, 1);
        let mut restarted = RenameEngine::new(data);
        assert_eq!(restarted.history().unwrap().unwrap().count, 1);
        assert_eq!(restarted.undo().unwrap().count, 1);
        assert!(source.exists());
        assert!(restarted.history().unwrap().is_none());
    }

    #[test]
    fn rejects_collisions_and_unsafe_names_without_touching_source() {
        let root = tempdir().unwrap();
        let source = root.path().join("Film.2020.mp4");
        fs::write(&source, b"original").unwrap();
        let mut engine = RenameEngine::new(root.path().join("data"));
        let item = engine
            .preview(vec![source.to_string_lossy().into_owned()])
            .unwrap()
            .remove(0);
        let edit = |name: &str| RenameEdit {
            id: item.id.clone(),
            source_path: item.source_path.clone(),
            target_name: name.into(),
        };
        assert!(engine.apply(vec![edit("CON.mp4")]).is_err());
        assert!(engine.apply(vec![edit("COM¹.mp4")]).is_err());
        assert!(engine.apply(vec![edit("LPT³.mp4")]).is_err());
        assert!(engine.apply(vec![edit("new.mkv")]).is_err());
        fs::write(root.path().join("Existing.mp4"), b"other").unwrap();
        assert!(engine.apply(vec![edit("Existing.mp4")]).is_err());
        assert_eq!(fs::read(source).unwrap(), b"original");
    }

    #[test]
    fn preview_token_cannot_be_forged_or_used_after_source_changes() {
        let root = tempdir().unwrap();
        let source = root.path().join("Film.2020.mp4");
        fs::write(&source, b"old").unwrap();
        let mut engine = RenameEngine::new(root.path().join("data"));
        let item = engine
            .preview(vec![source.to_string_lossy().into_owned()])
            .unwrap()
            .remove(0);
        assert!(engine
            .apply(vec![RenameEdit {
                id: Uuid::new_v4().to_string(),
                source_path: item.source_path.clone(),
                target_name: item.suggested_name.clone()
            }])
            .is_err());
        fs::write(&source, b"changed bytes").unwrap();
        assert!(engine
            .apply(vec![RenameEdit {
                id: item.id,
                source_path: item.source_path,
                target_name: item.suggested_name
            }])
            .is_err());
        assert!(source.exists());
        assert!(engine.history().unwrap().is_none());
    }

    #[test]
    fn empty_nested_selection_is_noop_and_batch_targets_are_case_insensitive() {
        let root = tempdir().unwrap();
        let parent = root.path().join("Series.2020");
        let child = parent.join("Season.2020");
        fs::create_dir(&parent).unwrap();
        fs::create_dir(&child).unwrap();
        let mut engine = RenameEngine::new(root.path().join("data"));
        let items = engine
            .preview(vec![
                parent.to_string_lossy().into_owned(),
                child.to_string_lossy().into_owned(),
            ])
            .unwrap();
        let edits = items
            .into_iter()
            .map(|item| RenameEdit {
                id: item.id,
                source_path: item.source_path,
                target_name: item.suggested_name,
            })
            .collect();
        assert_eq!(engine.apply(edits).unwrap().count, 0);
        assert!(parent.exists() && child.exists());

        let a = root.path().join("Alpha.2020.mp4");
        let b = root.path().join("Beta.2020.mp4");
        fs::write(&a, b"a").unwrap();
        fs::write(&b, b"b").unwrap();
        let items = engine
            .preview(vec![
                a.to_string_lossy().into_owned(),
                b.to_string_lossy().into_owned(),
            ])
            .unwrap();
        let edits = items
            .into_iter()
            .enumerate()
            .map(|(index, item)| RenameEdit {
                id: item.id,
                source_path: item.source_path,
                target_name: if index == 0 {
                    "Same.mp4".into()
                } else {
                    "same.mp4".into()
                },
            })
            .collect();
        assert!(engine.apply(edits).is_err());
        assert_eq!(fs::read(a).unwrap(), b"a");
        assert_eq!(fs::read(b).unwrap(), b"b");
    }

    #[test]
    fn multiple_batches_remain_undoable_after_restart() {
        let root = tempdir().unwrap();
        let data = root.path().join("data");
        let mut engine = RenameEngine::new(data.clone());
        let a = root.path().join("Alpha.2020.mp4");
        let b = root.path().join("Beta.2021.mp4");
        fs::write(&a, b"a").unwrap();
        fs::write(&b, b"b").unwrap();
        for source in [&a, &b] {
            let item = engine
                .preview(vec![source.to_string_lossy().into_owned()])
                .unwrap()
                .remove(0);
            assert_eq!(
                engine
                    .apply(vec![RenameEdit {
                        id: item.id,
                        source_path: item.source_path,
                        target_name: item.suggested_name
                    }])
                    .unwrap()
                    .count,
                1
            );
        }
        let mut restarted = RenameEngine::new(data);
        assert_eq!(restarted.undo().unwrap().count, 1);
        assert!(b.exists());
        assert!(!a.exists());
        assert_eq!(restarted.undo().unwrap().count, 1);
        assert!(a.exists());
        assert!(restarted.history().unwrap().is_none());
    }

    #[test]
    fn unfinished_journal_recovers_move_and_refuses_undo_collision() {
        let root = tempdir().unwrap();
        let source = root.path().join("Original.2020.mp4");
        let target = root.path().join("Original (2020).mp4");
        fs::write(&source, b"original bytes").unwrap();
        let data = root.path().join("data");
        let engine = RenameEngine::new(data.clone());
        let journal = JournalHeader {
            version: 1,
            created_at: Utc::now().to_rfc3339(),
            entries: vec![JournalEntry {
                source: source.clone(),
                target: target.clone(),
                fingerprint: fingerprint(&source).unwrap(),
            }],
        };
        engine.create_journal(&journal).unwrap();
        // Simulate termination after the move but before the completion event.
        move_no_replace(&source, &target).unwrap();
        let mut restarted = RenameEngine::new(data);
        fs::write(&source, b"new unrelated file").unwrap();
        assert!(restarted.undo().is_err());
        assert_eq!(fs::read(&target).unwrap(), b"original bytes");
        fs::remove_file(&source).unwrap();
        assert_eq!(restarted.undo().unwrap().count, 1);
        assert_eq!(fs::read(&source).unwrap(), b"original bytes");
    }

    #[test]
    fn incomplete_old_header_is_skipped_and_truncated_event_does_not_hide_close() {
        let root = tempdir().unwrap();
        let engine = RenameEngine::new(root.path().join("data"));
        let dir = engine.journal_dir();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("0000-incomplete.jsonl"), b"{\"version\":1").unwrap();
        assert!(engine.history().unwrap().is_none());
        let journal = JournalHeader {
            version: 1,
            created_at: Utc::now().to_rfc3339(),
            entries: Vec::new(),
        };
        let path = engine.create_journal(&journal).unwrap();
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"event\":\"clo")
            .unwrap();
        append_event(&path, "{\"event\":\"closed\"}").unwrap();
        assert!(engine.history().unwrap().is_none());
    }

    #[test]
    fn standalone_episode_is_not_treated_as_movie() {
        let root = tempdir().unwrap();
        let path = root.path().join("ShowS01E02.mkv");
        fs::write(&path, b"e").unwrap();
        let mut engine = RenameEngine::new(root.path().join("data"));
        let items = engine
            .preview(vec![path.to_string_lossy().into_owned()])
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "tv");
        assert_eq!(items[0].suggested_name, "Show - S01E02.mkv");
        assert!(!items[0].is_directory);
    }

    #[test]
    #[ignore = "manual QA only; requires REELINK_QA_SOURCE and REELINK_QA_JOURNAL"]
    fn qa_selected_series_round_trip() {
        let source = PathBuf::from(std::env::var("REELINK_QA_SOURCE").expect("set QA source"));
        let journal_dir =
            PathBuf::from(std::env::var("REELINK_QA_JOURNAL").expect("set isolated QA journal"));
        let query = std::env::var("REELINK_QA_QUERY").expect("set TMDb query");
        let expected_id: u64 = std::env::var("REELINK_QA_TMDB_ID")
            .expect("set TMDb id")
            .parse()
            .unwrap();
        let matches = tauri::async_runtime::block_on(crate::tmdb::search_tmdb(query, "tv".into()))
            .expect("public TMDb search failed before rename");
        let match_json = serde_json::to_value(matches).unwrap();
        let selected = match_json
            .as_array()
            .unwrap()
            .iter()
            .find(|value| value["id"].as_u64() == Some(expected_id))
            .expect("expected TMDb result absent");
        let title = selected["title"].as_str().expect("TMDb title missing");
        let year = selected["year"].as_str().expect("TMDb year missing");
        let target_name = format!("{title} ({year})");
        assert_eq!(
            target_name,
            std::env::var("REELINK_QA_EXPECTED_TARGET").expect("set reviewed target")
        );
        let target = source.with_file_name(&target_name);
        assert!(source.is_dir(), "selected source directory absent");
        assert!(
            !target.exists(),
            "target already exists; stop without changes"
        );
        let children_before = fs::read_dir(&source)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<HashSet<_>>();
        let mut engine = RenameEngine::new(journal_dir);
        let item = engine
            .preview(vec![source.to_string_lossy().into_owned()])
            .unwrap()
            .remove(0);
        assert_eq!(item.kind, "tv");
        assert_eq!(
            engine
                .apply(vec![RenameEdit {
                    id: item.id,
                    source_path: item.source_path,
                    target_name
                }])
                .unwrap()
                .count,
            1
        );
        assert!(!source.exists() && target.is_dir());
        let children_moved = fs::read_dir(&target)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<HashSet<_>>();
        assert_eq!(
            children_before, children_moved,
            "children changed; journal remains available for manual recovery"
        );
        assert_eq!(engine.undo().unwrap().count, 1);
        assert!(source.is_dir() && !target.exists());
        let children_after = fs::read_dir(&source)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<HashSet<_>>();
        assert_eq!(children_before, children_after);
    }
}

#[cfg(test)]
#[path = "rename_nested_tests.rs"]
mod nested_tests;
