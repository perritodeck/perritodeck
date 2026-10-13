//! Reload profiles when their files change.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use notify::{RecursiveMode, Watcher};
use tracing::{error, info};

use crate::Deck;

/// The profile id for a file in the profiles folder: `streaming.toml` → `streaming`.
pub fn profile_id(path: &Path) -> Option<String> {
    (path.extension()? == "toml" && !path.file_name()?.to_string_lossy().starts_with('.'))
        .then(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .flatten()
}

/// Load every `*.toml` profile in `dir` into the deck (besides the default, which is
/// already loaded). Files that don't load are reported and skipped.
pub fn load_all(dir: &Path, deck: &Deck) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let default = deck.default_profile_id();
    for path in entries.flatten().map(|e| e.path()) {
        let Some(id) = profile_id(&path) else {
            continue;
        };
        if id == default {
            continue;
        }
        match perritodeck_profile::load(&path) {
            Ok(p) => deck.load_profile(&id, p, Some(dir.to_owned())),
            Err(e) => error!("skipping profile {id}: {e}"),
        }
    }
}

/// Watch the profiles folder and keep the deck in step: edited files reload, new files
/// are added, deleted files are removed. Bad edits are reported and the last good
/// version stays on the tablets. Keep the returned watcher alive.
pub fn watch_profiles(dir: PathBuf, deck: Arc<Deck>) -> notify::Result<notify::RecommendedWatcher> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<PathBuf>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            // Reading a file (as reload does) also produces events; only react to writes.
            if ev.kind.is_access() {
                return;
            }
            // Editors often save by writing a temp file and renaming it over the
            // original, so watch the folder and match by name.
            for p in ev.paths {
                if profile_id(&p).is_some() {
                    let _ = tx.send(p);
                }
            }
        }
    })?;
    watcher.watch(&dir, RecursiveMode::NonRecursive)?;

    tokio::spawn(async move {
        while let Some(first) = rx.recv().await {
            // Let a burst of save events settle, then handle each file once.
            tokio::time::sleep(Duration::from_millis(250)).await;
            let mut changed: HashSet<PathBuf> = HashSet::from([first]);
            while let Ok(p) = rx.try_recv() {
                changed.insert(p);
            }
            for path in changed {
                let Some(id) = profile_id(&path) else {
                    continue;
                };
                if !path.exists() {
                    if deck.remove_profile(&id) {
                        info!("profile {id} removed");
                    }
                    continue;
                }
                match perritodeck_profile::load(&path) {
                    // The desktop app reloads the deck itself right after saving; don't
                    // redraw every tablet a second time for the same content.
                    Ok(profile)
                        if deck.is_current(&id, &profile)
                            && deck.reload_error_of(&id).is_none() => {}
                    Ok(profile) => {
                        info!("{} changed", path.display());
                        deck.load_profile(&id, profile, Some(dir.clone()));
                    }
                    Err(e) => {
                        error!("not applying your edit — {e}");
                        deck.set_reload_error_of(&id, e.to_string());
                    }
                }
            }
        }
    });
    Ok(watcher)
}
