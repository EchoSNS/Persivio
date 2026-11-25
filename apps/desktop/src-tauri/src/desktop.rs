//! Operating-system integration (Phase 9, ADR-020): tray icon, window show/hide,
//! notifications, autostart, the `magnet:` handler and external add requests.

use std::sync::{Mutex, PoisonError};

use pv_core::DesktopSettings;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_notification::NotificationExt;
use tauri_specta::Event as _;
use tokio::sync::watch;

use crate::AppState;
use crate::dto::{BrowserOffersEvent, ClipboardLinksEvent, ExternalRequestsEvent};
use crate::external::ExternalAddView;
use crate::integration::Sink;
use crate::sleep::SleepInhibitor;

/// Tray icon id.
pub const TRAY_ID: &str = "main";
/// Autostart passes this flag: start hidden in the tray.
pub const MINIMIZED_ARG: &str = "--minimized";

/// Desktop state: the integration settings (watched by background tasks) and external
/// requests waiting for the UI.
#[derive(Debug)]
pub struct Desktop {
    /// Current desktop settings; background tasks react to changes.
    pub settings: watch::Sender<DesktopSettings>,
    /// Browser bridge state, written by its supervisor.
    pub bridge: std::sync::Arc<Mutex<crate::bridge::BridgeStatus>>,
    pending: Mutex<Vec<ExternalAddView>>,
    /// How the window was before a question brought it up.
    before_question: Mutex<Option<Away>>,
}

/// A window that is not on the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Away {
    Hidden,
    Minimized,
}

impl Desktop {
    /// Starts with `settings`.
    #[must_use]
    pub fn new(settings: DesktopSettings) -> Self {
        Self {
            settings: watch::Sender::new(settings),
            bridge: std::sync::Arc::default(),
            pending: Mutex::new(Vec::new()),
            before_question: Mutex::new(None),
        }
    }

    /// Current bridge status.
    pub fn bridge_status(&self) -> crate::bridge::BridgeStatus {
        self.bridge
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Takes the queued external requests (the UI calls this when signalled and on load).
    pub fn take_pending(&self) -> Vec<ExternalAddView> {
        std::mem::take(&mut *self.pending.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn push(&self, view: ExternalAddView) {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(view);
    }
}

/// Shows, restores and focuses the main window.
pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Queues an external request, brings the window forward and tells the UI.
pub fn handle_external<R: Runtime>(app: &AppHandle<R>, view: ExternalAddView) {
    if view.is_empty() {
        return;
    }
    if let Some(desktop) = app.try_state::<Desktop>() {
        desktop.push(view);
    }
    show_main_window(app);
    if let Err(e) = ExternalRequestsEvent.emit(app) {
        tracing::warn!("cannot signal external request: {e}");
    }
}

/// Brings the window up to ask about a download from the browser.
pub fn ask_about_offers<R: Runtime>(app: &AppHandle<R>) {
    if let (Some(window), Some(desktop)) =
        (app.get_webview_window("main"), app.try_state::<Desktop>())
    {
        let away = if !window.is_visible().unwrap_or(true) {
            Some(Away::Hidden)
        } else if window.is_minimized().unwrap_or(false) {
            Some(Away::Minimized)
        } else {
            None
        };
        let mut before = desktop
            .before_question
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if away.is_some() {
            *before = away;
        }
    }
    show_main_window(app);
    if let Err(e) = BrowserOffersEvent.emit(app) {
        tracing::warn!("cannot signal a browser download: {e}");
    }
}

/// After the last answer: the window goes back to where it was, so the user is back in
/// the browser.
pub fn offers_answered<R: Runtime>(app: &AppHandle<R>) {
    let (Some(window), Some(desktop)) =
        (app.get_webview_window("main"), app.try_state::<Desktop>())
    else {
        return;
    };
    let before = desktop
        .before_question
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    let _ = match before {
        Some(Away::Hidden) => window.hide(),
        Some(Away::Minimized) => window.minimize(),
        None => Ok(()),
    };
}

/// Offers clipboard links to the UI.
pub fn offer_clipboard_links<R: Runtime>(app: &AppHandle<R>, links: Vec<String>) {
    if let Err(e) = (ClipboardLinksEvent { links }).emit(app) {
        tracing::warn!("cannot offer clipboard links: {e}");
    }
}

/// Applies the settings that live in the OS (autostart entry, `magnet:` handler). Only
/// changes what differs, and returns problems to show.
pub fn apply_os_settings<R: Runtime>(app: &AppHandle<R>, s: &DesktopSettings) -> Vec<String> {
    let mut problems = Vec::new();
    {
        use tauri_plugin_autostart::ManagerExt as _;
        let autolaunch = app.autolaunch();
        let result = match autolaunch.is_enabled() {
            Ok(on) if on == s.autostart => Ok(()),
            _ if s.autostart => autolaunch.enable(),
            _ => autolaunch.disable(),
        };
        if let Err(e) = result {
            problems.push(format!("Could not change \"start with the system\": {e}"));
        }
    }
    #[cfg(any(windows, target_os = "linux"))]
    {
        use tauri_plugin_deep_link::DeepLinkExt as _;
        let deep_link = app.deep_link();
        // `unregister` deletes the whole `magnet` key, whoever owns it: only touch it when
        // it points to this Persivio, so another torrent client's handler is never broken.
        let ours = deep_link.is_registered("magnet").unwrap_or(false);
        let result = if s.handle_magnet_links && !ours {
            deep_link.register("magnet")
        } else if !s.handle_magnet_links && ours {
            deep_link.unregister("magnet")
        } else {
            Ok(())
        };
        if let Err(e) = result {
            problems.push(format!("Could not change the magnet link handler: {e}"));
        }
    }
    problems
}

/// Builds the tray icon: tooltip with speeds, menu with Show / Pause all / Resume all /
/// Quit; a left click shows the window.
///
/// # Errors
/// If the platform has no tray or the menu cannot be built.
pub fn build_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Persivio", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause_all", "Pause all", true, None::<&str>)?;
    let resume = MenuItem::with_id(app, "resume_all", "Resume all", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &show,
            &PredefinedMenuItem::separator(app)?,
            &pause,
            &resume,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Persivio")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
            "pause_all" | "resume_all" => {
                let pause = event.id().as_ref() == "pause_all";
                let engine = app.state::<AppState>().running_engine();
                tauri::async_runtime::spawn(async move {
                    if let Some(engine) = engine {
                        let result = if pause {
                            engine.pause_all().await
                        } else {
                            engine.resume_all().await
                        };
                        if let Err(e) = result {
                            tracing::warn!("tray action failed: {e}");
                        }
                    }
                });
            }
            // Goes through ExitRequested: the engine shuts down gracefully.
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Applies integration effects to the real tray, notifications and sleep guard.
pub struct AppSink<R: Runtime> {
    app: AppHandle<R>,
    sleep: Option<SleepInhibitor>,
}

impl<R: Runtime> AppSink<R> {
    /// A sink for `app`; sleep prevention is skipped if its thread cannot start.
    pub fn new(app: AppHandle<R>) -> Self {
        let sleep = SleepInhibitor::start()
            .inspect_err(|e| tracing::warn!("sleep prevention unavailable: {e}"))
            .ok();
        Self { app, sleep }
    }
}

impl<R: Runtime> Sink for AppSink<R> {
    fn notify(&self, title: &str, body: &str) {
        // Only when the user is not looking at the window already.
        let in_front = self
            .app
            .get_webview_window("main")
            .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false));
        if in_front {
            return;
        }
        if let Err(e) = self
            .app
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show()
        {
            tracing::warn!("cannot show notification: {e}");
        }
    }

    fn tooltip(&self, text: &str) {
        if let Some(tray) = self.app.tray_by_id(TRAY_ID) {
            let _ = tray.set_tooltip(Some(text));
        }
    }

    fn keep_awake(&self, on: bool) {
        if let Some(sleep) = &self.sleep {
            sleep.set(on);
        }
    }
}
