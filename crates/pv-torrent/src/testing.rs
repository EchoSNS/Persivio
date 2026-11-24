//! Test support (feature `testing`): a local seeding session, so crates other than
//! `pv-torrent` can test torrents without depending on librqbit (ADR-006).

use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use librqbit::{CreateTorrentOptions, ListenerOptions, Session, SessionOptions};

/// A librqbit session seeding one file or folder on 127.0.0.1, with DHT off.
pub struct LocalSeeder {
    session: Arc<Session>,
    /// Address peers can connect to.
    pub addr: SocketAddr,
    /// The bencoded `.torrent` bytes.
    pub torrent: Vec<u8>,
    /// A `magnet:?xt=urn:btih:…` link for the torrent.
    pub magnet: String,
}

impl LocalSeeder {
    /// Creates a torrent for `path` (file or folder) named `name` and starts seeding it.
    ///
    /// # Panics
    /// On any setup failure: this is test-only code.
    #[allow(
        clippy::unwrap_used,
        clippy::missing_panics_doc,
        reason = "test support"
    )]
    pub async fn start(path: &Path, name: &str) -> Self {
        Self::start_on(path, name, 0).await
    }

    /// [`start`](Self::start), listening on a given port.
    ///
    /// # Panics
    /// On any setup failure: this is test-only code.
    #[allow(
        clippy::unwrap_used,
        clippy::missing_panics_doc,
        reason = "test support"
    )]
    pub async fn start_on(path: &Path, name: &str, port: u16) -> Self {
        let opts = SessionOptions {
            dht: None,
            persistence: None,
            disable_local_service_discovery: true,
            listen: Some(ListenerOptions {
                listen_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
                ..ListenerOptions::default()
            }),
            ..SessionOptions::default()
        };
        let session = Session::new_with_opts(path.parent().unwrap().to_path_buf(), opts)
            .await
            .unwrap();
        let (created, handle) = session
            .create_and_serve_torrent(
                path,
                CreateTorrentOptions {
                    name: Some(name),
                    piece_length: Some(16 * 1024),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        handle.wait_until_initialized().await.unwrap();
        Self {
            addr: session.listen_addr().unwrap(),
            torrent: created.as_bytes().unwrap().to_vec(),
            magnet: format!("magnet:?xt=urn:btih:{}", created.info_hash().as_string()),
            session,
        }
    }

    /// Stops seeding.
    pub async fn stop(&self) {
        self.session.stop().await;
    }
}
