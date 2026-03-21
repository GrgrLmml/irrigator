# Irrigator

Raspberry Pi-based garden irrigation controller with a Telegram bot interface. Built for a 10m x 2.5m driveway lawn seeding project — keeps freshly seeded lawn moist during the ~3 week germination period.

The system runs on a Raspberry Pi 3B with LTE cellular connectivity (no WiFi at the deployment site), controlled remotely via Telegram or SSH over Tailscale VPN.

## How It Works

A solenoid valve controls water flow from an outdoor tap through two soaker hoses. The Pi toggles the valve via a relay on GPIO 17. You control everything through Telegram:

```
/on 10       — open valve for 10 minutes
/off         — close valve
/status      — valve state, uptime, next scheduled watering
/schedule    — show current schedule
/set 06:00,8 10:00,8 14:00,8 18:00,8 — set watering times
/enable      — enable schedule
/disable     — disable schedule
/help        — list commands
```

A scheduler runs watering slots automatically (default: 4x daily, 8 minutes each). Every valve open has a max duration timer (default 120 min) to prevent flooding if connectivity is lost.

## Hardware

| Component | Model |
|---|---|
| Computer | Raspberry Pi 3 Model B |
| LTE Dongle | ZTE MF833U1 |
| Relay | AZDelivery KY-019 5V 1-channel |
| Solenoid Valve | 12V DC NC 3/4" DN20 |
| Sprinklers | 2x Gardena Perl-Regner 15m soaker hose |

### Wiring

```
[5V/3A USB Charger] → [Raspberry Pi 3B]
                          │
                     GPIO 17 (Pin 11) → Relay S (signal)
                     3.3V  (Pin 1)  → Relay + (VCC)
                     GND   (Pin 6)  → Relay - (GND) + 12V PSU GND (common ground!)

[12V 2A PSU] → Relay COM
               Relay NO → Solenoid Valve (+)
                          Solenoid Valve (−) → 12V PSU GND

Add 1N4007 flyback diode across solenoid terminals (cathode to +12V side).
```

### GPIO Pinout

The `1` on the board marks Pin 1. Odd pins are on the inner row (closer to the board center), even pins on the outer row (closer to the board edge).

```
J8 Header (top view, active USB ports on the right):

   3V3  (1) (2)  5V
 GPIO2  (3) (4)  5V
 GPIO3  (5) (6)  GND
 GPIO4  (7) (8)  GPIO14
   GND  (9) (10) GPIO15
GPIO17 (11) (12) GPIO18
GPIO27 (13) (14) GND
GPIO22 (15) (16) GPIO23
   3V3 (17) (18) GPIO24
GPIO10 (19) (20) GND
 GPIO9 (21) (22) GPIO25
GPIO11 (23) (24) GPIO8
   GND (25) (26) GPIO7
 GPIO0 (27) (28) GPIO1
 GPIO5 (29) (30) GND
 GPIO6 (31) (32) GPIO12
GPIO13 (33) (34) GND
GPIO19 (35) (36) GPIO16
GPIO26 (37) (38) GPIO20
   GND (39) (40) GPIO21
```

**Relay connections** (KY-019 has a 3-pin header with S, +, -):

| Relay Pin | Pi Pin | Pi Label | Location |
|---|---|---|---|
| **+** (VCC) | Pin 1 | 3V3 | 1st row, inner |
| **-** (GND) | Pin 6 | GND | 3rd row, outer |
| **S** (Signal) | Pin 11 | GPIO17 | 6th row, inner |

## Pi Setup

### 1. Flash OS

Flash **Raspberry Pi OS Lite (64-bit)** with Raspberry Pi Imager. Enable SSH, set hostname to `irrigator`, set username/password.

### 2. Strip Desktop Packages

The Pi only needs a terminal. Remove X server and desktop apps to free memory:

```bash
sudo apt purge -y 'xserver-*' 'x11-*' chromium firefox vlc 'vlc-*' \
  realvnc-vnc-server rpi-imager rpi-userguide 'rpd-wallpaper*' \
  'libgtk-3*' 'mesa-*' lxpanel lxappearance lxterminal pcmanfm \
  raspberrypi-ui-mods rpd-plym-splash
sudo apt autoremove -y && sudo apt clean
```

Disable auto-login:
```bash
sudo raspi-config nonint do_boot_behaviour B1
```

### 3. LTE Modem

Plug the ZTE MF833U1 into a USB port. It appears as a CDC Ethernet device:

```bash
# Wait for it to enumerate, then get an IP
sudo dhcpcd eth1

# Verify connectivity
ping -I eth1 1.1.1.1
```

The modem exposes an admin panel at `http://192.168.0.1` (accessible from the Pi). Its HTTP API requires a `Referer: http://192.168.0.1/index.html` header:

```bash
curl -s 'http://192.168.0.1/goform/goform_get_cmd_process?isTest=false&cmd=signalbar,network_type,network_provider,ppp_status&multi_data=1' \
  -H 'Referer: http://192.168.0.1/index.html'
```

### 4. Tailscale

Provides stable remote access over LTE without opening ports:

```bash
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up --ssh
# Open the printed URL in your browser to authenticate
```

Install Tailscale on your laptop/phone too. Then SSH via: `ssh irrigator@<tailscale-ip>`

### 5. Telegram Bot

1. Message [@BotFather](https://t.me/botfather) on Telegram
2. `/newbot` → choose a name and username
3. Copy the bot token
4. Send a message to your bot, then get your chat ID:
   ```
   curl -s https://api.telegram.org/bot<TOKEN>/getUpdates | jq '.result[0].message.chat.id'
   ```
5. Create `.env` on the Pi (or set environment variables):
   ```
   TELEGRAM_BOT_TOKEN=your-token-here
   TELEGRAM_CHAT_ID=your-chat-id
   ```

## Build & Deploy

### Cross-compile (from your dev machine)

```bash
# Install cross if needed
cargo install cross

# Build for Pi (ARMv7)
cross build --release --target armv7-unknown-linux-gnueabihf

# Deploy
scp target/armv7-unknown-linux-gnueabihf/release/irrigator irrigator@<tailscale-ip>:/usr/local/bin/
```

### Systemd Service

Create `/etc/systemd/system/irrigator.service` on the Pi:

```ini
[Unit]
Description=Irrigation Controller
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/irrigator
Restart=always
RestartSec=5
User=root
EnvironmentFile=/etc/irrigator/.env
ExecStopPost=/bin/sh -c 'echo 17 > /sys/class/gpio/export 2>/dev/null; echo out > /sys/class/gpio/gpio17/direction; echo 0 > /sys/class/gpio/gpio17/value'

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable irrigator
sudo systemctl start irrigator
```

## Safety

- **Auto-off timer**: every valve open has a max duration (default 120 min)
- **Forced close on startup**: valve is always OFF when the service starts
- **Forced close on shutdown**: SIGTERM/SIGINT closes the valve before exit
- **Systemd ExecStopPost**: forces GPIO low even if the process crashes
- **Owner-only**: Telegram commands only accepted from configured chat ID

## Development

The project compiles on macOS/Linux for development — GPIO calls are stubbed out on non-Linux platforms. Only `cargo check` / `cargo build` needed locally; use `cross` for the actual ARM binary.

```bash
# Local development
cargo check
cargo run  # runs with stub GPIO, needs TELEGRAM_BOT_TOKEN and TELEGRAM_CHAT_ID

# ARM build
cross build --release --target armv7-unknown-linux-gnueabihf
```

## License

MIT
