use std::sync::Arc;

use teloxide::prelude::*;
use teloxide::utils::command::BotCommands;
use tokio::sync::Mutex;
use tracing::{info, warn};

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
) {
    info!("telegram bot starting...");
    let bot = Bot::new(bot_token);
    let allowed_chat = ChatId(chat_id);

    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let state = Arc::clone(&state);
        let valve = Arc::clone(&valve);
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
                Ok(cmd) => handle_command(cmd, &state, &valve).await,
                Err(_) => "Unknown command. Send /help for available commands.".to_string(),
            };

            if let Err(e) = bot.send_message(msg.chat.id, &response).await {
                warn!(error = %e, "failed to send telegram reply");
            }

            Ok(())
        }
    })
    .await;
}

async fn handle_command(
    cmd: Command,
    state: &Arc<Mutex<AppState>>,
    valve: &Arc<Mutex<Valve>>,
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

            valve.lock().await.open();
            st.valve_open = true;
            st.auto_off_at = Some(chrono::Utc::now() + chrono::Duration::minutes(minutes as i64));
            st.record_watering(minutes, "telegram");
            format!("Valve OPENED for {minutes} minutes.")
        }
        Command::Off => {
            valve.lock().await.close();
            let mut st = state.lock().await;
            st.valve_open = false;
            st.auto_off_at = None;
            st.save();
            "Valve CLOSED.".to_string()
        }
        Command::Status => {
            let st = state.lock().await;
            st.status_text()
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
