# Planky 🦀 - A Modern TUI Todo App

Planky is a fast, lightweight, and interactive terminal Todo app and Planka client built in Rust.  
It keeps your todos in sync with your Planka boards (boards = projects), with near‑realtime updates.
Manage your tasks visually with an intuitive TUI—add, list, mark as done, and delete todos—right from your terminal!
 
## Features

- [x] Interactive terminal UI (TUI)
- [x] Planka backend integration (boards treated as “projects”)
- [x] Near‑realtime background sync (pull) + offline‑safe outbound queue (push)
- [x] Add, edit (Enter), delete, copy/paste description (y/p)
- [x] Mark Doing (k) and Done (m); Doing highlighted in Cyan, Done in Green
- [x] Natural language due dates (relative, weekdays, absolute, combos)
- [x] Search (?), project switch ([ and ]), set project (l), login (L), sync (S)
- [x] Persistent storage and config in XDG config dir
- [x] Detailed HTTP debug logs in debug builds

## Planka Integration
- Boards are treated as “projects” in the UI. Use [ and ] to cycle.
- Lists are auto‑detected (case/spacing agnostic): Todo/Doing/Done.
- Sync:
  - Inbound: background poller updates todos from Planka.
  - Outbound: actions create/move/update/delete remote cards; failures are queued and retried.
- Press L to log in (URL → username → password). Press S to sync now.

---

## Download

Get the latest release from the [Releases page](https://github.com/KushalMeghani1644/Planky/releases).

| Platform   | Download                                              |
|------------|-------------------------------------------------------|
| Linux      | `Planky-v2.1.1.tar.gz`                      |
| Windows    | `Planky-v2.1.1(windows).zip`                       |
=======
| Linux      | `Planky-v2.1.1.tar.gz`                      |
| Windows    | `Planky-v2.1.1(windows).zip`                       |

---

## How to Use

### Run Pre-Built Binary

1. Download the release for your platform.  
2. Extract the archive:
```bash
# For Linux
tar -xzf Planky-v2.1.1-linux.tar.gz
>>>>>>> c931762491b113cfb94339e1ddbefe0f3d1ea14e
=======
tar -xzf Planky-v2.1.1-linux.tar.gz
>>>>>>> c931762491b113cfb94339e1ddbefe0f3d1ea14e

# For Windows
# Extract using your preferred archive manager (e.g., 7-Zip)

# Run
./Planky    # Linux
Planky.exe  # Windows
```

---

## TUI Interaction Guide

- Add: a → description → Enter → due date → Enter
- Navigate: ↑/↓
- Edit selected: Enter
- Mark Doing: k
- Mark Done: m
- Delete: d
- Copy/Paste description: y / p
- Search: ?
- Prev/Next project: [ / ]
- Set project: l
- Planka login: L
- Sync now: S
- Quit: q

---

## Smart Date & Time Parsing

Planky features an intelligent date parser that understands natural language! No need to remember complex date formats—just type what feels natural.

### Task Status Colors
- **Green**: Completed tasks (regardless of due date)
- **Red**: Overdue tasks (not completed and past due date)
- **Yellow**: Pending tasks (not completed but not overdue)

### Supported Date Formats

#### **Relative Times**
```
now                    # Right now with current time
today                  # Today (date only)
tomorrow, tmr          # Tomorrow
yesterday              # Yesterday
```

#### **Weekdays**
```
friday                 # Next Friday
monday                 # Next Monday
next friday            # Explicitly next Friday
this wednesday         # This Wednesday (if not passed)
```

#### **Weekdays with Time**
```
friday 15:30           # Next Friday at 3:30 PM
next monday 09:00      # Next Monday at 9:00 AM
this thursday 14:45    # This Thursday at 2:45 PM
```

#### **Time Offsets**
```
in 30 minutes          # 30 minutes from now
in 2 hours             # 2 hours from now
in 3 days              # 3 days from now
5 minutes              # 5 minutes from now (without "in")
2 hours 30 minutes     # Combined time periods
in 1 day 3 hours       # 1 day and 3 hours from now
```

#### **Specific Dates & Times**
```
2024-12-25             # Christmas Day 2024
12-25                  # December 25th (current year)
15:30                  # Today at 3:30 PM
9:00am                 # Today at 9:00 AM
11:45pm                # Today at 11:45 PM
2024-12-25 15:30       # Christmas Day 2024 at 3:30 PM
```

#### **Relative Periods**
```
week, next week        # Next week (7 days from now)
month, next month      # Next month (30 days from now)
year, next year        # Next year (365 days from now)
```

### Supported Time Units
- **Seconds**: `second`, `seconds`, `sec`, `s`
- **Minutes**: `minute`, `minutes`, `min`, `m`
- **Hours**: `hour`, `hours`, `hr`, `h`
- **Days**: `day`, `days`, `d`
- **Weeks**: `week`, `weeks`, `w`
- **Months**: `month`, `months` (30 days)
- **Years**: `year`, `years` (365 days)

### Examples in Action
```
# Quick tasks
"Call mom" → "in 2 hours"
"Weekly standup" → "monday 10:00am"
"Doctor appointment" → "friday 14:30"

# Project deadlines
"Submit report" → "next friday"
"Code review" → "in 3 days"
"Meeting prep" → "tomorrow 09:00"

# Long-term goals
"Vacation planning" → "next month"
"Annual review" → "2024-12-15"
```

---

## Build from Source

### Prerequisites
- Rust
- Git

### Steps
```bash
# Clone the repository
git clone https://github.com/Kushal_Meghani1644/Planky.git
cd Planky

# Build and run
cargo run

# Or build release binary
cargo build --release
# Binary will be in target/release/Planky
```

---

## Configuration

Planky stores your todos in a JSON file:
- Todos: `~/.config/Planky/todos.json` (uses XDG_CONFIG_HOME if set)
- Planka config: `~/.config/Planky/planka.json`
- Debug HTTP log (debug builds): `~/.config/Planky/planka_debug.log`
- Pending ops (offline queue): `~/.config/Planky/pending_ops.json`
- Windows equivalents use `%APPDATA%\Planky\...`

The file is created automatically on first run.

## Troubleshooting
- Enable a debug build to capture HTTP logs and stderr traces.
- Check `~/.config/Planky/planka_debug.log` for full request/response logs (secrets masked).
- If an action fails offline, it will appear in the title as “⇅N” and retry automatically.

---

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add some amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

---

## License

This project is licensed under the GPL-3.0 License — see the [LICENSE](LICENSE) file for details.

---

**Built with ❤️ in Rust** 🦀

**Shout-out to [Kivooeo](https://github.com/Kivooeo) for contributing to the code!**
