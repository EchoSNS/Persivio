//! Phase 12 acceptance: an older app finds a newer version and accepts exactly the package
//! its publisher signed for that version. Uses the real updater plugin with the app's
//! configuration, a local server and throw-away keys; only running the installer is left
//! out.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test helpers outside #[test] fns are not covered by allow-unwrap-in-tests"
)]

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use base64::Engine as _;
use minisign::KeyPair;
use persivio_desktop::updates::{UpdateView, Updates};
use tauri::test::{MockRuntime, mock_builder, mock_context, noop_assets};
use tauri_plugin_updater::UpdaterExt;

const PACKAGE: &[u8] = b"pretend installer, version 9.9.9";

type Files = Arc<Mutex<HashMap<String, Vec<u8>>>>;

struct Env {
    files: Files,
    base: String,
    app: tauri::App<MockRuntime>,
    key: KeyPair,
}

fn encode(text: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(text)
}

/// The app's own updater settings from `tauri.conf.json`, so the test covers what ships.
fn shipped_updater_config() -> serde_json::Value {
    let config: serde_json::Value =
        serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    config["plugins"]["updater"].clone()
}

impl Env {
    async fn new() -> Self {
        let files = Files::default();
        let router = Router::new()
            .route(
                "/{name}",
                get(
                    async |State(files): State<Files>, Path(name): Path<String>| {
                        let file = files.lock().unwrap().get(&name).cloned();
                        file.ok_or(StatusCode::NOT_FOUND)
                    },
                ),
            )
            .with_state(files.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await });

        let mut config = shipped_updater_config();
        assert_eq!(
            config["requireSignedVersion"], true,
            "shipped with the version bound to the signature"
        );
        // The test server is plain HTTP on this computer.
        config["dangerousInsecureTransportProtocol"] = true.into();
        let mut context = mock_context(noop_assets());
        context
            .config_mut()
            .plugins
            .0
            .insert("updater".into(), config);
        let app = mock_builder()
            .plugin(tauri_plugin_updater::Builder::new().build())
            .build(context)
            .unwrap();
        Self {
            files,
            base,
            app,
            key: KeyPair::generate_unencrypted_keypair().unwrap(),
        }
    }

    /// Publishes `package` as `announced`, with a signature by `key` over `signed` that
    /// names `signed_version`.
    fn publish(
        &self,
        announced: &str,
        key: &KeyPair,
        signed: &[u8],
        signed_version: Option<&str>,
        package: &[u8],
    ) {
        let comment = signed_version.map_or_else(
            || "timestamp:1\tfile:x.exe".to_owned(),
            |v| format!("timestamp:1\tfile:x.exe\tversion:{v}"),
        );
        let signature = minisign::sign(
            None,
            &key.sk,
            Cursor::new(signed),
            Some(&comment),
            Some("signature from tauri secret key"),
        )
        .unwrap()
        .to_string();
        let target = tauri_plugin_updater::target().unwrap();
        let mut platforms = serde_json::Map::new();
        platforms.insert(
            target,
            serde_json::json!({
                "signature": encode(&signature),
                "url": format!("{}/package", self.base),
            }),
        );
        let manifest = serde_json::json!({
            "version": announced,
            "notes": "### Fixed\n- Everything",
            "pub_date": "2026-11-24T00:00:00Z",
            "platforms": platforms,
        });
        let mut files = self.files.lock().unwrap();
        files.insert("latest.json".into(), manifest.to_string().into_bytes());
        files.insert("package".into(), package.to_vec());
    }

    async fn check(&self) -> (Updates, Result<Option<UpdateView>, String>) {
        let checker = self
            .app
            .updater_builder()
            .pubkey(encode(&self.key.pk.to_box().unwrap().to_string()))
            .endpoints(vec![format!("{}/latest.json", self.base).parse().unwrap()])
            .unwrap()
            .build()
            .unwrap();
        let updates = Updates::default();
        let found = updates.check_with(&checker).await;
        (updates, found)
    }

    /// Checks, downloads and verifies, as "Install and restart" does before installing.
    async fn download(&self) -> Result<Vec<u8>, String> {
        let (updates, found) = self.check().await;
        found.unwrap().expect("an update is offered");
        let update = updates.take().unwrap();
        update
            .download(|_, _| {}, || {})
            .await
            .map_err(|e| e.to_string())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_older_app_finds_and_verifies_a_newer_version() {
    let env = Env::new().await;
    env.publish("9.9.9", &env.key, PACKAGE, Some("9.9.9"), PACKAGE);
    let (_, found) = env.check().await;
    let found = found.unwrap().unwrap();
    assert_eq!(found.version, "9.9.9");
    assert_eq!(found.date.as_deref(), Some("2026-11-24"));
    assert_eq!(found.notes.as_deref(), Some("### Fixed\n- Everything"));
    assert_eq!(env.download().await.unwrap(), PACKAGE);
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_is_offered_without_a_newer_version() {
    let env = Env::new().await;
    let (_, missing) = env.check().await;
    assert!(
        missing.unwrap_err().contains("could not check for updates"),
        "no manifest"
    );
    env.publish("0.0.0", &env.key, PACKAGE, Some("0.0.0"), PACKAGE);
    let (updates, found) = env.check().await;
    assert_eq!(found.unwrap(), None);
    assert!(updates.take().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn packages_that_were_not_signed_for_this_version_are_refused() {
    let env = Env::new().await;
    let refused = |what: &'static str, result: Result<Vec<u8>, String>| {
        println!("{what}: {:?}", result.as_ref().unwrap_err());
        assert!(result.is_err(), "{what}");
    };
    let mut tampered = PACKAGE.to_vec();
    tampered[0] ^= 1;
    env.publish("9.9.9", &env.key, PACKAGE, Some("9.9.9"), &tampered);
    refused("one changed bit", env.download().await);

    let stranger = KeyPair::generate_unencrypted_keypair().unwrap();
    env.publish("9.9.9", &stranger, PACKAGE, Some("9.9.9"), PACKAGE);
    refused("another key", env.download().await);

    env.publish("9.9.9", &env.key, PACKAGE, Some("0.2.0"), PACKAGE);
    refused("an old package announced as new", env.download().await);

    env.publish("9.9.9", &env.key, PACKAGE, None, PACKAGE);
    refused("a signature without a version", env.download().await);

    env.publish("9.9.9", &env.key, PACKAGE, Some("9.9.9"), PACKAGE);
    assert!(env.download().await.is_ok(), "the real one still works");
}
