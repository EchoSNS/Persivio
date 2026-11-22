//! `persivio`: headless client that runs the download engine in-process.
//!
//! The engine lives as long as the command. `add`, `resume` and `watch` keep running
//! until the queue is empty (Ctrl+C stops gracefully; tasks resume next time). The other
//! commands inspect or edit the queue without downloading anything.

mod settings_edit;
mod watch;

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use indicatif::HumanBytes;
use pv_core::{Source, TaskId, TaskSnapshot, TaskSpec};
use pv_engine::{Engine, EngineConfig, EngineHandle};
use url::Url;

/// Command-line interface definition.
#[derive(Debug, Parser)]
#[command(
    name = "persivio",
    version,
    about = "Persivio download manager (headless client)"
)]
struct Cli {
    /// Where the database and lock file live (default: the OS app-data directory).
    #[arg(long, global = true, env = "PERSIVIO_DATA_DIR")]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Add downloads and run them (unless --no-wait).
    Add {
        /// http(s) URLs, magnet links, or paths to .torrent files.
        #[arg(required = true)]
        urls: Vec<String>,
        /// Destination directory (default: settings, then your Downloads folder).
        #[arg(short, long)]
        dir: Option<PathBuf>,
        /// File name (only with a single URL).
        #[arg(short = 'o', long)]
        name: Option<String>,
        /// Parallel connections per download (1-32).
        #[arg(short, long)]
        connections: Option<u8>,
        /// Speed limit for these downloads, in bytes per second.
        #[arg(long)]
        limit: Option<u64>,
        /// Only queue the downloads; run them later with `watch`.
        #[arg(long)]
        no_wait: bool,
    },
    /// List all downloads.
    #[command(alias = "ls")]
    List,
    /// Pause a download (or `all`).
    Pause { id: String },
    /// Resume a paused or failed download (or `all`) and run the queue.
    Resume {
        id: String,
        /// Only re-queue; don't start downloading now.
        #[arg(long)]
        no_wait: bool,
    },
    /// Cancel a download for good (deletes its partial file).
    Cancel { id: String },
    /// Remove a download from the list.
    #[command(alias = "remove")]
    Rm {
        id: String,
        /// Also delete the downloaded file.
        #[arg(long)]
        delete_files: bool,
    },
    /// Run queued downloads with live progress until the queue is empty.
    Watch,
    /// Show or change settings.
    Settings {
        #[command(subcommand)]
        action: Option<SettingsAction>,
    },
}

#[derive(Debug, Subcommand)]
enum SettingsAction {
    /// Print all settings as JSON.
    Show,
    /// Set one value, e.g. `set max_concurrent_tasks 5`, `set global_speed_limit_bps none`,
    /// `set torrent.dht false`.
    Set { key: String, value: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("PERSIVIO_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    let data_dir = match cli.data_dir {
        Some(d) => d,
        None => directories::ProjectDirs::from("", "", "Persivio")
            .context("cannot determine the app data directory; pass --data-dir")?
            .data_dir()
            .to_path_buf(),
    };
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("cannot create {}", data_dir.display()))?;
    let _lock = lock_data_dir(&data_dir)?;
    let db = data_dir.join("persivio.db");

    let runs = matches!(
        cli.command,
        Command::Add { no_wait: false, .. }
            | Command::Resume { no_wait: false, .. }
            | Command::Watch
    );
    let mut config = EngineConfig::new(db);
    config.autostart = runs;
    let engine = Engine::start(config)
        .await
        .context("cannot start the engine")?;
    let result = run(&engine, cli.command).await;
    engine.shutdown().await;
    result
}

/// Only one Persivio process may use a data directory: two engines on one database
/// would both "own" the same downloads.
fn lock_data_dir(dir: &Path) -> Result<File> {
    let file = File::create(dir.join("persivio.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => {
            bail!("another Persivio instance is using {}", dir.display())
        }
        Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
    }
}

async fn run(engine: &EngineHandle, command: Command) -> Result<()> {
    match command {
        Command::Add {
            urls,
            dir,
            name,
            connections,
            limit,
            no_wait,
        } => {
            if name.is_some() && urls.len() > 1 {
                bail!("--name can only be used with a single URL");
            }
            let dest = match dir {
                Some(d) => absolute(&d)?,
                None => default_download_dir(engine).await?,
            };
            for raw in &urls {
                let source = parse_source(raw)?;
                let mut spec = TaskSpec::new(source, &dest);
                spec.file_name.clone_from(&name);
                spec.connections = connections;
                spec.speed_limit_bps = limit;
                let id = engine.add(spec).await?;
                println!("added {} {raw}", short(id));
            }
            if !no_wait {
                watch::watch(engine).await?;
            }
        }
        Command::List => print_list(&engine.list().await?),
        Command::Pause { id } => {
            if id == "all" {
                engine.pause_all().await?;
            } else {
                engine.pause(resolve(engine, &id).await?).await?;
            }
            println!("paused");
        }
        Command::Resume { id, no_wait } => {
            if id == "all" {
                engine.resume_all().await?;
            } else {
                engine.resume(resolve(engine, &id).await?).await?;
            }
            if !no_wait {
                watch::watch(engine).await?;
            }
        }
        Command::Cancel { id } => {
            engine.cancel(resolve(engine, &id).await?).await?;
            println!("cancelled");
        }
        Command::Rm { id, delete_files } => {
            engine
                .remove(resolve(engine, &id).await?, delete_files)
                .await?;
            println!("removed");
        }
        Command::Watch => watch::watch(engine).await?,
        Command::Settings { action } => match action.unwrap_or(SettingsAction::Show) {
            SettingsAction::Show => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&engine.settings().await?)?
                );
            }
            SettingsAction::Set { key, value } => {
                let updated = settings_edit::set(&engine.settings().await?, &key, &value)?;
                engine.set_settings(updated).await?;
                println!("{key} updated");
            }
        },
    }
    Ok(())
}

fn absolute(path: &Path) -> Result<PathBuf> {
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    })
}

async fn default_download_dir(engine: &EngineHandle) -> Result<PathBuf> {
    if let Some(dir) = engine.settings().await?.download_dir {
        return absolute(&dir);
    }
    let dirs = directories::UserDirs::new();
    match dirs.as_ref().and_then(directories::UserDirs::download_dir) {
        Some(d) => Ok(d.to_path_buf()),
        None => absolute(Path::new(".")),
    }
}

/// First 8 hex digits of the id's random part (UUID v7 ids share their time prefix).
fn short(id: TaskId) -> String {
    id.to_string().replace('-', "")[24..].to_owned()
}

/// Accepts a full id or a unique fragment of at least 4 characters.
async fn resolve(engine: &EngineHandle, query: &str) -> Result<TaskId> {
    let ids: Vec<TaskId> = engine.list().await?.iter().map(|t| t.id).collect();
    resolve_in(&ids, query)
}

fn resolve_in(ids: &[TaskId], query: &str) -> Result<TaskId> {
    if let Ok(id) = query.parse::<TaskId>() {
        return Ok(id);
    }
    let q = query.to_ascii_lowercase().replace('-', "");
    if q.len() < 4 {
        bail!("id `{query}` is too short (use at least 4 characters, see `persivio list`)");
    }
    let matches: Vec<&TaskId> = ids
        .iter()
        .filter(|id| id.to_string().replace('-', "").contains(&q))
        .collect();
    match matches.as_slice() {
        [one] => Ok(**one),
        [] => bail!("no download matches `{query}`"),
        _ => bail!(
            "`{query}` matches {} downloads; use more characters",
            matches.len()
        ),
    }
}

fn print_list(tasks: &[TaskSnapshot]) {
    if tasks.is_empty() {
        println!("No downloads.");
        return;
    }
    println!("{:<10} {:<12} {:>21}  NAME", "ID", "STATE", "PROGRESS");
    for t in tasks {
        let progress = match t.total_size {
            Some(total) => format!("{} / {}", HumanBytes(t.downloaded), HumanBytes(total)),
            None => HumanBytes(t.downloaded).to_string(),
        };
        let name = t
            .spec
            .file_name
            .clone()
            .unwrap_or_else(|| source_label(&t.spec.source));
        println!(
            "{:<10} {:<12} {progress:>21}  {name}",
            short(t.id),
            t.state.kind()
        );
        if let pv_core::TaskState::Failed { reason, .. } = &t.state {
            println!("{:<10} └─ {reason}", "");
        }
    }
}

fn source_label(source: &Source) -> String {
    match source {
        Source::Http { url, .. } => url.to_string(),
        Source::Magnet { .. } => "magnet link".to_owned(),
        Source::TorrentFile { .. } => "torrent file".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// Catches clap definition mistakes (duplicate flags, bad defaults) at test time
    /// instead of at runtime.
    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn ids_resolve_by_unique_fragment() {
        let ids: Vec<TaskId> = (0..3).map(|_| TaskId::new()).collect();
        assert_eq!(resolve_in(&ids, &ids[1].to_string()).unwrap(), ids[1]);
        assert_eq!(resolve_in(&ids, &short(ids[2])).unwrap(), ids[2]);
        assert!(
            resolve_in(&ids, "ab")
                .unwrap_err()
                .to_string()
                .contains("too short")
        );
        assert!(
            resolve_in(&ids, "zzzzzz")
                .unwrap_err()
                .to_string()
                .contains("no download")
        );
        // UUID v7 ids created together share a time prefix, so a prefix is ambiguous.
        let prefix = &ids[0].to_string()[..8];
        assert!(
            resolve_in(&ids, prefix)
                .unwrap_err()
                .to_string()
                .contains("matches 3")
        );
    }
}

/// Turns a command-line argument into a download source: a magnet link, a path to an
/// existing `.torrent` file, or an http(s) URL.
fn parse_source(raw: &str) -> Result<Source> {
    if raw
        .get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case("magnet:"))
    {
        return Ok(Source::magnet(raw)?);
    }
    let path = Path::new(raw);
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("torrent"))
        && path.is_file()
    {
        let bytes = std::fs::read(path).with_context(|| format!("cannot read {raw}"))?;
        let source = Source::TorrentFile { bytes };
        source.validate()?;
        return Ok(source);
    }
    let url = Url::parse(raw)
        .with_context(|| format!("`{raw}` is not a URL, magnet link or .torrent file"))?;
    Ok(Source::http(url)?)
}

#[cfg(test)]
mod source_tests {
    use super::*;

    #[test]
    fn arguments_become_the_right_source() {
        assert!(matches!(
            parse_source("https://e.x/a.iso").unwrap(),
            Source::Http { .. }
        ));
        assert!(matches!(
            parse_source("MAGNET:?xt=urn:btih:abc").unwrap(),
            Source::Magnet { .. }
        ));
        let dir = tempfile::tempdir().unwrap();
        let t = dir.path().join("x.Torrent");
        std::fs::write(&t, b"d4:infodee").unwrap();
        assert!(matches!(
            parse_source(t.to_str().unwrap()).unwrap(),
            Source::TorrentFile { .. }
        ));
        let err = parse_source("not a thing").unwrap_err().to_string();
        assert!(
            err.contains("not a URL, magnet link or .torrent file"),
            "{err}"
        );
        assert!(parse_source("ftp://e.x/f").is_err(), "only http(s) URLs");
    }
}
