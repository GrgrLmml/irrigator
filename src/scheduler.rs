use std::sync::Arc;

use chrono::{Local, Timelike, Utc};
use teloxide::prelude::*;
use tokio::sync::Mutex;
use tracing::info;

use crate::flow::FlowSensor;
use crate::state::AppState;
use crate::telegram;
use crate::valve::Valve;

/// Run the scheduler loop. Checks every 30 seconds for:
/// 1. Auto-off timer expiry
/// 2. Schedule slot matches
/// 3. Periodic flow reports while watering
pub async fn run(
    state: Arc<Mutex<AppState>>,
    valve: Arc<Mutex<Valve>>,
    flow: Arc<Mutex<FlowSensor>>,
    bot: Bot,
    chat_id: ChatId,
) {
    info!("scheduler started");
    let mut last_triggered: Option<(u32, u32)> = None;
    let mut last_flow_report: Option<tokio::time::Instant> = None;
    let mut last_reported_liters: f64 = 0.0;

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;

        // Check auto-off timer.
        {
            let mut st = state.lock().await;
            if st.valve_open {
                if let Some(off_at) = st.auto_off_at {
                    if Utc::now() >= off_at {
                        info!("auto-off timer expired, closing valve");
                        let final_liters = flow.lock().await.session_liters();
                        valve.lock().await.close();
                        st.valve_open = false;
                        st.auto_off_at = None;
                        st.update_last_watering_volume(final_liters);
                        telegram::notify(
                            &bot,
                            chat_id,
                            &format!("Auto-off: valve CLOSED. Total: {final_liters:.1}L."),
                        )
                        .await;
                    }
                }
            }
        }

        // Periodic flow reports while watering.
        {
            let st = state.lock().await;
            if st.valve_open {
                let sensor = flow.lock().await;
                let liters = sensor.session_liters();
                let elapsed = last_flow_report
                    .map(|t| t.elapsed().as_secs())
                    .unwrap_or(0);
                let delta = liters - last_reported_liters;

                if elapsed >= 120 || delta >= 5.0 {
                    drop(sensor);
                    drop(st);
                    telegram::notify(
                        &bot,
                        chat_id,
                        &format!("Watering... {liters:.1}L so far."),
                    )
                    .await;
                    last_flow_report = Some(tokio::time::Instant::now());
                    last_reported_liters = liters;
                }
            }
        }

        // Check schedule.
        {
            let mut st = state.lock().await;
            if !st.schedule.enabled || st.valve_open {
                continue;
            }

            let now = Local::now();
            let current_hour = now.hour();
            let current_minute = now.minute();

            // Copy matching slot data to satisfy borrow checker.
            let matched = st
                .schedule
                .slots
                .iter()
                .find(|s| s.hour == current_hour && s.minute == current_minute)
                .map(|s| s.duration_min);

            if let Some(duration_min) = matched {
                if last_triggered != Some((current_hour, current_minute)) {
                    info!(
                        hour = current_hour,
                        minute = current_minute,
                        duration = duration_min,
                        "schedule triggered"
                    );

                    flow.lock().await.start_session();
                    valve.lock().await.open();
                    st.valve_open = true;
                    st.auto_off_at =
                        Some(Utc::now() + chrono::Duration::minutes(duration_min as i64));
                    st.record_watering(duration_min, "schedule", None);
                    last_triggered = Some((current_hour, current_minute));
                    last_flow_report = Some(tokio::time::Instant::now());
                    last_reported_liters = 0.0;

                    telegram::notify(
                        &bot,
                        chat_id,
                        &format!(
                            "Schedule: valve OPENED for {duration_min}min ({:02}:{:02}).",
                            current_hour, current_minute
                        ),
                    )
                    .await;
                }
            }
        }
    }
}
