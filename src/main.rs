mod scheduler;
mod state;
mod telegram;
mod valve;

use std::sync::Arc;

use tokio::signal;
use tokio::sync::Mutex;
use tracing::info;

use valve::Valve;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "irrigator=info".parse().unwrap()),
        )
        .init();

    dotenvy::dotenv().ok();

    let bot_token = std::env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN must be set");
    let chat_id: i64 = std::env::var("TELEGRAM_CHAT_ID")
        .expect("TELEGRAM_CHAT_ID must be set")
        .parse()
        .expect("TELEGRAM_CHAT_ID must be a number");

    let state = Arc::new(Mutex::new(state::AppState::load()));
    let relay_pin = state.lock().await.relay_pin;

    let valve = Arc::new(Mutex::new(
        Valve::new(relay_pin).expect("failed to initialize GPIO"),
    ));

    info!("irrigator starting");

    // Spawn telegram bot.
    let tg_state = Arc::clone(&state);
    let tg_valve = Arc::clone(&valve);
    let tg_handle = tokio::spawn(async move {
        telegram::run(bot_token, chat_id, tg_state, tg_valve).await;
    });

    // Spawn scheduler.
    let sched_state = Arc::clone(&state);
    let sched_valve = Arc::clone(&valve);
    let sched_handle = tokio::spawn(async move {
        scheduler::run(sched_state, sched_valve).await;
    });

    // Wait for shutdown signal.
    signal::ctrl_c().await.ok();
    info!("shutdown signal received");

    // Force valve off.
    valve.lock().await.close();
    state.lock().await.valve_open = false;
    state.lock().await.save();
    info!("valve closed, state saved, exiting");

    tg_handle.abort();
    sched_handle.abort();
}
