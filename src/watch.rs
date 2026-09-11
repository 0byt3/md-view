//! Debounced single-file watching plus scroll-anchor retention.
//!
//! The parent directory is watched (not the file itself) so atomic saves
//! that replace the file keep reporting. `notify-debouncer-mini` reports no
//! remove/create distinction, so deletion is detected by checking existence
//! when a debounced event arrives.

use notify_debouncer_mini::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, Debouncer};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// Coalesced change for the watched file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchEvent {
    /// Content may have changed; reload and re-render.
    Modified,
    /// The file is gone; show the missing banner until it reappears.
    Gone,
}

/// Live watch. Dropping stops delivery, so keep it alive with the viewer.
pub struct FileWatch {
    _debouncer: Debouncer<RecommendedWatcher>,
}

/// Watch `path`'s parent for changes to `path`, coalesced by `debounce`.
/// Never panics; setup failures are returned for the caller to report.
pub fn watch_file(
    path: &Path,
    debounce: Duration,
    sink: Sender<WatchEvent>,
) -> Result<FileWatch, notify_debouncer_mini::notify::Error> {
    let target = absolute(path);
    let root = target
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"));
    // Canonicalize the root so event paths compare equal even when the
    // caller passed `.` or `..` segments. The file itself is compared by
    // name so deletion still matches.
    let root = root.canonicalize().unwrap_or(root);
    let name = target.file_name().map(|name| name.to_os_string());
    let match_root = root.clone();

    let mut debouncer = new_debouncer(debounce, move |result: DebounceEventResult| {
        if let Ok(events) = result {
            let hit = events.iter().any(|event| match &name {
                Some(name) => {
                    event.path.file_name() == Some(name.as_os_str())
                        && event.path.parent() == Some(match_root.as_path())
                }
                None => event.path == target,
            });
            if hit {
                let _ = sink.send(if target.exists() {
                    WatchEvent::Modified
                } else {
                    WatchEvent::Gone
                });
            }
        }
    })?;
    debouncer
        .watcher()
        .watch(&root, RecursiveMode::NonRecursive)?;
    Ok(FileWatch {
        _debouncer: debouncer,
    })
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("md-view-watch-{name}-{}.md", std::process::id()))
    }

    fn drain(rx: &std::sync::mpsc::Receiver<WatchEvent>, wait: Duration) -> Vec<WatchEvent> {
        let start = std::time::Instant::now();
        let mut out = Vec::new();
        loop {
            let elapsed = start.elapsed();
            if elapsed >= wait {
                break;
            }
            match rx.recv_timeout(wait - elapsed) {
                Ok(event) => out.push(event),
                Err(_) => break,
            }
        }
        out
    }

    fn write(path: &Path, body: &str) {
        let mut file = std::fs::File::create(path).unwrap();
        writeln!(file, "{body}").unwrap();
    }

    #[test]
    fn modify_delivers_single_modified() {
        let path = scratch("modify");
        write(&path, "one");
        let (tx, rx) = std::sync::mpsc::channel();
        let _watch = watch_file(&path, Duration::from_millis(50), tx).unwrap();
        // Let the watcher settle so setup noise is not attributed below.
        drain(&rx, Duration::from_millis(300));
        write(&path, "two");
        let events = drain(&rx, Duration::from_secs(5));
        assert_eq!(events, vec![WatchEvent::Modified]);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn delete_then_recreate() {
        let path = scratch("gone");
        write(&path, "here");
        let (tx, rx) = std::sync::mpsc::channel();
        let _watch = watch_file(&path, Duration::from_millis(50), tx).unwrap();
        drain(&rx, Duration::from_millis(300));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(drain(&rx, Duration::from_secs(5)), vec![WatchEvent::Gone]);
        write(&path, "back");
        assert_eq!(
            drain(&rx, Duration::from_secs(5)),
            vec![WatchEvent::Modified]
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn sibling_changes_are_ignored() {
        let dir = std::env::temp_dir().join(format!("md-view-watch-dir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("watched.md");
        let sibling = dir.join("other.md");
        write(&path, "watched");
        write(&sibling, "sibling");
        let (tx, rx) = std::sync::mpsc::channel();
        let _watch = watch_file(&path, Duration::from_millis(50), tx).unwrap();
        drain(&rx, Duration::from_millis(300));
        write(&sibling, "changed");
        assert!(drain(&rx, Duration::from_millis(500)).is_empty());
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_file(&sibling).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
}
