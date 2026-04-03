use std::sync::Arc;

use teloxide::prelude::*;
use teloxide::utils::command::BotCommands;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::flow::FlowSensor;
use crate::state::AppState;
use crate::valve::Valve;

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
enum Command {
    #[command(description = "open valve (default 10 min)")]
    On(String),
    #[command(description = "close valve")]
    Off,
    #[command(description = "show current status")]
    Status,
    #[command(description = "show schedule")]
    Schedule,
    #[command(description = "set schedule: /set 06:00,8 10:00,8 14:00,8")]
    Set(String),
    #[command(description = "enable schedule")]
    Enable,
    #[command(description = "disable schedule")]
    Disable,
    #[command(description = "show this help")]
    Help,
    #[command(description = "start")]
    Start,
}

/// Send a notification message to the owner chat.
pub async fn notify(bot: &Bot, chat_id: ChatId, message: &str) {
    if let Err(e) = bot.send_message(chat_id, message).await {
        tracing::warn!(error = %e, "failed to send notification");
    }
}

pub async fn run(
    bot_token: String,
    chat_id: i64,
    state: Arc<Mutex<AppState>>,
    valve: Arc<Mutex<Valve>>,
    flow: Arc<Mutex<FlowSensor>>,
) {
    info!("telegram bot starting...");
    let mut backoff_secs = 2u64;
    const MAX_BACKOFF: u64 = 120;

    loop {
        let bot = Bot::new(&bot_token);
        let allowed_chat = ChatId(chat_id);
        let state = Arc::clone(&state);
        let valve = Arc::clone(&valve);
        let flow = Arc::clone(&flow);

        // teloxide::repl panics on network errors during init (e.g. DNS failure
        // when LTE isn't ready yet). Catch the panic and retry with backoff.
        let result = tokio::spawn(async move {
            teloxide::repl(bot, move |bot: Bot, msg: Message| {
                let state = Arc::clone(&state);
                let valve = Arc::clone(&valve);
                let flow = Arc::clone(&flow);
                let allowed_chat = allowed_chat;
                async move {
                    if msg.chat.id != allowed_chat {
                        warn!(chat_id = %msg.chat.id, "ignoring message from unauthorized chat");
                        return Ok(());
                    }

                    let Some(text) = msg.text() else {
                        return Ok(());
                    };

                    let response = match Command::parse(text, "irrigator") {
                        Ok(cmd) => handle_command(cmd, &state, &valve, &flow).await,
                        Err(_) => "Unknown command. Send /help for available commands.".to_string(),
                    };

                    if let Err(e) = bot.send_message(msg.chat.id, &response).await {
                        warn!(error = %e, "failed to send telegram reply");
                    }

                    Ok(())
                }
            })
            .await;
        })
        .await;

        match result {
            Ok(()) => {
                warn!("telegram repl exited unexpectedly, restarting in {backoff_secs}s");
            }
            Err(e) => {
                warn!(
                    error = %e,
                    backoff = backoff_secs,
                    "telegram repl panicked, retrying in {backoff_secs}s"
                );
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF);
    }
}

async fn handle_command(
    cmd: Command,
    state: &Arc<Mutex<AppState>>,
    valve: &Arc<Mutex<Valve>>,
    flow: &Arc<Mutex<FlowSensor>>,
) -> String {
    match cmd {
        Command::On(args) => {
            let minutes: u32 = args.trim().parse().unwrap_or(10);
            let mut st = state.lock().await;

            if minutes == 0 {
                return "Duration must be > 0.".to_string();
            }
            if minutes > st.max_on_minutes {
                return format!("Max duration is {} minutes.", st.max_on_minutes);
            }

            flow.lock().await.start_session();
            valve.lock().await.open();
            st.valve_open = true;
            st.auto_off_at = Some(chrono::Utc::now() + chrono::Duration::minutes(minutes as i64));
            st.record_watering(minutes, "telegram", None);
            format!("Valve OPENED for {minutes} minutes.")
        }
        Command::Off => {
            let final_liters = flow.lock().await.session_liters();
            valve.lock().await.close();
            let mut st = state.lock().await;
            let had_valve_open = st.valve_open;
            st.valve_open = false;
            st.auto_off_at = None;
            if had_valve_open {
                st.update_last_watering_volume(final_liters);
            } else {
                st.save();
            }
            if had_valve_open && final_liters > 0.0 {
                format!("Valve CLOSED. Total: {final_liters:.1}L.")
            } else {
                "Valve CLOSED.".to_string()
            }
        }
        Command::Status => {
            let st = state.lock().await;
            let mut text = st.status_text();
            if st.valve_open {
                let sensor = flow.lock().await;
                text.push_str(&format!("\nFlow: {:.1}L", sensor.session_liters()));
            }
            text
        }
        Command::Schedule => {
            let st = state.lock().await;
            st.schedule_text()
        }
        Command::Set(args) => match parse_schedule(&args) {
            Ok(slots) => {
                let mut st = state.lock().await;
                let count = slots.len();
                st.schedule.slots = slots;
                st.save();
                format!("Schedule updated with {count} slot(s).")
            }
            Err(e) => format!("Parse error: {e}\nFormat: /set 06:00,8 10:00,8 14:00,8"),
        },
        Command::Enable => {
            let mut st = state.lock().await;
            st.schedule.enabled = true;
            st.save();
            "Schedule enabled.".to_string()
        }
        Command::Disable => {
            let mut st = state.lock().await;
            st.schedule.enabled = false;
            st.save();
            "Schedule disabled.".to_string()
        }
        Command::Help | Command::Start => Command::descriptions().to_string(),
    }
}

fn parse_schedule(input: &str) -> Result<Vec<crate::state::Slot>, String> {
    let mut slots = Vec::new();
    for part in input.split_whitespace() {
        let (time, dur) = part
            .split_once(',')
            .ok_or_else(|| format!("expected HH:MM,duration but got '{part}'"))?;
        let (h, m) = time
            .split_once(':')
            .ok_or_else(|| format!("expected HH:MM but got '{time}'"))?;
        let hour: u32 = h.parse().map_err(|_| format!("invalid hour: {h}"))?;
        let minute: u32 = m.parse().map_err(|_| format!("invalid minute: {m}"))?;
        let duration_min: u32 = dur.parse().map_err(|_| format!("invalid duration: {dur}"))?;

        if hour > 23 || minute > 59 {
            return Err(format!("invalid time: {time}"));
        }
        if duration_min == 0 || duration_min > 120 {
            return Err(format!("duration must be 1-120, got {duration_min}"));
        }

        slots.push(crate::state::Slot {
            hour,
            minute,
            duration_min,
        });
    }
    if slots.is_empty() {
        return Err("no slots provided".to_string());
    }
    slots.sort_by_key(|s| s.hour * 60 + s.minute);
    Ok(slots)
}
