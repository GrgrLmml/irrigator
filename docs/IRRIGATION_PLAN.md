# Irrigation Controller — Implementation Plan

## Project Overview

Build a remotely-controlled garden irrigation system for a 10m × 2.5m driveway lawn seeding project. The system runs on a Raspberry Pi 3B with LTE cellular connectivity (no WiFi available at site), controlled via SSH/REST API over Tailscale VPN, with SMS as fallback. The primary goal is to keep freshly seeded lawn moist for ~3 weeks during germination.

## Hardware Inventory (confirmed available)

| Component | Model / Spec | Notes |
|---|---|---|
| Computer | Raspberry Pi 3 Model B V1.2 | BCM2837 quad-core, 1GB RAM, 4× USB-A, GPIO header soldered |
| LTE Dongle | ZTE MF833U1 | CDC Ethernet mode, plug-and-play on Linux, shows as `usb0` or `eth1` |
| Power (Pi) | Schwaiger 5V/3A USB charger | USB-A to micro-USB cable to Pi PWR IN |
| Power (Valve) | 12V 2A DC PSU | Separate PSU for solenoid valve circuit |
| Relay | AZDelivery KY-019 5V 1-channel | High-level trigger, opto-isolated, 3.3V GPIO compatible |
| Solenoid Valve | 12V DC NC ¾" DN20 (Jenngaoo) | 300mA draw, 0.02-0.8 MPa, plastic + brass |
| Soil Sensor (future) | 2× ESP32-C3 Super Mini + capacitive sensor | ESP-NOW wireless, not yet purchased — design software to accept sensor data later |
| SIM Card | Prepaid (Aldi Talk or Congstar) | Minimal data needed, Telekom network preferred for rural coverage |
| Sprinkler | 2× Gardena 1969 Perl-Regner 15m soaker hose | Connected via Y-splitter to valve output |
| Adapters | 2× Gardena Geräteadapter 26.5mm (¾") | Screw onto solenoid valve threads |
| Enclosure | IP65 junction box | For Pi + relay, outdoor mounting near tap |

## Wiring Diagram

```
230V Mains
    │
    ├──→ [5V/3A USB Charger] ──micro-USB──→ [Raspberry Pi 3B]
    │                                            │
    │                                       GPIO 17 (Pin 11)
    │                                            │
    │                                       [KY-019 Relay]
    │                                        IN  VCC  GND
    │                                        │   │    │
    │                                        │   3.3V GND ← Pi GPIO
    │                                        │
    └──→ [12V 2A PSU] ──→ Relay COM          │
              │            Relay NO ──→ [Solenoid Valve +]
              │                         [Solenoid Valve -] ──→ PSU GND
              │
              └── IMPORTANT: Connect Pi GND to 12V PSU GND (common ground)

    [Pi USB-A port] ──→ [ZTE MF833U1 LTE Dongle]

    Solenoid Valve ──→ Gardena adapter ──→ Y-splitter ──→ 2× soaker hoses
```

## GPIO Pin Assignments

| BCM Pin | Physical Pin | Function |
|---|---|---|
| GPIO 17 | Pin 11 | Relay IN (signal) |
| 3.3V | Pin 1 | Relay VCC |
| GND | Pin 6 | Relay GND + common ground bus to 12V PSU GND |

### Future expansion pins (reserve, don't use):
- GPIO 2 (Pin 3) / GPIO 3 (Pin 5): I²C SDA/SCL for ADS1115 ADC (soil sensor)
- GPIO 27 (Pin 13): Second relay for zone 2
- GPIO 22 (Pin 15): Third relay for zone 3
- GPIO 4 (Pin 7): DS18B20 temperature sensor (frost detection)

## Software Architecture

### Decision: Rust or Python
The owner will decide. The plan below is language-agnostic where possible, with notes for both.

**If Rust:**
- Framework: Axum for REST API
- GPIO: `rppal` crate
- Serial/AT commands: `serialport` crate
- Cross-compile: `cross` with target `armv7-unknown-linux-gnueabihf` (Pi 3B is ARMv7)
- Binary size: ~2-5MB, no runtime dependencies
- Deploy: `scp` binary to Pi, run via systemd

**If Python:**
- Framework: FastAPI + uvicorn
- GPIO: `RPi.GPIO` or `gpiozero`
- Serial: `pyserial`
- Deploy: venv on Pi, systemd service

### System Components

```
┌─────────────────────────────────────────────────────┐
│                    irrigator                         │
│                                                     │
│  ┌──────────┐  ┌──────────┐  ┌───────────────────┐ │
│  │ REST API │  │ Schedule │  │ SMS Handler       │ │
│  │ (Axum/  │  │ Runner   │  │ (optional,        │ │
│  │ FastAPI) │  │          │  │  via ZTE HTTP API)│ │
│  └────┬─────┘  └────┬─────┘  └────────┬──────────┘ │
│       │              │                 │             │
│       └──────┬───────┘                 │             │
│              │                         │             │
│       ┌──────▼──────┐                  │             │
│       │  Command    │◄─────────────────┘             │
│       │  Processor  │                                │
│       └──────┬──────┘                                │
│              │                                       │
│       ┌──────▼──────┐  ┌──────────────┐             │
│       │   Valve     │  │  State       │             │
│       │  Controller │  │  Manager     │             │
│       │  (GPIO 17)  │  │  (JSON file) │             │
│       └─────────────┘  └──────────────┘             │
│                                                     │
│  ┌──────────────────────────────────────────┐       │
│  │  Sensor Receiver (future)                │       │
│  │  - Serial USB listener for ESP32         │       │
│  │  - Moisture readings → schedule override │       │
│  └──────────────────────────────────────────┘       │
└─────────────────────────────────────────────────────┘
```

### REST API Endpoints

```
GET  /api/status          → valve state, uptime, schedule, last watering, moisture
POST /api/valve/open      → { "minutes": 10 }
POST /api/valve/close     → {}
GET  /api/schedule        → current schedule
POST /api/schedule        → { "slots": [{"hour":6,"minute":0,"duration_min":8}, ...], "enabled": true }
POST /api/schedule/enable → {}
POST /api/schedule/disable→ {}
POST /api/threshold       → { "value": 40 }  (moisture % above which watering is skipped)
GET  /api/log             → last 50 watering events
POST /api/command         → { "command": "STATUS" }  (text command interface, same as SMS)
```

### Configuration File

Location: `/etc/irrigator/config.json`

```json
{
  "relay_pin": 17,
  "relay_active_high": true,
  "max_on_minutes": 120,
  "owner_phone": "+49170XXXXXXX",
  "moisture_enabled": false,
  "moisture_threshold": 40,
  "sms_enabled": true,
  "api_port": 8080,
  "api_host": "0.0.0.0"
}
```

### State File

Location: `/etc/irrigator/state.json`

Persisted across restarts. Contains:
- Current valve state (open/closed)
- Auto-off timestamp if valve is open
- Schedule slots and enabled flag
- Moisture threshold
- Boot timestamp
- Last 50 watering events log

### Default Schedule (germination mode)

```json
{
  "slots": [
    {"hour": 6, "minute": 0, "duration_min": 8},
    {"hour": 10, "minute": 0, "duration_min": 8},
    {"hour": 14, "minute": 0, "duration_min": 8},
    {"hour": 18, "minute": 0, "duration_min": 8}
  ],
  "enabled": true
}
```

4× daily, 8 minutes each. Keeps top 1-2cm of soil moist without pooling. Adjust via API once grass establishes (reduce frequency, increase duration).

### SMS Fallback (via ZTE HTTP API)

The ZTE MF833 in HiLink mode exposes an HTTP API at `http://192.168.0.1` (or whatever gateway IP it assigns). This can be used for SMS send/receive without serial port access:

- Poll `GET /api/sms/sms-list` for incoming messages
- Send via `POST /api/sms/send-sms` with XML body
- This is the fallback if Tailscale/SSH is unreachable

SMS Commands (same as REST but via text):
```
ON 10          → valve on for 10 minutes
OFF            → valve off
STATUS         → get current state
SCHEDULE       → show schedule
SCHEDULE OFF   → disable schedule
HELP           → list commands
```

### Safety Features

1. **Auto-off timer**: Every valve open command has a max duration. Default 120 min. Prevents flooding if connection is lost.
2. **Watchdog**: If the service crashes, systemd restarts it. On restart, valve is forced closed.
3. **Startup state**: Valve is always OFF on boot. Never assume previous state.
4. **Owner phone filter**: SMS commands only accepted from configured phone number.
5. **Max duration cap**: API rejects valve open requests > max_on_minutes.

## Deployment Steps

### Phase 1: Pi OS Setup (at home, with keyboard/screen or headless)

```bash
# 1. Flash Raspberry Pi OS Lite (64-bit) to microSD
#    Use Raspberry Pi Imager
#    In settings: enable SSH, set username/password, set hostname to "irrigator"

# 2. Boot Pi, connect to home WiFi or Ethernet for initial setup

# 3. Update system
sudo apt update && sudo apt upgrade -y

# 4. Install essentials
sudo apt install -y git curl build-essential

# 5. Install Tailscale
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up --ssh
# Follow the auth URL, approve the device in Tailscale admin

# 6. Verify SSH works over Tailscale from your laptop
# ssh pi@irrigator  (or use Tailscale IP)

# 7. Disable Pi WiFi and Bluetooth (won't be used in field)
# This saves ~0.5W power
sudo rfkill block wifi
sudo rfkill block bluetooth
```

### Phase 2: LTE Dongle Setup

```bash
# 1. Plug ZTE MF833U1 into Pi USB port
# 2. Wait 30 seconds for it to enumerate

# 3. Check if it appears as a network interface
ip addr show
# Should see usb0 or eth1 with a 192.168.x.x address

# 4. Test connectivity
ping -c 3 1.1.1.1

# 5. If not auto-configured, set up DHCP on the interface
sudo dhclient usb0  # or eth1, whatever the interface name is

# 6. Verify Tailscale works over LTE
# From your laptop: ssh pi@irrigator

# 7. ZTE admin panel (optional, for SMS API):
# http://192.168.0.1 (or check gateway: ip route show)
```

### Phase 3: GPIO + Relay Test

```bash
# Quick test: toggle GPIO 17 manually
# If Python:
python3 -c "
import RPi.GPIO as GPIO
import time
GPIO.setmode(GPIO.BCM)
GPIO.setup(17, GPIO.OUT)
GPIO.output(17, GPIO.HIGH)  # relay ON
time.sleep(2)
GPIO.output(17, GPIO.LOW)   # relay OFF
GPIO.cleanup()
print('Relay toggled successfully')
"

# Listen for the relay click. If you hear two clicks (on + off), wiring is correct.
```

### Phase 4: Software Deployment

```bash
# If Rust: cross-compile on dev machine, scp to Pi
cross build --release --target armv7-unknown-linux-gnueabihf
scp target/armv7-unknown-linux-gnueabihf/release/irrigator pi@irrigator:/usr/local/bin/

# If Python: set up on Pi directly
sudo mkdir -p /opt/irrigator
# copy source files
sudo python3 -m venv /opt/irrigator/venv
/opt/irrigator/venv/bin/pip install fastapi uvicorn RPi.GPIO pyserial
```

### Phase 5: Systemd Service

Create `/etc/systemd/system/irrigator.service`:

```ini
[Unit]
Description=Irrigation Controller
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
# Rust:
ExecStart=/usr/local/bin/irrigator
# Python:
# ExecStart=/opt/irrigator/venv/bin/python /opt/irrigator/irrigator.py
Restart=always
RestartSec=5
User=root
Environment=RUST_LOG=info

# Safety: force valve off on stop/crash
ExecStopPost=/bin/sh -c 'echo 17 > /sys/class/gpio/export 2>/dev/null; echo out > /sys/class/gpio/gpio17/direction; echo 0 > /sys/class/gpio/gpio17/value'

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable irrigator
sudo systemctl start irrigator
sudo systemctl status irrigator
```

### Phase 6: Field Deployment

1. Mount IP65 junction box near outdoor tap
2. Place Pi + relay + LTE dongle inside box
3. Connect 5V charger to Pi via micro-USB
4. Connect 12V PSU to relay COM terminal
5. Connect relay NO to solenoid valve (+)
6. Connect solenoid valve (-) to 12V PSU GND
7. Connect Pi GND (Pin 6) to 12V PSU GND (common ground)
8. Add 1N4007 flyback diode across solenoid terminals (cathode to +12V)
9. Insert SIM card into ZTE dongle
10. Plug ZTE dongle into Pi USB
11. Connect solenoid valve to tap via Gardena adapters
12. Connect Y-splitter to valve output
13. Lay 2× soaker hoses in serpentine across 10m × 2.5m driveway
14. Power on, verify SSH access via Tailscale
15. Verify `curl http://localhost:8080/api/status` returns valid JSON
16. Test: `curl -X POST http://localhost:8080/api/valve/open -d '{"minutes":1}'`
17. Confirm water flows through soaker hoses
18. Confirm auto-off after 1 minute

### Phase 7: Monitoring

```bash
# From your laptop/phone, via Tailscale:
ssh pi@irrigator
curl http://irrigator:8080/api/status
curl http://irrigator:8080/api/log

# Set up a simple cron health check (on Pi):
# Every hour, log status to file
*/60 * * * * curl -s http://localhost:8080/api/status >> /var/log/irrigator-health.log
```

## File Structure

### If Rust (Cargo workspace):

```
irrigator/
├── Cargo.toml
├── src/
│   ├── main.rs              # Entry point, signal handlers, startup
│   ├── config.rs            # Config loading from /etc/irrigator/config.json
│   ├── state.rs             # State persistence (JSON file read/write)
│   ├── valve.rs             # GPIO control via rppal, auto-off timer
│   ├── api.rs               # Axum REST API routes
│   ├── scheduler.rs         # Schedule runner (async loop, checks time + moisture)
│   ├── commands.rs          # Text command processor (shared by API + SMS)
│   ├── sms.rs               # SMS handler via ZTE HTTP API (reqwest)
│   └── sensor.rs            # Future: ESP-NOW serial receiver
├── Cross.toml               # Cross-compilation config
└── README.md
```

### If Python:

```
irrigator/
├── irrigator.py             # Main entry point, all-in-one
├── requirements.txt         # fastapi, uvicorn, RPi.GPIO, pyserial, requests
├── config.json              # Default config template
├── dashboard/
│   └── index.html           # Optional: simple web dashboard
└── README.md
```

## Testing Checklist

- [ ] Pi boots headless, SSH accessible over home WiFi
- [ ] LTE dongle provides internet, Pi gets IP on usb0/eth1
- [ ] Tailscale connected, SSH works over LTE from remote device
- [ ] GPIO 17 toggles relay (audible click test)
- [ ] Relay switches 12V circuit (measure with multimeter across valve terminals)
- [ ] Solenoid valve opens when relay activates (water flows)
- [ ] Solenoid valve closes when relay deactivates (water stops)
- [ ] REST API responds on port 8080
- [ ] POST /api/valve/open activates valve
- [ ] POST /api/valve/close deactivates valve
- [ ] Auto-off timer works (open for 1 min, confirm auto-close)
- [ ] Schedule fires at configured time
- [ ] Service survives reboot (systemd auto-start)
- [ ] Service restarts after crash (kill -9, verify restart)
- [ ] Valve is OFF after service restart
- [ ] SMS send/receive works via ZTE API (if enabled)
- [ ] Status endpoint returns all expected fields
- [ ] Watering log records events

## Notes for Claude Code Agent

- **Target platform**: Raspberry Pi 3 Model B, Raspberry Pi OS Lite 64-bit (Debian Bookworm)
- **Cross-compilation target** (Rust): `armv7-unknown-linux-gnueabihf`
- **GPIO library** (Rust): `rppal` — requires running as root or adding user to `gpio` group
- **No WiFi at deployment site**: All remote access is via LTE dongle + Tailscale
- **The relay is active-high**: GPIO HIGH = relay energized = valve open
- **Common ground is critical**: Pi GND must be connected to 12V PSU GND for relay signal to work
- **Flyback diode**: 1N4007 across solenoid, cathode to +12V side. Without this, relay contacts will arc and degrade.
- **State persistence**: Write state to JSON file on every change. Read on startup. Never assume valve state — always force OFF on boot.
- **Graceful shutdown**: On SIGTERM/SIGINT, close valve, cleanup GPIO, then exit.
- **Log to both file and stdout**: systemd captures stdout to journal, but also write to `/var/log/irrigator.log` for persistence.
- **The ZTE MF833 HTTP API base URL**: Check `ip route show default` to find gateway IP (likely 192.168.0.1 or 192.168.8.1). SMS API paths are similar to Huawei HiLink but may differ — test and document.
- **Future sensor integration**: Design the command processor and scheduler to accept moisture readings as input. Use a trait/interface so the source (serial, HTTP, mock) can be swapped. The ESP32 sensor node will send readings via USB serial as JSON lines: `{"moisture_pct": 42, "battery_v": 3.8, "rssi": -45}`
