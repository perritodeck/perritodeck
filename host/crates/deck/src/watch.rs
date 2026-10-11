//! Reload the profile when its file changes.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use notify::{RecursiveMode, Watcher};
use tracing::{error, info};

use crate::Deck;

/// Watch `path` and reload `deck` whenever it's saved. Bad edits are reported and the
/// last good profile stays on the tablets. Keep the returned watcher alive.
pub fn watch_profile(path: PathBuf, deck: Arc<Deck>) -> notify::Result<notify::RecommendedWatcher> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let file_name = path.file_name().map(|n| n.to_owned());
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            // Reading the file (as reload does) also produces events; only react to writes.
            if ev.kind.is_access() {
                return;
            }
            // Editors often save by writing a temp file and renaming it over the
            // original, so watch the folder and match by name.
            if ev
                .paths
                .iter()
                .any(|p| p.file_name() == file_name.as_deref())
            {
                let _ = tx.send(());
            }
        }
    })?;
    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    watcher.watch(&dir, RecursiveMode::NonRecursive)?;

    tokio::spawn(async move {
        while rx.recv().await.is_some() {
            // Let a burst of save events settle, then reload once.
            tokio::time::sleep(Duration::from_millis(250)).await;
            while rx.try_recv().is_ok() {}
            match perritodeck_profile::load(&path) {
                // The desktop app reloads the deck itself right after saving; don't
                // redraw every tablet a second time for the same content.
                Ok(profile) if profile == deck.profile() && deck.reload_error().is_none() => {}
                Ok(profile) => {
                    info!("{} changed", path.display());
                    deck.reload(profile);
                }
                Err(e) => {
                    error!("not applying your edit — {e}");
                    deck.set_reload_error(e.to_string());
                }
            }
        }
    });
    Ok(watcher)
}
