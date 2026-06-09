use crate::models::knowledge::{ChangeType, FileChangeEvent};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct FileWatcherService {
    watcher: Option<RecommendedWatcher>,
    watched_dirs: HashSet<PathBuf>,
    wiki_dir: PathBuf,
    debounce_ms: u64,
    startup_grace_ms: u64,
    internal_write_tracker: Arc<Mutex<Vec<(PathBuf, Instant)>>>,
}

impl FileWatcherService {
    pub fn new(wiki_dir: PathBuf) -> Self {
        Self {
            watcher: None,
            watched_dirs: HashSet::new(),
            wiki_dir,
            debounce_ms: 500,
            startup_grace_ms: 5000,
            internal_write_tracker: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn start(
        &mut self,
        tx: tokio::sync::mpsc::Sender<FileChangeEvent>,
    ) -> Result<(), String> {
        if !self.wiki_dir.exists() {
            std::fs::create_dir_all(&self.wiki_dir)
                .map_err(|e| format!("无法创建 wiki 目录: {e}"))?;
        }

        let tracker = self.internal_write_tracker.clone();
        let _wiki_dir = self.wiki_dir.clone();
        let debounce_ms = self.debounce_ms;
        let startup_grace_ms = self.startup_grace_ms;
        let start_time = Instant::now();

        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    if start_time.elapsed().as_millis() < startup_grace_ms as u128 {
                        return;
                    }

                    let path = match event.paths.first() {
                        Some(p) => p.clone(),
                        None => return,
                    };

                    let path_str = path.to_string_lossy().replace('\\', "/");

                    // No extension filtering — all file events pass through.
                    // Filtering happens in the event consumer (lib.rs).

                    let change_type = match event.kind {
                        EventKind::Create(_) => ChangeType::Created,
                        EventKind::Modify(_) => ChangeType::Modified,
                        EventKind::Remove(_) => ChangeType::Removed,
                        _ => return,
                    };

                    // Send absolute path; consumer normalises as needed
                    let event = FileChangeEvent {
                        file_path: path_str,
                        change_type,
                        namespace: None,
                    };

                    let tx = tx.clone();
                    let tracker = tracker.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(debounce_ms));
                        let t = tracker.lock().unwrap();
                        if t.iter().any(|(p, _)| p == &path) {
                            return;
                        }
                        drop(t);
                        let _ = tx.blocking_send(event);
                    });
                }
            },
            Config::default(),
        )
        .map_err(|e| format!("无法创建文件监视器: {e}"))?;

        watcher
            .watch(&self.wiki_dir, RecursiveMode::Recursive)
            .map_err(|e| format!("无法监视目录: {e}"))?;

        self.watched_dirs.insert(self.wiki_dir.clone());
        self.watcher = Some(watcher);
        log::info!(
            "FileWatcher started on {}",
            self.wiki_dir.display()
        );
        Ok(())
    }

    /// Mark a file path as written by AI-Hel2 internally (skip for 60s)
    pub fn track_internal_write(&self, path: &str) {
        let mut tracker = self.internal_write_tracker.lock().unwrap();
        tracker.push((PathBuf::from(path), Instant::now()));
        tracker.retain(|(_, t)| t.elapsed().as_secs() < 60);
    }
}

impl Drop for FileWatcherService {
    fn drop(&mut self) {
        log::info!("FileWatcher stopped");
    }
}
