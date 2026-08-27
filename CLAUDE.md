# CLAUDE.md

Project-specific instructions. These complement the global instructions in
`~/.claude/CLAUDE.md`.

## Project purpose

`mouse-desktop` is a Windows tray utility that switches virtual desktops with a mouse
gesture, so the user never has to move a hand to the keyboard for `Ctrl+Win+Arrow`.

It exists because of one specific situation: a corporate laptop where the user has no
Administrator account, so no vendor mouse driver (Logitech Options, Razer Synapse) can be
installed to remap buttons. The repository is public because that situation is common.

Default gesture: hold the wheel button (middle button) and drag horizontally. The wheel
button was chosen because it exists on essentially every mouse of every brand, which is a
hard requirement. Other triggers are configurable.

See `docs/prior-art.md` for the tools that were evaluated and rejected first.

## Hard constraints

These are not negotiable. A change that violates one of them is wrong even if it works.

1. **No Administrator privileges**, at install time or at run time. No services, no
   drivers, no writes outside the user profile, no `HKLM`, no elevation manifest.
2. **Documented Win32 APIs only.** `IVirtualDesktopManagerInternal` and every other
   undocumented virtual desktop interface is banned: Microsoft changes their GUIDs with
   nearly every major Windows update and breaks callers silently. Desktop switching is
   done by synthesising `Ctrl+Win+Left` / `Ctrl+Win+Right` with `SendInput`.
   The documented `IVirtualDesktopManager` is allowed, but only in tests.
3. **Windows 10 21H2 and later, and Windows 11** up to the current build. Development
   happens on Windows 10 build 19045; Windows 11 is the real deployment target, so any
   feature has to be verified on both before it is considered done.
4. **No runtime dependencies in the shipped binary.** No .NET runtime, no Visual C++
   redistributable, no DLLs next to the executable. One self-contained `.exe` that runs
   after being copied. This is why `crt-static` is set in `.cargo/config.toml`.
5. **Keep the dependency tree small.** Every crate added to `Cargo.toml` is a
   supply-chain liability in a process that installs a global input hook. Justify each
   one in the pull request.
6. **The release asset names are a contract with another repository.** The step named
   "Name the binary after its architecture" in `release.yml` copies each matrix build to
   `mouse-desktop-x64.exe` or `mouse-desktop-arm64.exe`.
   The product page at <https://mouse-desktop.nivelepsilon.com> links to them through
   `releases/latest/download/<name>`, which is the only form of URL that keeps pointing at
   the newest build without that page being rebuilt. Rename either asset and every download
   button there answers 404, silently: nothing in this repository would notice, and nobody
   reports a broken download, they just leave.
   `src/utils/releases.test.ts` in `web-mouse-desktop` spells both URLs out in full for
   exactly this reason, rather than composing them from the same constants the code uses.
   If a rename is genuinely needed, it is one change across two repositories.

## Build and verify commands

```
cargo build --release          # target\release\mouse-desktop.exe
cargo test                     # unit tests, must pass on every commit
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -- --ignored        # end-to-end tests, need an interactive session
```

The end to end test drives the real binary, so no installed copy may be running while it
does: the single instance guard would stop the copy it launches from installing a hook.
The test asserts this up front, and `Stop-Process -Name mouse-desktop` clears the way.

Never report a task as finished without running at least `cargo test` and `cargo clippy`.
For anything that touches the hook, the gesture logic or the key injection, also run the
manual smoke test in `README.md` before saying it works.

## Architecture rules

- `src/gesture.rs` holds the gesture state machine and **must not reference any Win32
  API**. It is pure logic over `RawEvent` in, `Verdict` out, so it can be unit tested
  without a mouse. Every behaviour change here needs a test.
- The `WH_MOUSE_LL` callback in `src/hook.rs` must stay in the low microseconds. It does
  arithmetic and `PostMessage`, nothing else. It must never call `SendInput`, allocate,
  lock a contended mutex, log to disk or block. Windows silently unhooks callbacks that
  exceed `LowLevelHooksTimeout` (default 300 ms).
- Discard every event whose `flags` contain `LLMHF_INJECTED` before it reaches the state
  machine, otherwise the clicks the utility replays are reprocessed as new gestures.
  The `--allow-injected` debug flag deliberately bypasses this so end-to-end tests can
  synthesise a gesture; it must never be the default.
- Actual input injection lives in `src/desktop.rs` and runs on the main thread, driven by
  messages posted from the hook.
- Keep the layering intact: `hook.rs` translates Win32 into `RawEvent`, `gesture.rs`
  decides, `desktop.rs` acts. Do not let Win32 types leak into `gesture.rs`.

## Windows gotchas

- **UIPI.** A low-level mouse hook installed by a non-elevated process is not called
  while the foreground window belongs to a higher integrity level process (an elevated
  terminal, Task Manager started as admin, the UAC dialog). The gesture will not work
  over those windows. This is Windows security behaviour, not a bug. Do not try to work
  around it, and keep it documented in `README.md`.
- Arrow keys are extended keys. `SendInput` needs `KEYEVENTF_EXTENDEDKEY` on them or the
  shortcut may not register.
- `Ctrl+Win+Arrow` does not wrap around. On the first or last desktop it is a no-op, and
  that is the intended behaviour here too.
- The desktop switch animation takes a few hundred milliseconds. Repeated switches need a
  cooldown or Windows drops them.
- Physically held modifier keys can combine with injected ones. Check `GetAsyncKeyState`
  for `VK_SHIFT` and `VK_MENU` and temporarily release them around the injection.
- A message-only window (`HWND_MESSAGE`) has no virtual desktop, so
  `IsWindowOnCurrentVirtualDesktop` fails on it. Tests need a real top-level window.
- The virtual desktop manager only tracks windows it would show in Task View. A `WS_POPUP`
  or tool window is invisible to it, and `IsWindowOnCurrentVirtualDesktop` then answers
  "yes" for every desktop, which quietly makes a test pass no matter what. Probe windows
  have to be visible and `WS_OVERLAPPEDWINDOW`.
- `SetCursorPos` warps the pointer without generating the events a low level mouse hook
  sees. Synthetic drags have to go through `SendInput` with `MOUSEEVENTF_ABSOLUTE`.
- `Ctrl+Win+D` appends the new desktop to the end of the list, not next to the current
  one, so a freshly created desktop is not necessarily adjacent to where you were.

## Language and style

- All code, comments, documentation, commit messages, UI strings, log lines and
  identifiers in **English**. The repository is public and international.
- Conversation with the repository owner happens in **Spanish**.
- Never use an em dash (U+2014). Use commas, parentheses or separate sentences.
- Headings in sentence case.
- No emojis anywhere, including `README.md` and tray menu entries.

## Git flow

Never commit directly to `main`. Use `feature/<name>` and `fix/<name>` branches created
with `git switch -c`. Do not commit or push unless explicitly asked.
