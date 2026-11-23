//! What the commands do, written against [`EngineApi`] so tests can use a mock engine.
//! `commands.rs` only adapts these to Tauri.

use std::path::{Path, PathBuf};

use pv_core::filename::contained_path;
use pv_core::{Source, TaskId, TaskSpec};
use url::Url;

use crate::api::EngineApi;
use crate::dto::{AddRequest, SettingsView, TaskView};
use crate::error::CommandError;

/// Parses a task id sent by the UI.
///
/// # Errors
/// [`CommandError::invalid`] for a malformed id.
pub fn parse_id(id: &str) -> Result<TaskId, CommandError> {
    id.parse()
        .map_err(|_| CommandError::invalid(format!("invalid task id `{id}`")))
}

/// Every task, in queue order.
///
/// # Errors
/// Engine errors.
pub async fn list_tasks(engine: &dyn EngineApi) -> Result<Vec<TaskView>, CommandError> {
    Ok(engine.list().await?.iter().map(TaskView::from).collect())
}

/// Parses what the user typed or pasted: a magnet link or an `http(s)` URL.
///
/// # Errors
/// [`CommandError::invalid`] for anything else.
pub fn parse_source(input: &str) -> Result<Source, CommandError> {
    let input = input.trim();
    if input
        .get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case("magnet:"))
    {
        return Source::magnet(input).map_err(|e| CommandError::invalid(e.to_string()));
    }
    let url = Url::parse(input)
        .map_err(|_| CommandError::invalid(format!("not a URL or magnet link: `{input}`")))?;
    Source::http(url).map_err(|e| CommandError::invalid(e.to_string()))
}

/// The folder new downloads go to: the settings' folder, else the OS one.
///
/// # Errors
/// Engine errors, or [`CommandError::invalid`] if neither exists.
pub async fn default_download_dir(
    engine: &dyn EngineApi,
    os_dir: Option<PathBuf>,
) -> Result<PathBuf, CommandError> {
    engine
        .settings()
        .await?
        .download_dir
        .or(os_dir)
        .ok_or_else(|| CommandError::invalid("no download folder: choose one in Settings"))
}

/// Adds a download and returns its id.
///
/// # Errors
/// Invalid source/destination/options, or engine errors.
pub async fn add_task(
    engine: &dyn EngineApi,
    request: AddRequest,
    os_dir: Option<PathBuf>,
) -> Result<String, CommandError> {
    let source = parse_source(&request.source)?;
    let dest_dir = match request.dest_dir.filter(|d| !d.trim().is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => default_download_dir(engine, os_dir).await?,
    };
    let mut spec = TaskSpec::new(source, dest_dir);
    spec.file_name = request.file_name.filter(|n| !n.trim().is_empty());
    spec.connections = request.connections;
    spec.speed_limit_bps = request.speed_limit_bps;
    spec.category = request.category.filter(|c| !c.trim().is_empty());
    // The engine validates the spec (absolute folder, connection range, ...).
    Ok(engine.add(spec).await?.to_string())
}

/// Replaces the settings (validated by the engine).
///
/// # Errors
/// [`CommandError`] with kind `invalid_input` for invalid values.
pub async fn set_settings(
    engine: &dyn EngineApi,
    settings: SettingsView,
) -> Result<(), CommandError> {
    Ok(engine.set_settings(settings.into()).await?)
}

/// The file (or folder) to show for a task: the downloaded file once it exists, else
/// the destination folder. Resolved here from the task id so the UI never sends a path.
///
/// # Errors
/// Unknown task, or engine errors.
pub async fn task_path(engine: &dyn EngineApi, id: TaskId) -> Result<PathBuf, CommandError> {
    let task = engine
        .list()
        .await?
        .into_iter()
        .find(|t| t.id == id)
        .ok_or(pv_engine::EngineError::NotFound(id))?;
    let dir = task.spec.dest_dir;
    Ok(task
        .spec
        .file_name
        .and_then(|name| contained_path(&dir, &name).ok())
        .filter(|p| p.exists())
        .unwrap_or(dir))
}

/// `path` as a string for the UI.
#[must_use]
pub fn display_path(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use pv_core::{EngineEvent, Settings, TaskSnapshot};
    use pv_engine::{EngineError, MoveDirection};
    use tokio::sync::broadcast;

    use super::*;
    use crate::error::ErrorKind;

    /// Records calls and keeps tasks in memory, like the engine would.
    #[derive(Default)]
    struct MockEngine {
        tasks: Mutex<Vec<TaskSnapshot>>,
        settings: Mutex<Settings>,
        calls: Mutex<Vec<String>>,
    }

    impl MockEngine {
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
        fn log(&self, call: impl Into<String>) {
            self.calls.lock().unwrap().push(call.into());
        }
    }

    #[async_trait]
    impl EngineApi for MockEngine {
        async fn add(&self, spec: TaskSpec) -> Result<TaskId, EngineError> {
            spec.validate()?;
            let task = TaskSnapshot::new(spec);
            let id = task.id;
            self.tasks.lock().unwrap().push(task);
            Ok(id)
        }
        async fn pause(&self, id: TaskId) -> Result<(), EngineError> {
            self.log(format!("pause {id}"));
            Ok(())
        }
        async fn resume(&self, id: TaskId) -> Result<(), EngineError> {
            Err(EngineError::NotFound(id))
        }
        async fn cancel(&self, _: TaskId) -> Result<(), EngineError> {
            Ok(())
        }
        async fn remove(&self, _: TaskId, _: bool) -> Result<(), EngineError> {
            Ok(())
        }
        async fn pause_all(&self) -> Result<(), EngineError> {
            Ok(())
        }
        async fn resume_all(&self) -> Result<(), EngineError> {
            Ok(())
        }
        async fn move_task(&self, _: TaskId, _: MoveDirection) -> Result<(), EngineError> {
            Ok(())
        }
        async fn select_files(&self, _: TaskId, _: Option<Vec<u32>>) -> Result<(), EngineError> {
            Ok(())
        }
        async fn settings(&self) -> Result<Settings, EngineError> {
            Ok(self.settings.lock().unwrap().clone())
        }
        async fn set_settings(&self, settings: Settings) -> Result<(), EngineError> {
            settings.validate()?;
            *self.settings.lock().unwrap() = settings;
            Ok(())
        }
        async fn list(&self) -> Result<Vec<TaskSnapshot>, EngineError> {
            Ok(self.tasks.lock().unwrap().clone())
        }
        fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
            broadcast::channel(1).1
        }
        async fn shutdown(&self) {}
    }

    fn request(source: &str) -> AddRequest {
        AddRequest {
            source: source.to_owned(),
            dest_dir: None,
            file_name: None,
            connections: None,
            speed_limit_bps: None,
            category: None,
        }
    }

    #[tokio::test]
    async fn add_uses_the_os_folder_then_the_settings_folder() {
        let engine = MockEngine::default();
        let os = std::env::temp_dir();
        add_task(
            &engine,
            request(" https://example.com/a.iso "),
            Some(os.clone()),
        )
        .await
        .unwrap();
        let custom = os.join("custom");
        engine.settings.lock().unwrap().download_dir = Some(custom.clone());
        add_task(
            &engine,
            request("magnet:?xt=urn:btih:abc"),
            Some(os.clone()),
        )
        .await
        .unwrap();
        let tasks = engine.list().await.unwrap();
        assert_eq!(tasks[0].spec.dest_dir, os);
        assert_eq!(tasks[1].spec.dest_dir, custom);
        assert_eq!(
            TaskView::from(&tasks[1]).kind,
            crate::dto::TaskKindView::Torrent
        );
    }

    #[tokio::test]
    async fn add_rejects_bad_input_with_clear_errors() {
        let engine = MockEngine::default();
        let os = Some(std::env::temp_dir());
        for bad in ["", "not a url", "ftp://example.com/f", "file:///etc/passwd"] {
            let err = add_task(&engine, request(bad), os.clone())
                .await
                .unwrap_err();
            assert_eq!(err.kind, ErrorKind::InvalidInput, "{bad}: {err:?}");
        }
        let mut relative = request("https://example.com/f");
        relative.dest_dir = Some("relative/dir".into());
        let err = add_task(&engine, relative, os.clone()).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidInput);
        let err = add_task(&engine, request("https://example.com/f"), None)
            .await
            .unwrap_err();
        assert!(err.message.contains("Settings"), "{err:?}");
        assert_eq!(engine.list().await.unwrap(), []);
    }

    #[tokio::test]
    async fn engine_errors_keep_their_category() {
        let engine = MockEngine::default();
        let id = TaskId::new();
        let err = CommandError::from(engine.resume(id).await.unwrap_err());
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert!(parse_id("nope").is_err());
        assert_eq!(parse_id(&id.to_string()).unwrap(), id);
        engine.pause(id).await.unwrap();
        assert_eq!(engine.calls(), [format!("pause {id}")]);
    }

    #[tokio::test]
    async fn invalid_settings_are_rejected() {
        let engine = MockEngine::default();
        let mut view = SettingsView::from(&Settings::default());
        view.max_concurrent_tasks = 0;
        let err = set_settings(&engine, view).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidInput);
    }

    #[tokio::test]
    async fn task_path_is_the_file_once_it_exists() {
        let engine = MockEngine::default();
        let dir = tempfile::tempdir().unwrap();
        let mut req = request("https://example.com/a.bin");
        req.dest_dir = Some(dir.path().display().to_string());
        let id = parse_id(&add_task(&engine, req, None).await.unwrap()).unwrap();
        assert_eq!(task_path(&engine, id).await.unwrap(), dir.path());

        engine.tasks.lock().unwrap()[0].spec.file_name = Some("a.bin".into());
        std::fs::write(dir.path().join("a.bin"), b"x").unwrap();
        assert_eq!(
            task_path(&engine, id).await.unwrap(),
            dir.path().join("a.bin")
        );

        // A name that escapes the folder falls back to the folder.
        engine.tasks.lock().unwrap()[0].spec.file_name = Some("../a.bin".into());
        assert_eq!(task_path(&engine, id).await.unwrap(), dir.path());

        let err = task_path(&engine, TaskId::new()).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
    }
}
