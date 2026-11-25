//! Downloads the browser handed over, waiting for the user's answer (ADR-028).
//!
//! An offer holds everything the download needs, cookies included, so those never leave
//! Rust: the UI gets an [`OfferView`] and answers with the offer's id.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use pv_core::filename::sanitize_filename;
use pv_core::{Source, TaskSpec};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::api::EngineApi;
use crate::dto::{display_source, guess_name};
use crate::error::CommandError;

/// More than this is a page starting downloads by itself, not a person.
pub const MAX_PENDING: usize = 20;
/// Longest media type kept.
const MAX_MIME: usize = 127;

/// What the confirmation shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OfferView {
    /// Answered with this.
    pub id: String,
    /// The file's name, safe to show and to save as.
    pub file_name: String,
    /// The link, without a user name or password in it.
    pub url: String,
    /// Host the file comes from.
    pub host: String,
    /// Host of the page the download started on, if it is another one.
    pub page_host: Option<String>,
    /// Bytes, if the browser knows.
    pub size: Option<u64>,
    /// Media type, if the browser knows.
    pub mime: Option<String>,
    /// Whether the site's cookies go along (you are logged in there).
    pub sends_credentials: bool,
    /// Folder it would be saved to.
    pub dest_dir: String,
}

/// The user's answer "download it".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AcceptOffer {
    /// Another folder.
    pub dest_dir: Option<String>,
    /// Another file name.
    pub file_name: Option<String>,
    /// Category label.
    pub category: Option<String>,
}

#[derive(Clone)]
struct Offer {
    id: u64,
    spec: TaskSpec,
    size: Option<u64>,
    mime: Option<String>,
}

impl Offer {
    fn view(&self) -> OfferView {
        let (url, referer, cookies) = match &self.spec.source {
            Source::Http {
                url,
                referer,
                cookies,
                ..
            } => (Some(url), referer.as_ref(), cookies.is_some()),
            _ => (None, None, false),
        };
        let host_of = |u: &url::Url| u.host_str().unwrap_or_default().to_owned();
        let host = url.map_or_else(String::new, host_of);
        OfferView {
            id: self.id.to_string(),
            file_name: self
                .spec
                .file_name
                .clone()
                .unwrap_or_else(|| sanitize_filename(&guess_name(&self.spec.source))),
            url: display_source(&self.spec.source),
            page_host: referer.map(host_of).filter(|h| !h.is_empty() && *h != host),
            host,
            size: self.size,
            mime: self.mime.clone(),
            sends_credentials: cookies,
            dest_dir: crate::ops::display_path(&self.spec.dest_dir),
        }
    }
}

/// A media type as plain `type/subtype`, or nothing.
fn clean_mime(mime: &str) -> Option<String> {
    let mime = mime.split(';').next().unwrap_or_default().trim();
    let ok = mime.len() <= MAX_MIME
        && mime.split('/').count() == 2
        && mime
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/.+-_".contains(&b));
    ok.then(|| mime.to_ascii_lowercase())
}

/// The offers waiting for an answer.
pub struct Offers {
    pending: Mutex<(u64, Vec<Offer>)>,
    /// Called when one arrived.
    arrived: Box<dyn Fn() + Send + Sync>,
}

impl Offers {
    /// No offers; `arrived` runs after each new one.
    pub fn new(arrived: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            pending: Mutex::new((0, Vec::new())),
            arrived: Box::new(arrived),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, (u64, Vec<Offer>)> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Adds an offer and returns its id, or `None` if too many wait. The same link again
    /// (an impatient second click) is the same offer.
    pub fn push(&self, spec: TaskSpec, size: Option<u64>, mime: Option<&str>) -> Option<String> {
        let id = {
            let mut guard = self.lock();
            let (next, pending) = &mut *guard;
            if let Some(same) = pending.iter_mut().find(|o| o.spec.source == spec.source) {
                same.id
            } else if pending.len() >= MAX_PENDING {
                return None;
            } else {
                *next += 1;
                pending.push(Offer {
                    id: *next,
                    spec,
                    size: size.filter(|s| *s > 0),
                    mime: mime.and_then(clean_mime),
                });
                *next
            }
        };
        (self.arrived)();
        Some(id.to_string())
    }

    /// What waits, oldest first.
    #[must_use]
    pub fn list(&self) -> Vec<OfferView> {
        self.lock().1.iter().map(Offer::view).collect()
    }

    fn find(&self, id: &str) -> Option<Offer> {
        self.lock()
            .1
            .iter()
            .find(|o| o.id.to_string() == id)
            .cloned()
    }

    /// Forgets an offer (answered "no", or added).
    pub fn remove(&self, id: &str) {
        self.lock().1.retain(|o| o.id.to_string() != id);
    }

    /// Adds the offered download. The offer stays if that fails, so the answer can be
    /// corrected (a folder that cannot be used).
    ///
    /// # Errors
    /// An unknown offer, or what the engine refuses.
    pub async fn accept(
        &self,
        engine: &dyn EngineApi,
        id: &str,
        answer: AcceptOffer,
    ) -> Result<String, CommandError> {
        let offer = self
            .find(id)
            .ok_or_else(|| CommandError::invalid("this download was already answered"))?;
        let mut spec = offer.spec;
        let set = |v: Option<String>| v.filter(|v| !v.trim().is_empty());
        if let Some(dir) = set(answer.dest_dir) {
            spec.dest_dir = PathBuf::from(dir.trim());
        }
        if let Some(name) = set(answer.file_name) {
            spec.file_name = Some(sanitize_filename(name.trim()));
        }
        spec.category = set(answer.category);
        let task = engine.add(spec).await?;
        self.remove(id);
        Ok(task.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use url::Url;

    use super::*;

    fn spec(url: &str) -> TaskSpec {
        TaskSpec::new(
            Source::Http {
                url: Url::parse(url).unwrap(),
                headers: Vec::new(),
                cookies: Some("session=secret".into()),
                referer: Some(Url::parse("https://shop.example/page?token=1").unwrap()),
            },
            std::env::temp_dir(),
        )
    }

    #[test]
    fn an_offer_shows_what_the_user_needs_and_no_secrets() {
        let offers = Offers::new(|| {});
        offers
            .push(
                spec("https://user:pw@cdn.example/a/My%20File.zip?sig=1"),
                Some(1234),
                Some("Application/ZIP; charset=x"),
            )
            .unwrap();
        let view = &offers.list()[0];
        assert_eq!(view.file_name, "My File.zip");
        assert_eq!(view.host, "cdn.example");
        assert_eq!(view.page_host.as_deref(), Some("shop.example"));
        assert_eq!(view.size, Some(1234));
        assert_eq!(view.mime.as_deref(), Some("application/zip"));
        assert!(view.sends_credentials);
        let json = serde_json::to_string(view).unwrap();
        for secret in ["session", "secret", "user", "pw@", "token"] {
            assert!(!json.contains(secret), "{secret} in {json}");
        }
    }

    #[test]
    fn untrusted_details_are_cleaned() {
        let offers = Offers::new(|| {});
        for (name, mime) in [
            ("evil%E2%80%AEfdp.exe", "a/b/c"),
            ("..%2F..%2Fx", "text/<b>"),
            ("a", ""),
        ] {
            offers.push(
                spec(&format!("https://a.example/{name}")),
                Some(0),
                Some(mime),
            );
        }
        for view in offers.list() {
            assert_eq!(view.mime, None);
            assert_eq!(view.size, None, "zero means unknown");
            assert!(
                !view.file_name.contains(['\u{202E}', '/', '\\']),
                "{}",
                view.file_name
            );
        }
        assert_eq!(clean_mime(&format!("a/{}", "b".repeat(200))), None);
    }

    #[test]
    fn the_same_link_is_one_offer_and_floods_are_refused() {
        let told = Arc::new(AtomicUsize::new(0));
        let t = Arc::clone(&told);
        let offers = Offers::new(move || {
            t.fetch_add(1, Ordering::SeqCst);
        });
        let a = offers.push(spec("https://a.example/0"), None, None);
        assert_eq!(offers.push(spec("https://a.example/0"), None, None), a);
        assert_eq!(offers.list().len(), 1);
        assert_eq!(told.load(Ordering::SeqCst), 2, "the window comes up again");
        for i in 1..MAX_PENDING {
            assert!(
                offers
                    .push(spec(&format!("https://a.example/{i}")), None, None)
                    .is_some()
            );
        }
        assert_eq!(
            offers.push(spec("https://a.example/more"), None, None),
            None
        );
        offers.remove(&a.unwrap());
        assert!(
            offers
                .push(spec("https://a.example/more"), None, None)
                .is_some()
        );
    }
}
