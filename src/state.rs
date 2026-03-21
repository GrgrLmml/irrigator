use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};

const DEFAULT_STATE_PATH: &str = "./state.json";
const SYSTEM_STATE_PATH: &str = "/etc/irrigator/state.json";
const MAX_LOG_ENTRIES: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Slot {
    pub hour: u32,
    pub minute: u32,
    pub duration_min: u32,
}

impl std::fmt::Display for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:02}:{:02} for {}min", self.hour, self.minute, self.duration_min)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub slots: Vec<Slot>,
    pub enabled: bool,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            slots: vec![
                Slot { hour: 6, minute: 0, duration_min: 8 },
                Slot { hour: 10, minute: 0, duration_min: 8 },
                Slot { hour: 14, minute: 0, duration_min: 8 },
                Slot { hour: 18, minute: 0, duration_min: 8 },
            ],
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WateringEvent {
    pub timestamp: DateTime<Utc>,
    pub duration_min: u32,
    pub source: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AppState {
    pub valve_open: bool,
    pub auto_off_at: Option<DateTime<Utc>>,
    pub schedule: Schedule,
    pub max_on_minutes: u32,
    pub relay_pin: u8,
    pub log: Vec<WateringEvent>,
    pub boot_time: DateTime<Utc>,

    #[serde(skip)]
    state_path: PathBuf,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            valve_open: false,
            auto_off_at: None,
            schedule: Schedule::default(),
            max_on_minutes: 120,
            relay_pin: 17,
            log: Vec::new(),
            boot_time: Utc::now(),
            state_path: PathBuf::from(DEFAULT_STATE_PATH),
        }
    }
}

impl AppState {
    pub fn load() -> Self {
        let path = if Path::new(SYSTEM_STATE_PATH).exists() {
            PathBuf::from(SYSTEM_STATE_PATH)
        } else {
            PathBuf::from(DEFAULT_STATE_PATH)
        };

        let mut state = if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(contents) => {
                    match serde_json::from_str::<AppState>(&contents) {
                        Ok(s) => {
                            tracing::info!("loaded state from {}", path.display());
                            s
                        }
                        Err(e) => {
                            tracing::warn!("failed to parse state file: {e}, using defaults");
                            AppState::default()
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("failed to read state file: {e}, using defaults");
                    AppState::default()
                }
            }
        } else {
            tracing::info!("no state file found, using defaults");
            AppState::default()
        };

        // Always reset valve state on load — we force it off on startup.
        state.valve_open = false;
        state.auto_off_at = None;
        state.boot_time = Utc::now();
        state.state_path = path;
        state
    }

    pub fn save(&self) {
        if let Some(parent) = self.state_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&self.state_path, json) {
                    tracing::error!("failed to save state: {e}");
                }
            }
            Err(e) => tracing::error!("failed to serialize state: {e}"),
        }
    }

    pub fn record_watering(&mut self, duration_min: u32, source: &str) {
        self.log.push(WateringEvent {
            timestamp: Utc::now(),
            duration_min,
            source: source.to_string(),
        });
        if self.log.len() > MAX_LOG_ENTRIES {
            self.log.drain(0..self.log.len() - MAX_LOG_ENTRIES);
        }
        self.save();
    }

    pub fn status_text(&self) -> String {
        let valve = if self.valve_open { "OPEN" } else { "CLOSED" };
        let auto_off = self
            .auto_off_at
            .map(|t| {
                let remaining = t - Utc::now();
                format!("{}min remaining", remaining.num_minutes())
            })
            .unwrap_or_else(|| "—".to_string());

        let uptime = Utc::now() - self.boot_time;
        let local_now = Local::now();

        let next = if self.schedule.enabled {
            self.next_scheduled_text()
        } else {
            "schedule disabled".to_string()
        };

        format!(
            "Valve: {valve}\n\
             Auto-off: {auto_off}\n\
             Schedule: {}\n\
             Next: {next}\n\
             Uptime: {}h {}m\n\
             Time: {}\n\
             Last watering: {}",
            if self.schedule.enabled { "enabled" } else { "disabled" },
            uptime.num_hours(),
            uptime.num_minutes() % 60,
            local_now.format("%H:%M %Z"),
            self.log
                .last()
                .map(|e| format!("{} ({}min, {})", e.timestamp.with_timezone(&Local).format("%H:%M"), e.duration_min, e.source))
                .unwrap_or_else(|| "none".to_string()),
        )
    }

    pub fn schedule_text(&self) -> String {
        if self.schedule.slots.is_empty() {
            return "No schedule configured.".to_string();
        }
        let mut text = format!(
            "Schedule ({})\n",
            if self.schedule.enabled { "enabled" } else { "disabled" }
        );
        for (i, slot) in self.schedule.slots.iter().enumerate() {
            text.push_str(&format!("  {}. {}\n", i + 1, slot));
        }
        text
    }

    fn next_scheduled_text(&self) -> String {
        let now = Local::now();
        let current_minutes = now.hour() * 60 + now.minute();

        // Find next slot today or tomorrow
        let mut next: Option<&Slot> = None;
        for slot in &self.schedule.slots {
            let slot_minutes = slot.hour * 60 + slot.minute;
            if slot_minutes > current_minutes {
                next = Some(slot);
                break;
            }
        }
        match next {
            Some(slot) => format!("{:02}:{:02} today", slot.hour, slot.minute),
            None => self
                .schedule
                .slots
                .first()
                .map(|s| format!("{:02}:{:02} tomorrow", s.hour, s.minute))
                .unwrap_or_else(|| "none".to_string()),
        }
    }
}

use chrono::Timelike;
