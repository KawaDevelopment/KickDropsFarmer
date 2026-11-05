mod discord;
mod kick;
mod notify;
mod settings;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kick::api::UserStatus;
use kick::endpoints;
use kick::models::FarmState;
use kick::watcher::Engine;
use settings::Settings;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Manager, RunEvent, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Window,
    WindowEvent, Wry,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tokio::sync::Mutex;

pub(crate) const TRAY_ID: &str = "main-tray";
const MAIN: &str = "main";
const MINIMIZED_ARG: &str = "--minimized";
const MIN_SIZE: f64 = 520.0;

struct AppState {
    settings: Arc<Mutex<Settings>>,
    engine: Arc<Engine>,
    pause_item: MenuItem<Wry>,
    close_to_tray: AtomicBool,
    tray_notice_shown: AtomicBool,
}

fn pause_label(paused: bool) -> &'static str {
    if paused {
        "Resume farming"
    } else {
        "Pause farming"
    }
}

fn open_url(url: &str) -> Result<(), String> {
    std::process::Command::new("explorer")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open the browser: {e}"))
}

// ---- window ----

fn open_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let (width, height) = app
        .try_state::<AppState>()
        .and_then(|s| s.settings.try_lock().ok().map(|g| (g.window_width, g.window_height)))
        .unwrap_or((1100.0, 720.0));
    let built = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::default())
        .title("KickDropFarmer")
        .inner_size(width.clamp(MIN_SIZE, 4000.0), height.clamp(MIN_SIZE, 3000.0))
        .min_inner_size(MIN_SIZE, MIN_SIZE)
        .resizable(true)
        .center()
        .visible(false)
        .build();
    if built.is_ok() {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(3)).await;
            if let Some(w) = handle.get_webview_window(MAIN) {
                if !w.is_visible().unwrap_or(true) {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        });
    }
}

fn remember_window(window: &Window) {
    let app = window.app_handle().clone();
    let maximized = window.is_maximized().unwrap_or(false);
    let size = window
        .inner_size()
        .ok()
        .zip(window.scale_factor().ok())
        .map(|(s, f)| (s.width as f64 / f, s.height as f64 / f))
        .filter(|(w, h)| *w >= MIN_SIZE && *h >= MIN_SIZE);
    let apply = move |s: &mut Settings| {
        s.window_maximized = maximized;
        if let (false, Some((w, h))) = (maximized, size) {
            s.window_width = w.round();
            s.window_height = h.round();
        }
        s.clone()
    };
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    if let Ok(mut guard) = state.settings.try_lock() {
        let snapshot = apply(&mut guard);
        drop(guard);
        let _ = settings::save(&app, &snapshot);
        return;
    }
    let settings_arc = state.settings.clone();
    tauri::async_runtime::spawn(async move {
        let snapshot = apply(&mut *settings_arc.lock().await);
        let _ = settings::save(&app, &snapshot);
    });
}

async fn apply_paused(app: &AppHandle, paused: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut s = state.settings.lock().await;
        s.paused = paused;
        s.clone()
    };
    let _ = state.pause_item.set_text(pause_label(paused));
    state.engine.wake();
    settings::save(app, &snapshot)
}

// ---- commands ----

#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    Ok(state.settings.lock().await.clone())
}

#[tauri::command]
async fn save_settings(app: AppHandle, state: State<'_, AppState>, new_settings: Settings) -> Result<(), String> {
    let (snapshot, refresh, wake) = {
        let mut s = state.settings.lock().await;
        let mut next = new_settings;
        next.kick_token = s.kick_token.clone();
        next.selected_games = s.selected_games.clone();
        next.paused = s.paused;
        next.window_width = s.window_width;
        next.window_height = s.window_height;
        next.window_maximized = s.window_maximized;
        if next.autostart != s.autostart {
            let mgr = app.autolaunch();
            let result = if next.autostart { mgr.enable() } else { mgr.disable() };
            result.map_err(|e| format!("could not change start on boot: {e}"))?;
        }
        let refresh = next.demo_mode != s.demo_mode;
        let wake = next.scheduler_enabled != s.scheduler_enabled
            || next.schedule_start != s.schedule_start
            || next.schedule_end != s.schedule_end
            || next.auto_claim != s.auto_claim;
        *s = next.clone();
        (next, refresh, wake)
    };
    state.close_to_tray.store(snapshot.close_to_tray, Ordering::Relaxed);
    if refresh {
        state.engine.request_refresh();
    } else if wake {
        state.engine.wake();
    }
    settings::save(&app, &snapshot)
}

#[tauri::command]
async fn get_state(state: State<'_, AppState>) -> Result<FarmState, String> {
    Ok(state.engine.state.lock().await.clone())
}

#[tauri::command]
async fn set_selected_games(app: AppHandle, state: State<'_, AppState>, games: Vec<String>) -> Result<(), String> {
    let snapshot = {
        let mut s = state.settings.lock().await;
        s.selected_games = games;
        s.clone()
    };
    state.engine.request_refresh();
    settings::save(&app, &snapshot)
}

#[tauri::command]
async fn set_paused(app: AppHandle, paused: bool) -> Result<(), String> {
    apply_paused(&app, paused).await
}

#[tauri::command]
async fn set_kick_token(app: AppHandle, state: State<'_, AppState>, token: String) -> Result<String, String> {
    let trimmed = token.trim().to_string();
    let engine = &state.engine;
    engine.proxy.set_manual_token((!trimmed.is_empty()).then(|| trimmed.clone()));

    let snapshot = {
        let mut s = state.settings.lock().await;
        s.kick_token = trimmed.clone();
        if !trimmed.is_empty() {
            s.demo_mode = false;
        }
        s.clone()
    };
    settings::save(&app, &snapshot)?;

    if trimmed.is_empty() {
        {
            let mut st = engine.state.lock().await;
            st.logged_in = false;
            st.username = None;
            st.push_log("Kick cookie cleared.");
        }
        engine.request_refresh();
        return Ok("cleared".into());
    }

    let full_header = trimmed.contains("kick_session");
    let status = match kick::api::fetch_user(&engine.proxy).await {
        UserStatus::LoggedIn(None) => UserStatus::LoggedIn(kick::api::fetch_username(&engine.proxy).await),
        other => other,
    };
    let result = {
        let mut st = engine.state.lock().await;
        match status {
            UserStatus::LoggedIn(name) => {
                st.logged_in = true;
                st.username = name.clone();
                let who = name.unwrap_or_else(|| "your account".into());
                st.push_log(format!("Connected to Kick as {who}."));
                if full_header {
                    "connected"
                } else {
                    "partial"
                }
            }
            UserStatus::LoggedOut => {
                st.logged_in = false;
                st.push_log("Kick rejected that cookie. Copy the full cookie header again from a logged-in kick.com tab.");
                "rejected"
            }
            UserStatus::Unknown => {
                st.push_log("Saved the cookie, but Kick could not be reached to verify it.");
                "unverified"
            }
        }
    };
    engine.request_refresh();
    Ok(result.into())
}

#[tauri::command]
async fn claim_drop(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.engine.claim_manual(&id).await
}

#[tauri::command]
fn open_channel(slug: String) -> Result<(), String> {
    let url = endpoints::channel_page(&slug).ok_or("invalid channel name")?;
    open_url(&url)
}

#[tauri::command]
fn open_kick() -> Result<(), String> {
    open_url(endpoints::SITE)
}

#[tauri::command]
async fn test_discord(state: State<'_, AppState>, url: String) -> Result<(), String> {
    discord::send_test(&state.engine.proxy, &url).await
}

#[tauri::command]
async fn run_diagnostics(state: State<'_, AppState>) -> Result<Vec<kick::diag::EndpointProbe>, String> {
    Ok(kick::diag::run(&state.engine.proxy).await)
}

#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[tauri::command]
async fn ui_ready(window: WebviewWindow, state: State<'_, AppState>) -> Result<(), String> {
    if !window.is_visible().unwrap_or(false) {
        if state.settings.lock().await.window_maximized {
            let _ = window.maximize();
        }
        let _ = window.show();
        let _ = window.set_focus();
    }
    state.engine.publish().await;
    Ok(())
}

// ---- tray ----

fn build_tray(app: &AppHandle, paused: bool) -> tauri::Result<MenuItem<Wry>> {
    let show = MenuItem::with_id(app, "show", "Open KickDropFarmer", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", pause_label(paused), true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &pause, &separator, &quit])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("KickDropFarmer")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => open_main_window(app),
            "pause" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let paused = app.state::<AppState>().settings.lock().await.paused;
                    let _ = apply_paused(&app, !paused).await;
                });
            }
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
                open_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(pause)
}

// ---- entry ----

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| open_main_window(app)))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![MINIMIZED_ARG]),
        ))
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            get_state,
            set_selected_games,
            set_paused,
            set_kick_token,
            claim_drop,
            open_channel,
            open_kick,
            test_discord,
            run_diagnostics,
            app_version,
            ui_ready,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let loaded = settings::load(&handle);
            let settings = Arc::new(Mutex::new(loaded.clone()));
            let engine = Arc::new(Engine::new(handle.clone(), settings.clone()));
            if !loaded.kick_token.trim().is_empty() {
                engine.proxy.set_manual_token(Some(loaded.kick_token.clone()));
            }
            if loaded.autostart {
                let _ = handle.autolaunch().enable();
            }
            let pause_item = build_tray(&handle, loaded.paused)?;
            app.manage(AppState {
                settings,
                engine: engine.clone(),
                pause_item,
                close_to_tray: AtomicBool::new(loaded.close_to_tray),
                tray_notice_shown: AtomicBool::new(false),
            });
            engine.spawn();

            let minimized = loaded.start_minimized && std::env::args().any(|a| a == MINIMIZED_ARG);
            if !minimized {
                open_main_window(&handle);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != MAIN {
                return;
            }
            match event {
                WindowEvent::CloseRequested { .. } => {
                    remember_window(window);
                    let app = window.app_handle();
                    if let Some(state) = app.try_state::<AppState>() {
                        let to_tray = state.close_to_tray.load(Ordering::Relaxed);
                        if to_tray && !state.tray_notice_shown.swap(true, Ordering::Relaxed) {
                            if let Ok(s) = state.settings.try_lock() {
                                notify::still_running(app, &s);
                            }
                        }
                    }
                }
                WindowEvent::Focused(true) => {
                    if let Some(state) = window.app_handle().try_state::<AppState>() {
                        let engine = state.engine.clone();
                        tauri::async_runtime::spawn(async move { engine.publish().await });
                    }
                }
                _ => {}
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building KickDropFarmer");

    app.run(|app, event| {
        if let RunEvent::ExitRequested { code: None, api, .. } = event {
            let keep = app
                .try_state::<AppState>()
                .is_some_and(|s| s.close_to_tray.load(Ordering::Relaxed));
            if keep {
                api.prevent_exit();
            }
        }
    });
}
