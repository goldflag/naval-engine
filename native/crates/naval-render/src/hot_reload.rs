//! Shader hot reload (native only). Always compiled, so every build shares one `naval-render`; the client's and
//! harness's `hot-reload` features decide whether anything polls it.
//!
//! Polls the modification time of every registered WGSL file that has a disk
//! path, at most every quarter second. Polling needs no watcher thread or extra
//! dependency, sees editors that save by renaming a new file into place, and
//! costs a few `stat` calls a second for a few dozen files.

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

use crate::shaders::ShaderLibrary;

const INTERVAL: Duration = Duration::from_millis(250);

#[derive(Default)]
pub struct ShaderWatcher {
    /// Library path → the modification time last seen.
    seen: HashMap<String, SystemTime>,
    last_poll: Option<Instant>,
}

impl ShaderWatcher {
    /// Library paths and new text of files changed since the last poll. On a
    /// file's first sighting its disk text is compared with the library's, so a
    /// fresh renderer (a scene reload) picks up edits made since the build
    /// instead of reverting to the embedded source.
    pub fn poll(&mut self, library: &ShaderLibrary) -> Vec<(String, String)> {
        let now = Instant::now();
        if self.last_poll.is_some_and(|last| now - last < INTERVAL) {
            return Vec::new();
        }
        self.last_poll = Some(now);
        let mut changes = Vec::new();
        for (path, disk) in library.disk_files() {
            let Ok(modified) = std::fs::metadata(disk).and_then(|meta| meta.modified()) else {
                continue;
            };
            // Updated in place: a file seen before costs no allocation per poll.
            let first_sighting = match self.seen.get_mut(path) {
                None => {
                    self.seen.insert(path.to_owned(), modified);
                    true
                }
                Some(previous) if *previous != modified => {
                    *previous = modified;
                    false
                }
                Some(_) => continue,
            };
            let Ok(text) = std::fs::read_to_string(disk) else {
                continue;
            };
            if !first_sighting || library.text(path) != Some(text.as_str()) {
                changes.push((path.to_owned(), text));
            }
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ShaderFile;

    fn scratch_file(name: &str, text: &str) -> &'static str {
        let dir =
            std::env::temp_dir().join(format!("naval-render-hot-reload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        Box::leak(path.to_string_lossy().into_owned().into_boxed_str())
    }

    const EMBEDDED: &str =
        "@fragment fn fragment() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }\n";

    #[test]
    fn a_first_sighting_picks_up_a_file_edited_since_the_build() {
        let disk = scratch_file(
            "edited.wgsl",
            "@fragment fn fragment() -> @location(0) vec4<f32> { return vec4<f32>(0.5); }\n",
        );
        let mut library = ShaderLibrary::new();
        library
            .add(ShaderFile {
                path: "test/edited.wgsl",
                source: EMBEDDED,
                disk: Some(disk),
            })
            .unwrap();
        let changes = ShaderWatcher::default().poll(&library);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].0, "test/edited.wgsl");
    }

    #[test]
    fn a_first_sighting_of_an_unchanged_file_is_quiet() {
        let disk = scratch_file("same.wgsl", EMBEDDED);
        let mut library = ShaderLibrary::new();
        library
            .add(ShaderFile {
                path: "test/same.wgsl",
                source: EMBEDDED,
                disk: Some(disk),
            })
            .unwrap();
        let changes = ShaderWatcher::default().poll(&library);
        assert!(
            changes.iter().all(|(path, _)| path != "test/same.wgsl"),
            "{changes:?}"
        );
    }
}
