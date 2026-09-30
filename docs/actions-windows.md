# Actions on Windows

Your own scripts, run on a finished meeting from the done page.
This page describes exactly what the Windows build does
(`src/actions.rs`, `src/platform.rs`, `src/models.rs`).
The Unix page is different on purpose: keep both in sync only where the code agrees.

## Where actions live

In `%APPDATA%\omarchy-meeting-recorder\config.toml`
(code: `platform::config_dir()` → `dirs::config_dir()` on Windows,
plus `APP_NAME`/`config.toml` in `models::config_file()`),
re-read every time the menu opens, so an edit shows up without restarting:

```toml
[[action]]
name = "Show title"
command = "echo %MEETING_TITLE% (%MEETING_DATE%)"
```

The `[[action]]` parser is a line format, not full TOML:
`key = "value"` pairs (`"..."` with backslash escapes or `'...'` literal),
`#` comments, nothing nested. Entries without both `name` and `command` are ignored.

## How the command runs (Windows)

`actions::run()` builds the process as (Unix branch shown for contrast only):

- Windows: `cmd /C <command>` — literally
  `silent_command("cmd").arg("/C").arg(&action.command)`.
- Unix: `sh -c <command> meeting-action <dir>`.

Then, on both systems:

- working directory = the meeting folder (`.current_dir(dir)`),
- standard input = null,
- no console window (`platform::silent_command` sets `CREATE_NO_WINDOW`),
- environment = the `MEETING_*` variables below, inherited process environment included.

What the action prints and how it ends:

- success (exit 0): the last non-empty stdout line is shown on the done page;
  a link in it (`https://`, `http://`, `obsidian://`) goes behind an Open
  button and is removed from the message; an empty output shows "Done".
- failure (non-zero exit): the last non-empty stderr line, else the last
  stdout line, else `exited with <status>`.
- timeout: after 10 minutes, the action and its descendants are terminated,
  including descendants holding stdout or stderr open after `cmd` exits.
  The output readers finish before the timeout is reported.
- when the action edited `transcript.md` or the `.meeting-recorder` file,
  the done page reads them again (same `fingerprint` check as Unix).

Actions run inside a Windows Job Object. The shell starts suspended and is
assigned to the job before it runs, so descendants are contained from their
creation. Any remaining descendants are stopped when the action finishes;
background processes should be launched separately from the action runner.
Both output streams are drained concurrently, retaining at most their last
1 MiB each.

## The meeting folder is NOT passed as an argument

On Unix the folder arrives as `$1` (the `meeting-action` + `dir` extra args
to `sh -c`). **On Windows no extra argument is appended**: there is no `%1`
or `$1` to read. Use the working directory (`%CD%` in `cmd`,
`$PWD` / `Get-Location` in PowerShell) or `%MEETING_DIR%` instead.
When calling your own script file, pass what it needs explicitly:

```toml
[[action]]
name = "Archive meeting"
command = "C:\\Tools\\archive-meeting.cmd \"%MEETING_DIR%\" \"%MEETING_TRANSCRIPT%\""
```

## Variables

| Variable | Content (from `actions::run`) | `cmd` | PowerShell |
|---|---|---|---|
| `MEETING_DIR` | meeting folder path | `%MEETING_DIR%` | `$env:MEETING_DIR` |
| `MEETING_TRANSCRIPT` | `<dir>\transcript.md` | `%MEETING_TRANSCRIPT%` | `$env:MEETING_TRANSCRIPT` |
| `MEETING_MANIFEST` | path of the `.meeting-recorder` file in the folder (`meeting::find`), empty when none | `%MEETING_MANIFEST%` | `$env:MEETING_MANIFEST` |
| `MEETING_TITLE` | meeting title | `%MEETING_TITLE%` | `$env:MEETING_TITLE` |
| `MEETING_DATE` | local start, format `%Y-%m-%d %H:%M` | `%MEETING_DATE%` | `$env:MEETING_DATE` |
| `MEETING_STARTED_AT` | unix start time | `%MEETING_STARTED_AT%` | `$env:MEETING_STARTED_AT` |
| `MEETING_DURATION` | seconds | `%MEETING_DURATION%` | `$env:MEETING_DURATION` |
| `MEETING_LANGUAGE` | language code or `auto` | `%MEETING_LANGUAGE%` | `$env:MEETING_LANGUAGE` |
| `MEETING_SPEAKERS` | speaker names joined with newlines | `%MEETING_SPEAKERS%` | `$env:MEETING_SPEAKERS` |
| `MEETING_AUDIO` | first existing file among `audio.ogg`, `mic.ogg`, `computer.ogg`, else empty | `%MEETING_AUDIO%` | `$env:MEETING_AUDIO` |

Note: `MEETING_SPEAKERS` contains `\n` separators, which `cmd` does not split
into words; prefer PowerShell (`$env:MEETING_SPEAKERS -split "`n"`) when you
need the list.

## `.cmd` examples (run by `cmd /C`, working directory = meeting folder)

```toml
[[action]]
name = "List meeting files"
command = "dir /B"

[[action]]
name = "Copy transcript to Desktop"
command = "copy \"%MEETING_TRANSCRIPT%\" \"%USERPROFILE%\\Desktop\\\""
```

## `.ps1` example (PowerShell must be invoked explicitly — the runner is `cmd`)

`%VAR%` does not expand inside PowerShell; use `$env:VAR`:

```toml
[[action]]
name = "Count transcript words"
command = "powershell -NoProfile -ExecutionPolicy Bypass -Command \"(Get-Content $env:MEETING_TRANSCRIPT | Measure-Object -Word).Words\""
```

(Schemas checked against the runner by reading `actions::run`, not executed here.)

## Trying an action from a terminal

Same run as the done page, printing the outcome. The MSI adds neither PATH
nor App Paths, so use the installed full path:

```text
"%LOCALAPPDATA%\Programs\MeetingRecorder\meeting-recorder-windows.exe" action "<name>" <meeting folder or .meeting-recorder file>
```

## Differences vs Unix

- runner: `cmd /C` vs `sh -c`; write `.cmd`/`.ps1`, not `bash`, and quote
  Windows-style (`"..."`, `%VAR%` under `cmd`, `$env:VAR` under PowerShell).
- no `$1`: the meeting folder is the working directory + `%MEETING_DIR%`,
  never a positional argument.
- config file: `%APPDATA%\omarchy-meeting-recorder\config.toml`
  vs `~/.config/omarchy-meeting-recorder/config.toml`.
- helper binary: `"%LOCALAPPDATA%\Programs\MeetingRecorder\meeting-recorder-windows.exe"` vs `omarchy-meeting-recorder`.
- the "Add actions…" button on the done page opens this page
  (`actions::DOCS` on Windows, via `platform::open_uri` → `ShellExecuteW`);
  on Unix it opens the upstream `docs/actions.md`.

## Note on `examples/actions/*`

`examples/actions/store-in-obsidian` (shebang `#!/usr/bin/env python3`,
calls the `omarchy-meeting-recorder ask` binary, `OBSIDIAN_VAULT=~/...`)
and `examples/actions/publish-transcript` (`bash`, calls
`omarchy-meeting-recorder ask`, plus `gh`, `mktemp`, `sed`, `tail`)
assume a Unix setup and the Unix binary name: they do not run as-is on
Windows. Adapt them (Python via `py`, GitHub CLI for Windows, installed exe
`"%LOCALAPPDATA%\Programs\MeetingRecorder\meeting-recorder-windows.exe"`) instead of running them directly.
They are intentionally left untouched.
