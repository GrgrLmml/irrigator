//! HTTP server: JSON API + mobile web UI.
//!
//! Lock order across the whole daemon: state → flow → valve.
//! Handlers MUST snapshot fields into a local DTO and drop the mutex guard
//! BEFORE serializing JSON. Never hold a lock across an .await on anything
//! other than a mutex.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Local, Timelike, Utc};
use serde::{Deserialize, Serialize};
use teloxide::prelude::*;
use tokio::sync::Mutex;
use tower_http::compression::CompressionLayer;
use tracing::{info, warn};

use crate::flow::FlowSensor;
use crate::state::{AppState, Slot, WateringEvent};
use crate::telegram;
use crate::valve::Valve;

// --- Embedded static assets ---------------------------------------------------
// For release builds these ship inside the binary. For dev, override via the
// IRRIGATOR_UI_DIR env var (served from disk, one `cargo run` iteration).
const INDEX_HTML: &str = include_str!("../ui/index.html");
const APP_JS: &str = include_str!("../ui/app.js");
const STYLES_CSS: &str = include_str!("../ui/styles.css");
const MANIFEST_JSON: &str = include_str!("../ui/manifest.json");
const SW_JS: &str = include_str!("../ui/sw.js");

#[derive(Clone)]
struct WebState {
    state: Arc<Mutex<AppState>>,
    valve: Arc<Mutex<Valve>>,
    flow: Arc<Mutex<FlowSensor>>,
    bot: Bot,
    chat_id: ChatId,
    ui_dir: Option<String>,
}

pub async fn run(
    state: Arc<Mutex<AppState>>,
    valve: Arc<Mutex<Valve>>,
    flow: Arc<Mutex<FlowSensor>>,
    bot: Bot,
    chat_id: ChatId,
    bind_addr: String,
) {
    let ui_dir = std::env::var("IRRIGATOR_UI_DIR").ok();
    if let Some(ref d) = ui_dir {
        info!(dir = %d, "web UI serving from disk (dev override)");
    }

    let shared = WebState { state, valve, flow, bot, chat_id, ui_dir };

    let app = Router::new()
        .route("/", get(serve_index))
        .route("/app.js", get(serve_js))
        .route("/styles.css", get(serve_css))
        .route("/manifest.json", get(serve_manifest))
        .route("/sw.js", get(serve_sw))
        .route("/api/health", get(handle_health))
        .route("/api/status", get(handle_status))
        .route("/api/summary", get(handle_summary))
        .route("/api/history", get(handle_history))
        .route("/api/schedule", get(handle_schedule_get).post(handle_schedule_set))
        .route("/api/schedule/enabled", post(handle_schedule_enabled))
        .route("/api/valve/open", post(handle_valve_open))
        .route("/api/valve/close", post(handle_valve_close))
        .route("/api/settings/daily_target", post(handle_daily_target))
        .layer(CompressionLayer::new())
        .with_state(shared);

    let addr: SocketAddr = match bind_addr.parse() {
        Ok(a) => a,
        Err(e) => {
            warn!(addr = %bind_addr, error = %e, "invalid bind address, falling back to 0.0.0.0:8080");
            "0.0.0.0:8080".parse().unwrap()
        }
    };

    info!(addr = %addr, "web server listening");

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            warn!(error = %e, "failed to bind web server");
            return;
        }
    };

    if let Err(e) = axum::serve(listener, app).await {
        warn!(error = %e, "web server exited");
    }
}

// --- Static asset handlers ----------------------------------------------------

async fn serve_asset(state: &WebState, relative: &str, fallback: &str, content_type: &str) -> Response {
    let body = if let Some(dir) = &state.ui_dir {
        let path = std::path::Path::new(dir).join(relative);
        tokio::fs::read_to_string(&path).await.unwrap_or_else(|_| fallback.to_string())
    } else {
        fallback.to_string()
    };
    ([(header::CONTENT_TYPE, content_type)], body).into_response()
}

async fn serve_index(State(s): State<WebState>) -> Response {
    serve_asset(&s, "index.html", INDEX_HTML, "text/html; charset=utf-8").await
}
async fn serve_js(State(s): State<WebState>) -> Response {
    serve_asset(&s, "app.js", APP_JS, "application/javascript; charset=utf-8").await
}
async fn serve_css(State(s): State<WebState>) -> Response {
    serve_asset(&s, "styles.css", STYLES_CSS, "text/css; charset=utf-8").await
}
async fn serve_manifest(State(s): State<WebState>) -> Response {
    serve_asset(&s, "manifest.json", MANIFEST_JSON, "application/manifest+json").await
}
async fn serve_sw(State(s): State<WebState>) -> Response {
    serve_asset(&s, "sw.js", SW_JS, "application/javascript; charset=utf-8").await
}

// --- API handlers -------------------------------------------------------------

#[derive(Serialize)]
struct HealthDto { ok: bool, version: &'static str, valve_open: bool }

async fn handle_health(State(s): State<WebState>) -> Json<HealthDto> {
    let valve_open = { s.state.lock().await.valve_open };
    Json(HealthDto {
        ok: true,
        version: env!("CARGO_PKG_VERSION"),
        valve_open,
    })
}

#[derive(Serialize)]
struct StatusDto {
    valve_open: bool,
    auto_off_at: Option<DateTime<Utc>>,
    auto_off_seconds_remaining: Option<i64>,
    session_liters: f64,
    session_seconds: u64,
    session_flow_lpm: f64,
    session_source: Option<String>,
    schedule_enabled: bool,
    next_slot: Option<NextSlotDto>,
    boot_time: DateTime<Utc>,
    uptime_seconds: i64,
    now: DateTime<Utc>,
    server_tz_offset_minutes: i32,
    anomaly: bool,
}

#[derive(Serialize)]
struct NextSlotDto {
    hour: u32,
    minute: u32,
    duration_min: u32,
    when: &'static str, // "today" or "tomorrow"
}

async fn handle_status(State(s): State<WebState>) -> Json<StatusDto> {
    let now = Utc::now();
    // Snapshot state fields and drop guard before serializing.
    let (
        valve_open, auto_off_at, schedule_enabled, next_slot,
        boot_time, uptime_seconds, source,
    ) = {
        let st = s.state.lock().await;
        let next = if st.schedule.enabled { next_slot_from(&st) } else { None };
        let src = if st.valve_open {
            st.log.last().map(|e| e.source.clone())
        } else {
            None
        };
        (
            st.valve_open,
            st.auto_off_at,
            st.schedule.enabled,
            next,
            st.boot_time,
            (now - st.boot_time).num_seconds(),
            src,
        )
    };

    let (session_liters, session_seconds, session_flow_lpm) = if valve_open {
        let f = s.flow.lock().await;
        (f.session_liters(), f.session_elapsed_secs(), f.session_flow_lpm())
    } else {
        (0.0, 0, 0.0)
    };

    let auto_off_seconds_remaining = auto_off_at.map(|t| (t - now).num_seconds());

    // Flow anomaly: valve open, running for ≥30s, less than 0.05 L/min.
    let anomaly = valve_open && session_seconds >= 30 && session_flow_lpm < 0.05;

    let tz_offset_minutes = Local::now().offset().local_minus_utc() / 60;

    Json(StatusDto {
        valve_open,
        auto_off_at,
        auto_off_seconds_remaining,
        session_liters,
        session_seconds,
        session_flow_lpm,
        session_source: source,
        schedule_enabled,
        next_slot,
        boot_time,
        uptime_seconds,
        now,
        server_tz_offset_minutes: tz_offset_minutes,
        anomaly,
    })
}

fn next_slot_from(st: &AppState) -> Option<NextSlotDto> {
    let now = Local::now();
    let cur = now.hour() * 60 + now.minute();
    for s in &st.schedule.slots {
        if s.hour * 60 + s.minute > cur {
            return Some(NextSlotDto { hour: s.hour, minute: s.minute, duration_min: s.duration_min, when: "today" });
        }
    }
    st.schedule.slots.first().map(|s| NextSlotDto {
        hour: s.hour, minute: s.minute, duration_min: s.duration_min, when: "tomorrow"
    })
}

#[derive(Serialize)]
struct SummaryDto {
    today: TodayDto,
    last_7_days: Vec<DayDto>,
    lifetime: LifetimeDto,
    peak_flow_lpm: f64,
    avg_session_liters: f64,
    daily_target_liters: f64,
}
#[derive(Serialize)]
struct TodayDto { sessions: usize, liters: f64, minutes: u32, target_liters: f64 }
#[derive(Serialize)]
struct DayDto { date: String, liters: f64, sessions: usize }
#[derive(Serialize)]
struct LifetimeDto { liters: f64, sessions: usize }

async fn handle_summary(State(s): State<WebState>) -> Json<SummaryDto> {
    let st = s.state.lock().await;

    let today_local = Local::now().date_naive();
    let mut today = TodayDto { sessions: 0, liters: 0.0, minutes: 0, target_liters: st.daily_target_liters };

    // 7-day buckets (local dates, oldest first, today last).
    let mut days: Vec<(chrono::NaiveDate, f64, usize)> = (0..7)
        .rev()
        .map(|d| (today_local - chrono::Duration::days(d), 0.0, 0usize))
        .collect();

    for e in &st.log {
        let local_date = e.timestamp.with_timezone(&Local).date_naive();
        if local_date == today_local {
            today.sessions += 1;
            today.liters += e.volume_liters.unwrap_or(0.0);
            today.minutes += e.duration_min;
        }
        for d in days.iter_mut() {
            if d.0 == local_date {
                d.1 += e.volume_liters.unwrap_or(0.0);
                d.2 += 1;
                break;
            }
        }
    }

    let peak_flow_lpm = st.log.iter()
        .filter_map(|e| {
            let v = e.volume_liters?;
            if e.duration_min == 0 { return None; }
            Some(v / (e.duration_min as f64))
        })
        .fold(0.0, f64::max);

    let (total_vol, count) = st.log.iter()
        .filter_map(|e| e.volume_liters)
        .fold((0.0, 0usize), |(s, n), v| (s + v, n + 1));
    let avg_session_liters = if count > 0 { total_vol / count as f64 } else { 0.0 };

    let lifetime = LifetimeDto {
        liters: st.lifetime_liters,
        sessions: st.log.len(),
    };

    let last_7_days = days.into_iter()
        .map(|(d, l, n)| DayDto { date: d.format("%Y-%m-%d").to_string(), liters: l, sessions: n })
        .collect();

    Json(SummaryDto {
        today,
        last_7_days,
        lifetime,
        peak_flow_lpm,
        avg_session_liters,
        daily_target_liters: st.daily_target_liters,
    })
}

#[derive(Deserialize)]
struct HistoryQuery { limit: Option<usize> }

#[derive(Serialize)]
struct HistoryDto { events: Vec<HistoryEventDto> }
#[derive(Serialize)]
struct HistoryEventDto {
    timestamp: DateTime<Utc>,
    duration_min: u32,
    source: String,
    volume_liters: Option<f64>,
    flow_lpm: Option<f64>,
}

async fn handle_history(State(s): State<WebState>, Query(q): Query<HistoryQuery>) -> Json<HistoryDto> {
    let limit = q.limit.unwrap_or(10).min(500);
    let st = s.state.lock().await;
    let events: Vec<HistoryEventDto> = st.log.iter().rev().take(limit).map(|e| {
        let flow_lpm = e.volume_liters.and_then(|v| {
            if e.duration_min == 0 { None } else { Some(v / e.duration_min as f64) }
        });
        HistoryEventDto {
            timestamp: e.timestamp,
            duration_min: e.duration_min,
            source: e.source.clone(),
            volume_liters: e.volume_liters,
            flow_lpm,
        }
    }).collect();
    Json(HistoryDto { events })
}

#[derive(Serialize, Deserialize)]
struct ScheduleDto {
    enabled: bool,
    slots: Vec<Slot>,
    max_on_minutes: u32,
}

async fn handle_schedule_get(State(s): State<WebState>) -> Json<ScheduleDto> {
    let st = s.state.lock().await;
    Json(ScheduleDto {
        enabled: st.schedule.enabled,
        slots: st.schedule.slots.clone(),
        max_on_minutes: st.max_on_minutes,
    })
}

#[derive(Deserialize)]
struct SchedulePost { slots: Vec<Slot>, enabled: Option<bool> }

async fn handle_schedule_set(
    State(s): State<WebState>,
    Json(body): Json<SchedulePost>,
) -> Response {
    match crate::state::Schedule::try_from_slots(body.slots) {
        Ok(sorted) => {
            let mut st = s.state.lock().await;
            st.schedule.slots = sorted;
            if let Some(en) = body.enabled {
                st.schedule.enabled = en;
            }
            st.save();
            let dto = ScheduleDto {
                enabled: st.schedule.enabled,
                slots: st.schedule.slots.clone(),
                max_on_minutes: st.max_on_minutes,
            };
            Json(dto).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(err(&e))).into_response(),
    }
}

#[derive(Deserialize)]
struct EnabledPost { enabled: bool }

async fn handle_schedule_enabled(
    State(s): State<WebState>,
    Json(body): Json<EnabledPost>,
) -> Json<ScheduleDto> {
    let mut st = s.state.lock().await;
    st.schedule.enabled = body.enabled;
    st.save();
    Json(ScheduleDto {
        enabled: st.schedule.enabled,
        slots: st.schedule.slots.clone(),
        max_on_minutes: st.max_on_minutes,
    })
}

#[derive(Deserialize)]
struct ValveOpenPost { duration_min: u32 }

async fn handle_valve_open(
    State(s): State<WebState>,
    Json(body): Json<ValveOpenPost>,
) -> Response {
    {
        let st = s.state.lock().await;
        if body.duration_min == 0 {
            return (StatusCode::BAD_REQUEST, Json(err("duration must be > 0"))).into_response();
        }
        if body.duration_min > st.max_on_minutes {
            return (
                StatusCode::BAD_REQUEST,
                Json(err(&format!("duration must be ≤ {} minutes", st.max_on_minutes))),
            ).into_response();
        }
    }

    // state → flow → valve
    let mut st = s.state.lock().await;
    s.flow.lock().await.start_session();
    s.valve.lock().await.open();
    st.start_session(body.duration_min, "web");
    drop(st);

    let msg = format!("Web: valve OPENED for {}min.", body.duration_min);
    telegram::notify(&s.bot, s.chat_id, &msg).await;

    // Return a fresh status snapshot.
    status_response(&s).await
}

async fn handle_valve_close(State(s): State<WebState>) -> Response {
    let final_liters = s.flow.lock().await.session_liters();
    s.valve.lock().await.close();
    let mut st = s.state.lock().await;
    let had_valve_open = st.valve_open;
    st.finish_session(final_liters);
    drop(st);

    if had_valve_open {
        let msg = if final_liters > 0.0 {
            format!("Web: valve CLOSED. Total: {final_liters:.1}L.")
        } else {
            "Web: valve CLOSED.".to_string()
        };
        telegram::notify(&s.bot, s.chat_id, &msg).await;
    }

    status_response(&s).await
}

#[derive(Deserialize)]
struct DailyTargetPost { liters: f64 }

async fn handle_daily_target(
    State(s): State<WebState>,
    Json(body): Json<DailyTargetPost>,
) -> Response {
    if body.liters < 0.0 || body.liters > 10_000.0 {
        return (StatusCode::BAD_REQUEST, Json(err("liters must be 0-10000"))).into_response();
    }
    let mut st = s.state.lock().await;
    st.daily_target_liters = body.liters;
    st.save();
    Json(serde_json::json!({"daily_target_liters": body.liters})).into_response()
}

// Helper: fetch current status after a mutation.
async fn status_response(s: &WebState) -> Response {
    let Json(dto) = handle_status(State(s.clone())).await;
    Json(dto).into_response()
}

fn err(msg: &str) -> serde_json::Value {
    serde_json::json!({ "error": msg })
}

// Silence unused-import warnings on macOS where some imports are only needed by handlers
// that end up mono-morphized on linux builds.
#[allow(dead_code)]
fn _unused(_e: WateringEvent) {}
