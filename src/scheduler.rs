use std::sync::Arc;

use chrono::{Local, Timelike, Utc};
use tokio::sync::Mutex;
use tracing::info;

use crate::state::AppState;
use crate::valve::Valve;

/// Run the scheduler loop. Checks every 30 seconds for:
/// 1. Auto-off timer expiry
/// 2. Schedule slot matches
pub async fn run(state: Arc<Mutex<AppState>>, valve: Arc<Mutex<Valve>>) {
    info!("scheduler started");
    let mut last_triggered: Option<(u32, u32)> = None;

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;

        // Check auto-off timer.
        {
            let mut st = state.lock().await;
            if st.valve_open {
                if let Some(off_at) = st.auto_off_at {
                    if Utc::now() >= off_at {
                        info!("auto-off timer expired, closing valve");
                        valve.lock().await.close();
                        st.valve_open = false;
                        st.auto_off_at = None;
                        st.save();
                    }
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

                    valve.lock().await.open();
                    st.valve_open = true;
                    st.auto_off_at =
                        Some(Utc::now() + chrono::Duration::minutes(duration_min as i64));
                    st.record_watering(duration_min, "schedule");
                    last_triggered = Some((current_hour, current_minute));
                }
            }
        }
    }
}
