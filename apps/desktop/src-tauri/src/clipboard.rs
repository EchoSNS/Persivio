//! Opt-in clipboard watching (ADR-020): when a newly copied text contains http(s) or
//! magnet links, the UI offers to add them. Nothing is added without a click.
//!
//! Operating systems have no portable "clipboard changed" event, so the text is read once
//! a second, and only while the setting is on. When it is off the task waits on the
//! settings channel and costs nothing. The text is never logged or stored.

use std::time::Duration;

use pv_core::DesktopSettings;
use tokio::sync::watch;

use crate::external::{Arg, classify_args};

/// How often the clipboard is read while watching.
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Longer texts are ignored (not a copied link).
const MAX_TEXT: usize = 64 * 1024;
/// At most this many links per offer.
const MAX_LINKS: usize = 20;

/// The http(s) and magnet links in `text` (unique, in order).
#[must_use]
pub fn links_in(text: &str) -> Vec<String> {
    if text.len() > MAX_TEXT {
        return Vec::new();
    }
    let mut links = Vec::new();
    for arg in classify_args(text.split_whitespace()) {
        if let Arg::Link(link) = arg
            && !links.contains(&link)
        {
            links.push(link);
            if links.len() == MAX_LINKS {
                break;
            }
        }
    }
    links
}

/// Watches the clipboard while `settings.watch_clipboard` is on, calling `offer` with
/// the links of each newly copied text. Text already on the clipboard when watching
/// starts is not offered. Ends when the settings sender is dropped.
pub async fn watch_clipboard(
    read: impl Fn() -> Option<String>,
    mut settings: watch::Receiver<DesktopSettings>,
    offer: impl Fn(Vec<String>),
) {
    loop {
        if !settings.borrow_and_update().watch_clipboard {
            if settings.changed().await.is_err() {
                return;
            }
            continue;
        }
        let mut last = read();
        let mut tick = tokio::time::interval(POLL_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                changed = settings.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    if !settings.borrow().watch_clipboard {
                        break;
                    }
                }
                _ = tick.tick() => {
                    let text = read();
                    if text != last {
                        if let Some(links) = text.as_deref().map(links_in).filter(|l| !l.is_empty()) {
                            offer(links);
                        }
                        last = text;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn only_http_and_magnet_links_are_offered() {
        assert_eq!(
            links_in(
                "see https://a.example/f.iso and magnet:?xt=urn:btih:abc\nhttps://a.example/f.iso ftp://x/y"
            ),
            ["https://a.example/f.iso", "magnet:?xt=urn:btih:abc"]
        );
        assert_eq!(links_in("just some text"), Vec::<String>::new());
        assert_eq!(
            links_in(&format!("https://a.example/ {}", "x".repeat(MAX_TEXT))),
            Vec::<String>::new()
        );
        let many = (0..30)
            .map(|i| format!("https://a.example/{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(links_in(&many).len(), MAX_LINKS);
    }

    #[tokio::test(start_paused = true)]
    async fn offers_new_links_only_while_enabled() {
        let clipboard = Arc::new(Mutex::new(Some("https://old.example/".to_owned())));
        let offers = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = watch::channel(DesktopSettings::default());
        let task = tokio::spawn({
            let clipboard = Arc::clone(&clipboard);
            let offers = Arc::clone(&offers);
            watch_clipboard(
                move || clipboard.lock().unwrap().clone(),
                rx,
                move |links| offers.lock().unwrap().push(links),
            )
        });
        let copy = |text: &str| *clipboard.lock().unwrap() = Some(text.to_owned());
        let wait = || tokio::time::sleep(POLL_INTERVAL * 2);

        copy("https://ignored.example/"); // watching is off
        wait().await;
        tx.send_modify(|s| s.watch_clipboard = true);
        wait().await; // the text present at start is not offered
        copy("hello");
        wait().await;
        copy("magnet:?xt=urn:btih:abc");
        wait().await;
        tx.send_modify(|s| s.watch_clipboard = false);
        wait().await;
        copy("https://after.example/");
        wait().await;
        drop(tx);
        task.await.unwrap();

        assert_eq!(
            *offers.lock().unwrap(),
            [vec!["magnet:?xt=urn:btih:abc".to_owned()]]
        );
    }
}
