use crate::{
    app_state::AppState,
    config::{persistence, schema::AppConfig},
    display_driver, renderer,
    sensors::poller,
    superwidgets, ui, windows_startup,
};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, Request, Response, StatusCode},
    middleware::{self, Next},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{net::SocketAddr, sync::atomic::Ordering, time::Duration};
use tokio::{sync::oneshot, task::JoinHandle};

const INDEX: &str = include_str!("../frontend/index.html");
const STYLES: &str = include_str!("../frontend/styles.css");
const I18N: &str = include_str!("../frontend/i18n.js");
const APP: &str = include_str!("../frontend/app-v2.js");
const BRIDGE: &str = include_str!("../frontend/remote-bridge.js");

#[derive(Serialize)]
struct ApiResponse {
    ok: bool,
    result: Value,
    error: Option<String>,
}

#[derive(Deserialize)]
struct RemoteTextUpdate {
    text: String,
}

pub async fn manage(state: AppState) {
    let mut server: Option<(oneshot::Sender<()>, JoinHandle<()>)> = None;
    loop {
        let enabled = state.config.read().remote.enabled;
        match (enabled, server.is_some()) {
            (true, false) => {
                let (shutdown_tx, shutdown_rx) = oneshot::channel();
                let server_state = state.clone();
                let handle = tokio::spawn(async move {
                    if let Err(error) = serve(server_state, shutdown_rx).await {
                        tracing::error!(%error, "Remote Deck server stopped");
                    }
                });
                server = Some((shutdown_tx, handle));
            }
            (false, true) => {
                let (shutdown, handle) = server.take().expect("server handle exists");
                let _ = shutdown.send(());
                let _ = handle.await;
            }
            _ => {}
        }
        if server
            .as_ref()
            .is_some_and(|(_, handle)| handle.is_finished())
        {
            if let Some((_, handle)) = server.take() {
                let _ = handle.await;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn serve(state: AppState, shutdown: oneshot::Receiver<()>) -> anyhow::Result<()> {
    let running = state.remote_running.clone();
    let app = router(state);
    let address = SocketAddr::from(([0, 0, 0, 0], 8787));
    let listener = tokio::net::TcpListener::bind(address).await?;
    running.store(true, Ordering::Relaxed);
    tracing::info!(%address, "TelemetryForge Remote Deck listening");
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = shutdown.await;
        })
        .await;
    running.store(false, Ordering::Relaxed);
    result?;
    Ok(())
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/styles.css", get(styles))
        .route("/i18n.js", get(i18n))
        .route("/remote-bridge.js", get(bridge))
        .route("/app-v2.js", get(app_script))
        .route("/api/invoke/{command}", post(invoke))
        .route(
            "/api/widget-text/{key}",
            post(update_widget_text).layer(axum::extract::DefaultBodyLimit::max(8192)),
        )
        .route("/api/health", get(health))
        .route_layer(middleware::from_fn_with_state(state.clone(), authorize))
        .with_state(state)
}

async fn authorize(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
    let remote = state.config.read().remote.clone();
    let authorization = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    if let Some(token) = authorization.strip_prefix("Bearer ") {
        let text_only = request.method() == axum::http::Method::POST
            && request.uri().path().starts_with("/api/widget-text/");
        if text_only
            && crate::remote_auth::verify_request(token.to_owned(), remote.text_token_hash).await
        {
            return next.run(request).await;
        }
        return (StatusCode::UNAUTHORIZED, "Invalid token or token scope").into_response();
    }
    if !remote.authentication_enabled {
        return next.run(request).await;
    }
    let credentials = authorization
        .strip_prefix("Basic ")
        .and_then(|encoded| STANDARD.decode(encoded).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|credentials| {
            let (username, password) = credentials.split_once(':')?;
            (username == remote.username).then(|| password.to_owned())
        });
    let authorized = match credentials {
        Some(password) => crate::remote_auth::verify_request(password, remote.password_hash).await,
        None => false,
    };
    if authorized {
        next.run(request).await
    } else {
        Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header(
                header::WWW_AUTHENTICATE,
                "Basic realm=\"TelemetryForge Remote Deck\"",
            )
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from("Authentication required"))
            .expect("valid authentication response")
    }
}

async fn health() -> Json<Value> {
    Json(json!({"name":"TelemetryForge Remote Deck","version":env!("CARGO_PKG_VERSION")}))
}

/// Lightweight external-data endpoint. It shares the Remote Deck's auth
/// middleware, but does not write a screen/configuration file.
async fn update_widget_text(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(payload): Json<RemoteTextUpdate>,
) -> impl IntoResponse {
    let key = key.trim();
    if key.is_empty()
        || key.len() > 64
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":"Use a key up to 64 characters: letters, numbers, -, _ or ."}))).into_response();
    }
    if payload.text.chars().count() > 512 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"Text exceeds 512 characters"})),
        )
            .into_response();
    }
    let mut texts = state.remote_texts.write();
    if texts.len() >= 256 && !texts.contains_key(key) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"ok":false,"error":"Remote text key limit reached (256)"})),
        )
            .into_response();
    }
    let changed = texts.get(key) != Some(&payload.text);
    if changed {
        texts.insert(key.to_owned(), payload.text);
    }
    // The visual signature already observes this map. A data update must not
    // trigger a screen transition or override automatic screen switching.
    (
        StatusCode::OK,
        Json(json!({"ok":true,"key":key,"changed":changed})),
    )
        .into_response()
}

async fn index() -> impl IntoResponse {
    let html = INDEX.replace(
        "<script src=\"i18n.js\"></script>",
        "<script src=\"remote-bridge.js\"></script>\n  <script src=\"i18n.js\"></script>",
    );
    asset("text/html; charset=utf-8", html.into_bytes())
}

async fn styles() -> impl IntoResponse {
    asset("text/css; charset=utf-8", STYLES.as_bytes().to_vec())
}

async fn i18n() -> impl IntoResponse {
    asset("text/javascript; charset=utf-8", I18N.as_bytes().to_vec())
}

async fn bridge() -> impl IntoResponse {
    asset("text/javascript; charset=utf-8", BRIDGE.as_bytes().to_vec())
}

async fn app_script() -> impl IntoResponse {
    asset("text/javascript; charset=utf-8", APP.as_bytes().to_vec())
}

fn asset(content_type: &'static str, bytes: Vec<u8>) -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(bytes))
        .expect("valid asset response")
}

async fn invoke(
    State(state): State<AppState>,
    Path(command): Path<String>,
    payload: Option<Json<Value>>,
) -> impl IntoResponse {
    let arguments = payload
        .map(|Json(value)| value)
        .unwrap_or_else(|| json!({}));
    let result = tokio::task::spawn_blocking(move || invoke_command(&state, &command, arguments))
        .await
        .unwrap_or_else(|error| Err(format!("Remote command failed: {error}")));
    match result {
        Ok(result) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                result,
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                result: Value::Null,
                error: Some(error),
            }),
        ),
    }
}

fn invoke_command(state: &AppState, command: &str, args: Value) -> Result<Value, String> {
    match command {
        "get_config" => value(ui::public_config(state.config.read().clone())),
        "get_remote_info" => value(ui::remote_info_state(state)),
        "set_remote_security" => {
            ui::set_remote_security_state(state, argument(&args, "settings")?)?;
            value(ui::remote_info_state(state))
        }
        "get_active_screen" => value(state.active_screen.read().clone()),
        "save_config" => {
            let mut config: AppConfig = argument(&args, "config")?;
            config.remote = state.config.read().remote.clone();
            config.quick_screens = state.config.read().quick_screens.clone();
            persistence::save(&state.config_path, &config).map_err(error)?;
            *state.config.write() = config;
            Ok(Value::Null)
        }
        "list_screens" => value(persistence::list_profiles(&state.config_path).map_err(error)?),
        "set_quick_screen" => {
            let slot: String = argument(&args, "slot")?;
            let screen: Option<String> = argument(&args, "screen")?;
            ui::set_quick_screen_state(state, &slot, screen)?;
            Ok(Value::Null)
        }
        "save_screen" => {
            let name: String = argument(&args, "name")?;
            let mut config: AppConfig = argument(&args, "config")?;
            config.remote = state.config.read().remote.clone();
            config.quick_screens = state.config.read().quick_screens.clone();
            let path = persistence::profile_path(&state.config_path, &name).map_err(error)?;
            persistence::save(&path, &ui::profile_config(config.clone())).map_err(error)?;
            persistence::save(&state.config_path, &config).map_err(error)?;
            persistence::save_active_screen(&state.config_path, Some(&name)).map_err(error)?;
            *state.config.write() = config;
            *state.active_screen.write() = Some(name);
            Ok(Value::Null)
        }
        "load_screen" => {
            let name: String = argument(&args, "name")?;
            let path = persistence::profile_path(&state.config_path, &name).map_err(error)?;
            let loaded = persistence::load_or_create(&path).map_err(error)?;
            let config = merge_screen_settings(loaded, &state.config.read());
            persistence::save(&state.config_path, &config).map_err(error)?;
            persistence::save_active_screen(&state.config_path, Some(&name)).map_err(error)?;
            *state.config.write() = config.clone();
            *state.active_screen.write() = Some(name);
            state.scene_revision.fetch_add(1, Ordering::Relaxed);
            value(ui::public_config(config))
        }
        "new_screen" => new_screen(state, argument(&args, "name")?),
        "delete_screen" => {
            let name: String = argument(&args, "name")?;
            let path = persistence::profile_path(&state.config_path, &name).map_err(error)?;
            if path.exists() {
                std::fs::remove_file(path).map_err(error)?;
            }
            if state.active_screen.read().as_deref() == Some(name.as_str()) {
                persistence::save_active_screen(&state.config_path, None).map_err(error)?;
                *state.active_screen.write() = None;
            }
            Ok(Value::Null)
        }
        "get_preview" => preview(state, state.config.read().clone()),
        "preview_config" => preview(state, argument(&args, "config")?),
        "test_sensors" => test_sensors(state),
        "list_displays" => value(display_driver::detection::list().map_err(error)?),
        "list_superwidgets" => value(superwidgets::list().map_err(error)?),
        "start_rendering" => {
            ui::start_rendering_state(state)?;
            Ok(Value::Null)
        }
        "stop_rendering" => {
            ui::stop_rendering_state(state);
            Ok(Value::Null)
        }
        "get_status" => value(ui::get_status_state(state)),
        "render_once" => {
            render_once(state)?;
            Ok(Value::Null)
        }
        "test_display" => value(test_display(state)?),
        "set_display_brightness" => {
            set_brightness(state, argument(&args, "brightness")?)?;
            Ok(Value::Null)
        }
        "get_autostart" => value(windows_startup::is_enabled().map_err(error)?),
        "set_autostart" => {
            windows_startup::set_enabled(argument(&args, "enabled")?).map_err(error)?;
            Ok(Value::Null)
        }
        "select_background"
        | "select_background_folder"
        | "select_gif"
        | "import_package"
        | "export_package"
        | "import_superwidget" => Err(
            "This file operation is not available in Remote Deck yet. Use the Windows app.".into(),
        ),
        _ => Err(format!("Unsupported Remote Deck command: {command}")),
    }
}

fn new_screen(state: &AppState, name: String) -> Result<Value, String> {
    let current = state.config.read().clone();
    let mut config = AppConfig::default();
    config.display = current.display;
    config.libre_hardware_monitor_dll = current.libre_hardware_monitor_dll;
    config.cpu_temperature_source = current.cpu_temperature_source;
    config.cpu_clock_source = current.cpu_clock_source;
    config.fan_sensor = current.fan_sensor;
    config.disk_source = current.disk_source;
    config.weather = current.weather;
    config.automation = current.automation;
    config.transition = current.transition;
    config.effects = current.effects;
    config.remote = current.remote;
    config.quick_screens = current.quick_screens;
    config.widgets.clear();
    let path = persistence::profile_path(&state.config_path, &name).map_err(error)?;
    if path.exists() {
        return Err("A screen with this name already exists".into());
    }
    persistence::save(&path, &ui::profile_config(config.clone())).map_err(error)?;
    persistence::save(&state.config_path, &config).map_err(error)?;
    persistence::save_active_screen(&state.config_path, Some(&name)).map_err(error)?;
    *state.config.write() = config.clone();
    *state.active_screen.write() = Some(name);
    state.scene_revision.fetch_add(1, Ordering::Relaxed);
    value(ui::public_config(config))
}

fn merge_screen_settings(mut screen: AppConfig, current: &AppConfig) -> AppConfig {
    screen.automation = current.automation.clone();
    screen.transition = current.transition.clone();
    screen.effects = current.effects.clone();
    screen.libre_hardware_monitor_dll = current.libre_hardware_monitor_dll.clone();
    screen.cpu_temperature_source = current.cpu_temperature_source;
    screen.cpu_clock_source = current.cpu_clock_source;
    screen.fan_sensor = current.fan_sensor.clone();
    screen.disk_source = current.disk_source.clone();
    screen.weather = current.weather.clone();
    screen.remote = current.remote.clone();
    screen.quick_screens = current.quick_screens.clone();
    screen
}

fn preview(state: &AppState, config: AppConfig) -> Result<Value, String> {
    let image = renderer::render(&config, &state.sensors.read(), &state.remote_texts.read())
        .map_err(error)?;
    let png = renderer::canvas::png_bytes(&image).map_err(error)?;
    value(format!("data:image/png;base64,{}", STANDARD.encode(png)))
}

fn test_sensors(state: &AppState) -> Result<Value, String> {
    let config = state.config.read().clone();
    let snapshot = poller::read_snapshot(
        config.libre_hardware_monitor_dll.as_deref(),
        config.cpu_temperature_source,
        config.cpu_clock_source,
        config.fan_sensor.as_deref(),
        config.disk_source.as_deref(),
        &config.weather,
    );
    *state.sensors.write() = snapshot.clone();
    value(snapshot)
}

fn render_once(state: &AppState) -> Result<(), String> {
    let config = state.config.read().clone();
    let snapshot = poller::read_snapshot(
        config.libre_hardware_monitor_dll.as_deref(),
        config.cpu_temperature_source,
        config.cpu_clock_source,
        config.fan_sensor.as_deref(),
        config.disk_source.as_deref(),
        &config.weather,
    );
    let image = renderer::render(&config, &snapshot, &state.remote_texts.read()).map_err(error)?;
    display_driver::send_frame(&config.display, &image).map_err(error)?;
    *state.sensors.write() = snapshot;
    *state.status.write() = "Frame sent".into();
    Ok(())
}

fn test_display(state: &AppState) -> Result<String, String> {
    use image::{Rgb, RgbImage};
    let config = state.config.read().clone();
    let port = display_driver::detection::resolve_port(&config.display.port).map_err(error)?;
    let mut image = RgbImage::new(config.display.width, config.display.height);
    let half_width = image.width() / 2;
    let half_height = image.height() / 2;
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        *pixel = match (x < half_width, y < half_height) {
            (true, true) => Rgb([255, 0, 0]),
            (false, true) => Rgb([0, 255, 0]),
            (true, false) => Rgb([0, 0, 255]),
            (false, false) => Rgb([255, 255, 255]),
        };
    }
    display_driver::send_frame(&config.display, &image).map_err(error)?;
    let message = format!("RGB test successfully sent to {port}");
    *state.status.write() = message.clone();
    Ok(message)
}

fn set_brightness(state: &AppState, brightness: u8) -> Result<(), String> {
    let mut config = state.config.write();
    config.display.brightness = brightness.min(100);
    persistence::save(&state.config_path, &config).map_err(error)?;
    if state.worker.lock().is_none() {
        display_driver::apply_brightness(&config.display).map_err(error)?;
    }
    Ok(())
}

fn argument<T: serde::de::DeserializeOwned>(value: &Value, name: &str) -> Result<T, String> {
    serde_json::from_value(
        value
            .get(name)
            .cloned()
            .ok_or_else(|| format!("Missing argument: {name}"))?,
    )
    .map_err(error)
}

fn value<T: Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(error)
}

fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::{Mutex, RwLock};
    use std::{
        collections::HashMap,
        sync::{
            atomic::{AtomicBool, AtomicU64},
            Arc,
        },
    };

    #[test]
    fn http_text_token_is_scoped_and_posts_do_not_change_scene() {
        let root = crate::safe_files::Staging::new(&std::env::temp_dir()).unwrap();
        let mut config = AppConfig::default();
        config.remote.authentication_enabled = true;
        config.remote.password_hash =
            crate::remote_auth::hash_password("admin-password-test").unwrap();
        config.remote.text_token_hash =
            crate::remote_auth::hash_password("text-token-for-test").unwrap();
        let state = AppState {
            config_path: root.0.join("config.yaml"),
            config: Arc::new(RwLock::new(config)),
            sensors: Arc::new(RwLock::new(Default::default())),
            worker: Arc::new(Mutex::new(None)),
            status: Arc::new(RwLock::new(String::new())),
            scene_revision: Arc::new(AtomicU64::new(7)),
            active_screen: Arc::new(RwLock::new(None)),
            remote_texts: Arc::new(RwLock::new(HashMap::new())),
            remote_running: Arc::new(AtomicBool::new(false)),
            diagnostics: Arc::new(RwLock::new(Default::default())),
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = router(state.clone());
        let server = runtime.spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let text_url = format!("{url}/api/widget-text/status");
        assert_eq!(
            client
                .post(&text_url)
                .json(&json!({"text":"Hello"}))
                .send()
                .unwrap()
                .status(),
            401
        );
        for changed in [true, false] {
            let response = client
                .post(&text_url)
                .bearer_auth("text-token-for-test")
                .json(&json!({"text":"Hello"}))
                .send()
                .unwrap();
            assert_eq!(response.status(), 200);
            assert_eq!(response.json::<Value>().unwrap()["changed"], changed);
        }
        assert_eq!(state.scene_revision.load(Ordering::Relaxed), 7);
        assert_eq!(state.remote_texts.read()["status"], "Hello");
        assert_eq!(
            client
                .post(format!("{url}/api/invoke/get_config"))
                .bearer_auth("text-token-for-test")
                .json(&json!({}))
                .send()
                .unwrap()
                .status(),
            401
        );
        assert_eq!(
            client
                .post(&text_url)
                .bearer_auth("text-token-for-test")
                .json(&json!({"text":"x".repeat(513)}))
                .send()
                .unwrap()
                .status(),
            400
        );
        state.config.write().remote.text_token_hash.clear();
        assert_eq!(
            client
                .post(&text_url)
                .bearer_auth("text-token-for-test")
                .json(&json!({"text":"No"}))
                .send()
                .unwrap()
                .status(),
            401
        );
        assert_eq!(
            client
                .get(format!("{url}/api/health"))
                .basic_auth("admin", Some("admin-password-test"))
                .send()
                .unwrap()
                .status(),
            200
        );
        server.abort();
    }
}
