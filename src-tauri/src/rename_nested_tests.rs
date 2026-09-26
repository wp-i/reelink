use super::*;
use tempfile::tempdir;

fn edits(items: Vec<RenameItem>) -> Vec<RenameEdit> {
    items
        .into_iter()
        .map(|item| RenameEdit {
            id: item.id,
            source_path: item.source_path,
            target_name: item.suggested_name,
        })
        .collect()
}
fn preview(engine: &mut RenameEngine, folder: &Path) -> Vec<RenameItem> {
    engine
        .preview(vec![folder.to_string_lossy().into_owned()])
        .unwrap()
}

#[test]
fn episode_ranges_do_not_confuse_resolution_with_an_episode() {
    assert_eq!(
        episode_code("Show.S01E01E02.1080p", None).as_deref(),
        Some("S01E01-E02")
    );
    assert_eq!(episode_code("Show.S01E01-1080p", None), None);
}

#[test]
fn movie_folder_preserves_container_subtitles_and_structure() {
    let root = tempdir().unwrap();
    let folder = root.path().join("Downloads");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("Film.2020.1080p.mkv"), b"movie").unwrap();
    fs::write(folder.join("Film.2020.srt"), b"subtitle").unwrap();
    let mut engine = RenameEngine::new(root.path().join("data"));
    let items = preview(&mut engine, &folder);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].kind, "movie");
    assert_eq!(engine.apply(edits(items)).unwrap().count, 1);
    assert_eq!(fs::read(folder.join("Film (2020).mkv")).unwrap(), b"movie");
    assert_eq!(fs::read(folder.join("Film.2020.srt")).unwrap(), b"subtitle");
    assert_eq!(engine.undo().unwrap().count, 1);
}

#[test]
fn nested_series_renames_episodes_then_folder_and_restores_after_restart() {
    let root = tempdir().unwrap();
    let folder = root.path().join("Show.2020.1080p");
    let season = folder.join("Season 02");
    fs::create_dir_all(&season).unwrap();
    fs::write(season.join("Show.S02E01-E02.WEB.mkv"), b"two episodes").unwrap();
    fs::write(season.join("EP03.mp4"), b"third episode").unwrap();
    fs::write(folder.join("bonus.mkv"), b"unknown episode").unwrap();
    let data = root.path().join("data");
    let mut engine = RenameEngine::new(data.clone());
    let items = engine
        .preview(vec![
            folder.to_string_lossy().into_owned(),
            season.to_string_lossy().into_owned(),
        ])
        .unwrap();
    assert_eq!(items.len(), 4);
    assert!(items.iter().all(|item| item.group_id == items[0].group_id));
    assert_eq!(
        items
            .iter()
            .find(|i| i.original_name == "bonus.mkv")
            .unwrap()
            .suggested_name,
        "bonus.mkv"
    );
    assert_eq!(engine.apply(edits(items)).unwrap().count, 3);
    let renamed = root.path().join("Show (2020)");
    assert_eq!(
        fs::read(renamed.join("Season 02/Show - S02E01-E02.mkv")).unwrap(),
        b"two episodes"
    );
    assert_eq!(
        fs::read(renamed.join("Season 02/Show - S02E03.mp4")).unwrap(),
        b"third episode"
    );
    let mut restarted = RenameEngine::new(data);
    assert_eq!(restarted.history().unwrap().unwrap().count, 3);
    assert_eq!(restarted.undo().unwrap().count, 3);
    assert_eq!(
        fs::read(season.join("Show.S02E01-E02.WEB.mkv")).unwrap(),
        b"two episodes"
    );
    assert_eq!(fs::read(season.join("EP03.mp4")).unwrap(), b"third episode");
    assert!(folder.join("bonus.mkv").exists());
}

#[test]
fn selected_season_does_not_rename_ancestor_outside_selection() {
    let root = tempdir().unwrap();
    let parent = root.path().join("Show.2021");
    let folder = parent.join("Season 02");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("EP01.mkv"), b"episode").unwrap();
    let mut engine = RenameEngine::new(root.path().join("data"));
    let items = preview(&mut engine, &folder);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].suggested_name, "Show - S02E01.mkv");
    assert_eq!(engine.apply(edits(items)).unwrap().count, 1);
    assert!(parent.is_dir() && folder.is_dir());
    assert_eq!(engine.undo().unwrap().count, 1);
}

#[test]
fn nested_collision_preflight_does_not_mutate_anything() {
    let root = tempdir().unwrap();
    let folder = root.path().join("Show.2020");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("ShowS01E01.mkv"), b"one").unwrap();
    fs::write(folder.join("Show.S01E01.1080p.mkv"), b"other edition").unwrap();
    let mut engine = RenameEngine::new(root.path().join("data"));
    let items = preview(&mut engine, &folder);
    assert!(engine.apply(edits(items)).is_err());
    assert_eq!(fs::read(folder.join("ShowS01E01.mkv")).unwrap(), b"one");
    assert!(engine.history().unwrap().is_none());
}

#[test]
fn blocked_parent_undo_never_changes_unrelated_recreated_directory() {
    let root = tempdir().unwrap();
    let folder = root.path().join("Show.2020");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("ShowS01E01.mkv"), b"episode").unwrap();
    let data = root.path().join("data");
    let mut engine = RenameEngine::new(data.clone());
    let items = preview(&mut engine, &folder);
    assert_eq!(engine.apply(edits(items)).unwrap().count, 2);
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("unrelated.txt"), b"unrelated").unwrap();
    let mut restarted = RenameEngine::new(data);
    assert!(restarted.undo().is_err());
    assert_eq!(
        fs::read(folder.join("unrelated.txt")).unwrap(),
        b"unrelated"
    );
    assert_eq!(
        fs::read(root.path().join("Show (2020)/Show - S01E01.mkv")).unwrap(),
        b"episode"
    );
    fs::remove_file(folder.join("unrelated.txt")).unwrap();
    fs::remove_dir(&folder).unwrap();
    assert_eq!(restarted.undo().unwrap().count, 2);
    assert_eq!(fs::read(folder.join("ShowS01E01.mkv")).unwrap(), b"episode");
}

#[test]
fn interrupted_child_move_before_parent_is_recoverable_without_event() {
    let root = tempdir().unwrap();
    let folder = root.path().join("Show.2020");
    fs::create_dir(&folder).unwrap();
    let child = folder.join("ShowS01E01.mkv");
    let target_child = folder.join("Show - S01E01.mkv");
    fs::write(&child, b"episode").unwrap();
    let data = root.path().join("data");
    let engine = RenameEngine::new(data.clone());
    engine
        .create_journal(&JournalHeader {
            version: 1,
            created_at: Utc::now().to_rfc3339(),
            entries: vec![
                JournalEntry {
                    source: child.clone(),
                    target: target_child.clone(),
                    fingerprint: fingerprint(&child).unwrap(),
                },
                JournalEntry {
                    source: folder.clone(),
                    target: root.path().join("Show (2020)"),
                    fingerprint: fingerprint(&folder).unwrap(),
                },
            ],
        })
        .unwrap();
    move_no_replace(&child, &target_child).unwrap();
    let mut restarted = RenameEngine::new(data);
    assert_eq!(restarted.history().unwrap().unwrap().count, 1);
    assert_eq!(restarted.undo().unwrap().count, 1);
    assert_eq!(fs::read(child).unwrap(), b"episode");
}
