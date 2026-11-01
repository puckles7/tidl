# tidl Outlook COM Bridge — Installation

## Prerequisites

- Windows 10/11 with Outlook installed and configured
- A running local tidl instance on the work machine
- An app token from tidl Settings → App Tokens (write access required)

## Steps

### 1. Place the files

Copy `bridge.ps1` and `bridge.conf.example` to a permanent location, e.g.:

```
C:\Users\<you>\tidl-bridge\
```

### 2. Configure

Copy `bridge.conf.example` to `bridge.conf` and edit it:

```
TIDL_URL=http://localhost:5000
TIDL_USER=alice
TIDL_TOKEN=tidl_<your-token>
TIDL_CALENDAR=work
OUTLOOK_CALENDAR=Calendar
```

The `TIDL_CALENDAR` must already exist in tidl. Create it via Settings → Calendars.
If you want to write-lock it so only the bridge can modify it, set it as the
`sync_user` calendar via Settings → Remote Sources after adding it.

### 3. Test manually

Open PowerShell and run:

```powershell
powershell.exe -ExecutionPolicy Bypass -File "C:\Users\<you>\tidl-bridge\bridge.ps1"
```

Check the log at `%APPDATA%\tidl-bridge\bridge.log` for any errors.

### 4. Schedule with Task Scheduler

Open Task Scheduler and create a new task:

**General tab**
- Name: `tidl Outlook Bridge`
- Run only when user is logged on
- Do not store password

**Triggers tab**
- New trigger: At log on (this user)
- Repeat task every: 10 minutes, for a duration of indefinitely

**Actions tab**
- Action: Start a program
- Program: `powershell.exe`
- Arguments: `-ExecutionPolicy Bypass -NonInteractive -WindowStyle Hidden -File "C:\Users\<you>\tidl-bridge\bridge.ps1"`

**Conditions tab**
- Uncheck "Start only if on AC power" (if on a laptop)

**Settings tab**
- Allow task to be run on demand: yes
- If the task is already running: Do not start a new instance

### 5. Verify

After the next scheduled run, check:
- `%APPDATA%\tidl-bridge\bridge.log` — should show PUT/DELETE counts
- tidl calendar view — work events should appear in the "work" calendar

## Troubleshooting

| Symptom | Likely cause |
|---------|-------------|
| "Outlook is not running" in log | Outlook was not open when the task ran — normal if machine was locked |
| `HTTP 401` errors | Token is wrong or expired — regenerate in tidl Settings → App Tokens |
| `HTTP 403` errors | Token is read-only, or calendar write-lock rejects this user |
| `HTTP 409` errors | Target calendar does not exist in tidl — create it first |
| `HTTP 412` errors | ETag mismatch on PUT — transient, resolves on next run |
| Calendar folder not found | `OUTLOOK_CALENDAR` name doesn't match — check Outlook folder name exactly |
