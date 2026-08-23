# mouse-desktop

Switch Windows virtual desktops with a mouse gesture. Hold the wheel button, drag left or
right, and Windows moves to the neighbouring desktop.

No Administrator privileges, no runtime to install, no driver, no service. One 270 KB
executable that works with any mouse of any brand.

## Why this exists

Windows already changes virtual desktop with `Ctrl+Win+Left` and `Ctrl+Win+Right`, but
that means taking a hand off the mouse. Vendor software such as Logitech Options or Razer
Synapse can remap a mouse button to that shortcut, and it is genuinely pleasant to use.

The problem is that every one of those tools needs an Administrator account to install. On
a managed corporate laptop there is no such account, so the comfortable way of working
disappears along with it. This utility gives it back, using only what a standard user is
allowed to do.

Several free tools already do something similar. They were evaluated first, and why none
of them fitted is written up in [docs/prior-art.md](docs/prior-art.md).

## What it does

- Hold the wheel button and drag horizontally to move one desktop per 60 pixels.
- Keep dragging in the same direction to walk across several desktops in one motion.
- Press and release the wheel button without dragging and the click is passed on intact,
  so middle click keeps closing browser tabs.
- Drag vertically and nothing happens, so the gesture stays out of the way.

The trigger button, the distance, the direction and the rest are configurable.

## Requirements

- Windows 10 version 21H2 or later, or Windows 11.
- A mouse with a wheel button, or any of the other configurable triggers.
- Nothing else. The executable links only against DLLs that ship with Windows.

## Installing

Everything below works from an ordinary user account. Nothing here asks for elevation, and
if a User Account Control prompt ever appears, something is wrong: stop and read the
troubleshooting table.

### Step 1: check the machine will run it at all

On a managed machine, do this before anything else. It takes a minute and tells you
whether the rest is worth attempting.

```powershell
New-Item -ItemType Directory -Force "$env:LOCALAPPDATA\Programs\mouse-desktop" | Out-Null
Copy-Item C:\Windows\System32\notepad.exe "$env:LOCALAPPDATA\Programs\mouse-desktop\probe.exe"
Start-Process "$env:LOCALAPPDATA\Programs\mouse-desktop\probe.exe"
```

If Notepad opens, the folder is allowed to run programs and this utility will work. Close
it and delete `probe.exe`.

If instead you get a message about your organisation's policy, an application control
policy is blocking user-writable folders. See the troubleshooting table below.

### Step 2: get the executable

Pick one:

- **From a release.** Download `mouse-desktop-x64.exe` from the
  [releases page](https://github.com/fpinero/mouse-desktop/releases). Use the `arm64`
  build only on an Arm laptop such as a Snapdragon X machine; the x64 build also runs
  there under emulation.
- **Build it yourself**, on a machine where you can install Rust. This is the option that
  lets you know exactly what you are running:

  ```powershell
  cargo build --release
  ```

  The result is `target\release\mouse-desktop.exe`, about 270 KB. Copy that one file to
  the target machine, by memory stick, OneDrive, or however files normally reach it. No
  build tools are needed on the machine that runs it.

### Step 3: install it

The script does the whole job:

```powershell
.\scripts\install.ps1 -Source .\mouse-desktop.exe
```

It copies the executable to `%LOCALAPPDATA%\Programs\mouse-desktop`, adds a start-up entry
under `HKEY_CURRENT_USER`, and starts it. Add `-NoAutostart` or `-NoStart` to skip either
part.

If PowerShell refuses to run the script because of the execution policy, either allow it
for the current session only:

```powershell
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass
```

or skip the script entirely and do it by hand, which is the same three actions:

```powershell
$dir = "$env:LOCALAPPDATA\Programs\mouse-desktop"
New-Item -ItemType Directory -Force $dir | Out-Null
Copy-Item .\mouse-desktop.exe "$dir\mouse-desktop.exe" -Force
New-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' `
    -Name 'mouse-desktop' -Value "`"$dir\mouse-desktop.exe`"" -PropertyType String -Force
Start-Process "$dir\mouse-desktop.exe"
```

The first time the file runs, Windows SmartScreen may show a blue "Windows protected your
PC" panel because the executable is not signed. Choose "More info", then "Run anyway".

### Step 4: check it works

1. An icon of two blue arrows appears in the notification area. On Windows 11 it may be
   hidden behind the chevron next to the clock; drag it onto the taskbar to keep it in
   sight.
2. Create a second virtual desktop with `Ctrl+Win+D`.
3. Hold the wheel button and drag about 60 pixels to the left. The desktop should change.
4. Middle click a browser tab. It should close, which confirms that a click without a drag
   is still passed through.
5. Right click the tray icon and confirm "Start with Windows" is ticked.
6. Sign out and back in. The icon should come back on its own.

### Portable, with no installation at all

Put `mouse-desktop.exe` anywhere, including a memory stick, and run it. To keep the
configuration beside the executable rather than in your profile, create an empty
`config.toml` in the same folder. The utility fills it in on the next start and reads it
from there, so nothing is left behind on the machine.

### Updating

Run `install.ps1` again with the new executable. It stops the running copy first, so there
is nothing else to do. Your configuration file is not touched.

### Removing

```powershell
.\scripts\uninstall.ps1
```

This stops the utility, removes the start-up entry and deletes the folder. Add
`-RemoveConfig` to delete the configuration too.

### Troubleshooting

| What you see | What it means | What to do |
| --- | --- | --- |
| "This app has been blocked by your system administrator" | AppLocker or App Control is blocking user-writable folders | Ask IT to allow the file by hash, or to name a folder you may run programs from, and install there |
| SmartScreen panel on first run | The executable is unsigned | "More info", then "Run anyway". Building it yourself avoids the question |
| The script will not run | PowerShell execution policy | `Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass`, or use the manual commands above |
| No icon in the notification area | It may simply be hidden | Click the chevron next to the clock and drag the icon onto the taskbar |
| The gesture works everywhere except one window | That window belongs to an elevated process | Expected, see the known limitations |
| Nothing happens at all | Another copy may already be running | Check with `Get-Process mouse-desktop`, and start it with `--verbose` to get a log next to the configuration file |

## Configuration

The file lives at `%APPDATA%\mouse-desktop\config.toml` and is created with comments on
the first run. Open it from the tray menu, edit it, then choose "Reload configuration".

| Setting | Default | Meaning |
| --- | --- | --- |
| `trigger` | `"middle"` | Button to hold: `middle`, `right`, `x1` or `x2` |
| `threshold_px` | `60` | Horizontal distance that moves one desktop |
| `repeat_cooldown_ms` | `250` | Shortest delay between two switches |
| `max_vertical_ratio` | `1.0` | How far from horizontal a drag may stray and still count |
| `invert` | `false` | Swap the direction of travel |
| `replay_click_when_no_gesture` | `true` | Pass the click on when there was no drag |
| `enabled` | `true` | Start with gesture recognition switched on |

`x1` and `x2` are the two side buttons found on most mice, usually mapped to back and
forward. Choosing one of them as the trigger makes the gesture feel closest to a vendor
button remap.

## Known limitations

**Elevated windows.** While the window in front belongs to a process running as
Administrator, such as an elevated terminal, Task Manager started as administrator, or the
UAC prompt itself, the gesture does nothing. Windows deliberately hides input from
lower-privileged processes through User Interface Privilege Isolation, and there is no way
around it that does not require the Administrator account this utility exists to avoid.

**No wrap around.** On the first or the last desktop, dragging further does nothing. That
is how `Ctrl+Win+Arrow` behaves too.

**Unsigned executable.** The binary carries no code signing certificate, so Windows
SmartScreen warns the first time it runs, and a managed laptop with AppLocker or App
Control policies may refuse to run it from a user-writable folder at all. If that happens,
the options are to have IT allow it by hash or to place it in a folder the policy permits.
Building it yourself from this repository is the surest way to know what you are running.

**Corporate policy.** Nothing here bypasses a policy. It only avoids needing rights a
standard user does not have.

## How it works

A global low level mouse hook, `SetWindowsHookEx(WH_MOUSE_LL, ...)`, watches for the
trigger button being held. When the pointer has travelled far enough horizontally, the
utility synthesises `Ctrl+Win+Left` or `Ctrl+Win+Right` with `SendInput`. Neither call
needs any privileges.

Switching could also be done through `IVirtualDesktopManagerInternal`, a COM interface
that changes desktops directly. This project does not use it: Microsoft changes its GUIDs
with nearly every major Windows update, which silently breaks callers several times a
year. Synthesising the documented shortcut is slower by a few milliseconds and survives
Windows updates.

The trigger button is hidden from other applications while it is held, so a drag cannot
also open a context menu or start an autoscroll. If the button is released without a drag,
a real click is injected at the pointer, which is what keeps ordinary clicking with that
button working.

## Building and testing

```powershell
cargo build --release
cargo test                          # gesture, configuration and icon logic
cargo clippy --all-targets -- -D warnings
cargo test -- --ignored --nocapture # end to end, needs an interactive session
```

The end to end test starts the real executable, synthesises a wheel drag, and checks
through the documented `IVirtualDesktopManager` interface that Windows actually changed
desktop. It creates two scratch desktops for the measurement and closes both afterwards.
It takes over the mouse pointer for about five seconds, so do not run it while you are
using the machine.

Stop any installed copy before running it. The utility refuses to start twice, so the copy
the test launches would put up a dialog instead of installing its hook. The test checks
for this and says so rather than reporting a broken gesture.

Manual check, worth doing after any change to the hook:

1. Create three virtual desktops with `Ctrl+Win+D`.
2. Start the utility and confirm the icon in the notification area.
3. Hold the wheel button and drag right over the desktop, over Explorer and over a
   browser. Each should move one desktop forward.
4. Keep dragging without releasing. It should keep moving forward.
5. Middle click a browser tab. It should close, which proves the click was passed on.
6. Hold the wheel button and drag vertically. Nothing should happen.

## License

MIT. See [LICENSE](LICENSE).
