# Prior art

Before writing any code, three existing free tools that switch Windows virtual desktops
with the mouse were evaluated against this project's hard requirements:

1. Works with **any mouse**, regardless of brand or extra buttons.
2. Installs and runs **without Administrator privileges**.
3. No additional runtime or redistributable to install.
4. Actively maintained, with a clear license.

Evaluation date: 2026-08-23. Test machine: Windows 10 Pro 22H2 (build 19045), x64.

## Summary

| Tool | Trigger | Tech | License | Latest release | Verdict |
| --- | --- | --- | --- | --- | --- |
| [GestureWheel](https://github.com/iodes/GestureWheel) | Wheel button held + horizontal drag | C# / WPF, `net8.0-windows` | MIT | 1.1.0.0 (2022-08-15) | Rejected: needs the .NET Desktop Runtime |
| [BetterMouse](https://github.com/mwenku/BetterMouse) | Mouse button 5 (XBUTTON2) + movement | C++ | MIT | 1.7.6 (2025-03-12) | Rejected: side button hardcoded |
| [right-button-ninja](https://github.com/hansenwangvip/right-button-ninja) | Right button held + swipe | AutoHotkey | none detected | 1.0.1 (2019-07-14) | Rejected: unlicensed, stale, AutoHotkey binary |

None of them satisfies all four requirements, so this project was written.

## GestureWheel

Closest match in terms of gesture design: hold the wheel button, drag left or right.

Rejected because its project file targets `net8.0-windows` with `<UseWPF>true</UseWPF>`,
so it needs `Microsoft.WindowsDesktop.App` at runtime. On the test machine only
`Microsoft.NETCore.App` and `Microsoft.AspNetCore.App` are present, and installing the
.NET Desktop Runtime machine-wide requires Administrator privileges. That is exactly the
constraint this project exists to work around.

Secondary concerns:

- The published release (1.1.0.0, August 2022) is older than the source in the default
  branch (1.2.0.0), so the binary users download is not the maintained code.
- The only release asset is an Inno Setup installer, with no portable archive.
- Seven third-party NuGet dependencies for a tool of this size.

A self-contained build from source would clear the runtime problem, but that means
maintaining a fork, which is not less work than the utility itself.

## BetterMouse

Rejected because the trigger is hardcoded to `XBUTTON2` (mouse button 5) in
`LowLevelMouseProc`, with no configuration file. Mice without side buttons cannot use it
at all, which fails requirement 1.

Other observations from reading `main.cpp` (496 lines, read in full, not executed):

- While the gesture is active it calls `SystemParametersInfo(SPI_SETMOUSESPEED, ...)` to
  slow the pointer system-wide, then restores it on button release. If the process dies
  mid-gesture the user is left with a permanently altered mouse speed.
- It warps the cursor back to its starting position after every gesture.
- `MOVEMENT_THRESHOLD` is 2 pixels, which is far too twitchy to coexist with normal use
  of that button.
- It requires the Visual C++ 2015-2022 redistributable, another admin-level install.
- The repository has 3 stars and the binary is unsigned, which is a meaningful
  supply-chain consideration for a tool that installs a global input hook.

Useful confirmation: it switches desktops with `keybd_event(VK_LWIN)` +
`keybd_event(VK_CONTROL)` + arrow key, which is the same documented, version-stable
approach this project uses. See [Design decisions confirmed by this review](#design-decisions-confirmed-by-this-review) below.

## right-button-ninja

Rejected because GitHub detects no license despite the README claiming MIT, the last
commit is from January 2023, the README states Windows 10 compatibility only, and the
release ships a compiled AutoHotkey executable. AutoHotkey binaries are frequently
flagged by antivirus and EDR products, which is disqualifying for a corporate laptop.

The right mouse button also has to suppress and later replay the context menu, which is
more fragile than using the wheel button.

## Design decisions confirmed by this review

- `SetWindowsHookEx(WH_MOUSE_LL, ...)` plus synthetic `Ctrl+Win+Arrow` input is the
  approach every one of these tools converged on. It needs no privileges and does not
  depend on undocumented interfaces.
- `IVirtualDesktopManagerInternal` would allow switching desktops directly, but Microsoft
  changes its GUIDs with nearly every major Windows update, silently breaking callers
  several times a year. This project does not use it.
- The trigger must be configurable, and the default must be a button that exists on
  essentially every mouse. The wheel button is that button.
- Swallowing the trigger button and replaying a real click when no gesture happened is
  what keeps the tool from stealing normal middle-click behaviour. None of the three
  tools reviewed does this well.

## Also considered

- **PowerToys** has an open feature request for exactly this
  ([microsoft/PowerToys#45714](https://github.com/microsoft/PowerToys/issues/45714)),
  unimplemented as of this evaluation.
- **Vendor drivers** (Logitech Options, Razer Synapse and similar) solve the problem by
  remapping buttons to `Ctrl+Win+Arrow`, but every one of them requires Administrator
  privileges to install, which is the situation this project exists to handle.
