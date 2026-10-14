//! Actions: your own scripts, run on a finished meeting from the done page.
//!
//! They live in `config.toml`, each with a name for the menu and a command:
//!
//! ```toml
//! [[action]]
//! name = "Store transcript in Obsidian"
//! command = "~/.local/bin/meeting-to-obsidian"
//! ```
//!
//! The command runs through `sh -c` in the meeting folder (`cmd /C` on
//! Windows), with that folder as `$1` (Unix only) and the meeting described
//! in `MEETING_*` variables. What it prints last
//! is shown when it is done; a link there (web or `obsidian://`) goes behind an
//! Open button instead. An action may change the meeting itself: when it
//! edited the transcript or the meeting file, the done page reads them again.

use std::collections::VecDeque;
use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use crate::meeting::Manifest;
use crate::platform::silent_command;

use crate::action_process as process;

/// How actions work, for people and their agents; the done page links here
/// when there are none yet.
#[cfg(target_os = "windows")]
pub const DOCS: &str =
    "https://github.com/Rhadamanthe0/meeting-recorder-windows/blob/master/docs/actions-windows.md";
#[cfg(not(target_os = "windows"))]
pub const DOCS: &str =
    "https://github.com/jankeesvw/omarchy-meeting-recorder/blob/main/docs/actions.md";

#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub name: String,
    pub command: String,
}

/// The actions in the config file, in its order. Read every time, so an edit
/// shows up without restarting the app.
pub fn load() -> Vec<Action> {
    std::fs::read_to_string(crate::models::config_file())
        .map(|text| parse(&text))
        .unwrap_or_default()
}

/// The `[[action]]` tables of a config file. A line format rather than a full
/// TOML parser: `key = "value"` pairs, `#` comments, nothing nested.
fn parse(text: &str) -> Vec<Action> {
    let mut actions = Vec::new();
    let mut current: Option<(String, String)> = None;
    let mut finish = |current: &mut Option<(String, String)>| {
        if let Some((name, command)) = current.take()
            && !name.is_empty()
            && !command.is_empty()
        {
            actions.push(Action { name, command });
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            finish(&mut current);
            if line == "[[action]]" {
                current = Some((String::new(), String::new()));
            }
            continue;
        }
        let Some((name, command)) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = unquote(value.trim());
        match key.trim() {
            "name" => *name = value,
            "command" => *command = value,
            _ => {}
        }
    }
    finish(&mut current);
    actions
}

/// A TOML string: `"..."` with backslash escapes, or `'...'` taken literally.
pub(crate) fn unquote(value: &str) -> String {
    if let Some(inner) = value.strip_prefix('\'').and_then(|v| v.split('\'').next()) {
        return inner.to_owned();
    }
    let Some(rest) = value.strip_prefix('"') else {
        return value.split('#').next().unwrap_or("").trim().to_owned();
    };
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => break,
            },
            c => out.push(c),
        }
    }
    out
}

/// How an action ended: the last thing it printed, and a link in it if any.
pub struct Outcome {
    pub message: String,
    pub url: Option<String>,
}

/// How long a user action may run before it is killed. Actions may run a
/// long time (they can call out to scripts and services), so this is roomy:
/// only a truly hung action hits it.
const ACTION_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Enough output to preserve useful diagnostics without letting a noisy action
/// consume memory without bound.
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

fn drain_bounded(mut stream: impl Read) -> Vec<u8> {
    let mut output = VecDeque::with_capacity(MAX_OUTPUT_BYTES);
    let mut chunk = [0; 8192];
    while let Ok(size) = stream.read(&mut chunk) {
        if size == 0 {
            break;
        }
        if size >= MAX_OUTPUT_BYTES {
            output.clear();
            output.extend(&chunk[size - MAX_OUTPUT_BYTES..size]);
            continue;
        }
        let overflow = output
            .len()
            .saturating_add(size)
            .saturating_sub(MAX_OUTPUT_BYTES);
        if overflow > 0 {
            output.drain(..overflow);
        }
        output.extend(&chunk[..size]);
    }
    output.into_iter().collect()
}

/// Runs `action` on the meeting in `dir`. Blocking; the caller runs it off the
/// main thread.
pub fn run(action: &Action, dir: &Path, manifest: &Manifest) -> Result<Outcome, String> {
    run_with_timeout(action, dir, manifest, ACTION_TIMEOUT)
}

fn run_with_timeout(
    action: &Action,
    dir: &Path,
    manifest: &Manifest,
    timeout: Duration,
) -> Result<Outcome, String> {
    // The shell changes its working directory. Resolve paths first so
    // MEETING_* and Unix $1 still refer to this meeting for relative CLI input.
    let dir = std::fs::canonicalize(dir).map_err(|e| e.to_string())?;
    let dir = dir.as_path();
    let date = gtk::glib::DateTime::from_unix_local(manifest.started_at)
        .and_then(|t| t.format("%Y-%m-%d %H:%M"))
        .map(|s| s.to_string())
        .unwrap_or_default();
    let audio = ["audio.ogg", "mic.ogg", "computer.ogg"]
        .iter()
        .map(|f| dir.join(f))
        .find(|p| p.exists());
    #[cfg(not(target_os = "windows"))]
    let mut shell = silent_command("sh");
    #[cfg(target_os = "windows")]
    let mut shell = silent_command("cmd");
    #[cfg(target_os = "windows")]
    {
        shell.arg("/C").arg(&action.command);
    }
    #[cfg(not(target_os = "windows"))]
    {
        shell
            .arg("-c")
            .arg(&action.command)
            .arg("meeting-action")
            .arg(dir);
    }
    shell
        .current_dir(dir)
        .env("MEETING_DIR", dir)
        .env("MEETING_TRANSCRIPT", dir.join("transcript.md"))
        .env(
            "MEETING_MANIFEST",
            crate::meeting::find(dir).unwrap_or_default(),
        )
        .env("MEETING_TITLE", &manifest.title)
        .env("MEETING_DATE", &date)
        .env("MEETING_STARTED_AT", manifest.started_at.to_string())
        .env("MEETING_DURATION", manifest.duration_secs.to_string())
        .env("MEETING_LANGUAGE", &manifest.language)
        .env("MEETING_SPEAKERS", manifest.people().join("\n"))
        .env("MEETING_AUDIO", audio.unwrap_or_default())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut process = process::ActionProcess::spawn(&mut shell).map_err(|e| e.to_string())?;
    let child = &mut process.child;
    // Drain both pipes while the action runs. Waiting for the child to exit
    // before reading can deadlock when either finite pipe buffer fills.
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let stdout_reader = std::thread::spawn(move || drain_bounded(stdout));
    let stderr_reader = std::thread::spawn(move || drain_bounded(stderr));

    // Poll rather than waiting without bound, so a hanging action cannot leak
    // this worker thread and leave the "running" toast on screen forever.
    let started = Instant::now();
    let mut status = None;
    let result = loop {
        if status.is_none() {
            match process.child.try_wait() {
                Ok(value) => status = value,
                Err(error) => break Err(error.to_string()),
            }
        }
        if let Some(status) = status
            && stdout_reader.is_finished()
            && stderr_reader.is_finished()
        {
            break Ok(status);
        }
        // Un descendant peut garder les pipes ouverts après la sortie du
        // shell. La même échéance couvre aussi cette attente de fin des flux.
        if started.elapsed() >= timeout {
            break Err(format!(
                "timed out after {} minutes",
                timeout.as_secs() / 60
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    process.terminate();
    let stdout = stdout_reader
        .join()
        .map_err(|_| "could not read action stdout".to_owned())?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| "could not read action stderr".to_owned())?;
    let status = result?;
    let stdout = String::from_utf8_lossy(&stdout);
    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr);
        let why = last_line(&stderr)
            .or_else(|| last_line(&stdout))
            .unwrap_or_else(|| format!("exited with {status}"));
        return Err(why);
    }
    let line = last_line(&stdout).unwrap_or_default();
    let url = find_url(&line);
    // The link goes behind the Open button, so the message is only the words.
    let message = match &url {
        Some(url) => line.replace(url.as_str(), ""),
        None => line,
    };
    let message = message.trim().trim_end_matches(':').trim().to_owned();
    Ok(Outcome {
        message: if message.is_empty() {
            "Done".to_owned()
        } else {
            message
        },
        url,
    })
}

/// `action "<name>" <meeting>`: runs one of your actions on a meeting folder
/// or `.meeting-recorder` file, exactly as the done page does, and prints how
/// it went. For trying an action out, by you or by your agent.
pub fn cli(args: &[String]) -> gtk::glib::ExitCode {
    use gtk::glib::ExitCode;
    let actions = load();
    let [name, meeting] = args else {
        eprintln!(
            "Usage: {} action \"<name>\" <meeting folder>",
            crate::APP_NAME
        );
        eprintln!();
        if actions.is_empty() {
            eprintln!(
                "No actions yet; add them to {}",
                crate::models::config_file().display()
            );
            eprintln!("See {DOCS}");
        } else {
            eprintln!("Actions in {}:", crate::models::config_file().display());
            for action in &actions {
                eprintln!("  {}", action.name);
            }
        }
        return ExitCode::from(2);
    };
    let Some(action) = actions.iter().find(|a| &a.name == name) else {
        eprintln!(
            "No action called \"{name}\" in {}",
            crate::models::config_file().display()
        );
        return ExitCode::FAILURE;
    };
    let Some((dir, manifest)) = crate::meeting::open(Path::new(meeting)) else {
        eprintln!("No meeting in {meeting}");
        return ExitCode::FAILURE;
    };
    match run(action, &dir, &manifest) {
        Ok(outcome) => {
            println!("{}", outcome.message);
            ExitCode::SUCCESS
        }
        Err(why) => {
            eprintln!("{} failed: {why}", action.name);
            ExitCode::FAILURE
        }
    }
}

/// When the transcript and the meeting file of `dir` last changed, and their
/// sizes: compared before and after an action to see whether it edited them.
pub fn fingerprint(dir: &Path) -> Vec<Option<(std::time::SystemTime, u64)>> {
    [Some(dir.join("transcript.md")), crate::meeting::find(dir)]
        .into_iter()
        .map(|path| {
            let meta = std::fs::metadata(path?).ok()?;
            Some((meta.modified().ok()?, meta.len()))
        })
        .collect()
}

fn last_line(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.chars().take(200).collect())
}

/// A link to open: a web page, or a note in Obsidian.
fn find_url(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|w| {
            ["https://", "http://", "obsidian://"]
                .iter()
                .any(|s| w.starts_with(s))
        })
        .map(|w| w.trim_end_matches(['.', ',', ')', '"', '\'']).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn descendant_fixture() {
        let Ok(mode) = std::env::var("OMR_ACTION_FIXTURE") else {
            return;
        };
        if mode == "leaf" {
            for _ in 0..3000 {
                println!("descendant stdout");
                eprintln!("descendant stderr");
                std::thread::sleep(Duration::from_millis(10));
            }
            return;
        }
        let mut child = silent_command(std::env::current_exe().unwrap())
            .args([
                "actions::tests::descendant_fixture",
                "--exact",
                "--nocapture",
            ])
            .env("OMR_ACTION_FIXTURE", "leaf")
            .spawn()
            .unwrap();
        std::fs::write("descendant.pid", child.id().to_string()).unwrap();
        // Reap a child that finishes early. In the "exited" fixture the
        // parent can still exit immediately, leaving its child alive:
        // the action's Job Object must clean it up.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        if mode == "running" {
            std::thread::sleep(Duration::from_secs(30));
        }
    }

    #[test]
    #[cfg(windows)]
    fn timeout_reaps_descendants_and_pipe_readers_even_after_shell_exit() {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        for mode in ["running", "exited"] {
            let dir = std::env::temp_dir().join(format!(
                "mr-action-descendants-{}-{mode}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("fixture.cmd"),
                format!(
                    "@echo off\r\nset OMR_ACTION_FIXTURE={mode}\r\n\"{}\" actions::tests::descendant_fixture --exact --nocapture\r\n",
                    std::env::current_exe().unwrap().display()
                ),
            )
            .unwrap();
            let manifest = Manifest {
                title: "Test".into(),
                started_at: 0,
                duration_secs: 0,
                format: crate::export::Format::Mono,
                language: "en".into(),
                speakers: Vec::new(),
                labels: Vec::new(),
                imported: None,
                speaker_count: None,
                model: None,
                chapters: Vec::new(),
                chapters_by: None,
            };
            let action = Action {
                name: "Descendant fixture".into(),
                command: "fixture.cmd".into(),
            };
            let started = Instant::now();
            let result = run_with_timeout(&action, &dir, &manifest, Duration::from_secs(2));
            let error = result.err().unwrap();
            assert!(error.starts_with("timed out"), "{mode}: {error}");
            assert!(started.elapsed() < Duration::from_secs(5));
            // run_with_timeout a joint les deux lecteurs avant de revenir.
            // Vérifier aussi l'arrêt du descendant, pas seulement des lecteurs.
            let pid = std::fs::read_to_string(dir.join("descendant.pid"))
                .unwrap()
                .parse()
                .unwrap();
            unsafe {
                let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if !handle.is_null() {
                    let mut code = 0;
                    let queried = GetExitCodeProcess(handle, &mut code);
                    CloseHandle(handle);
                    assert_ne!(queried, 0);
                    assert_ne!(code, STILL_ACTIVE as u32);
                }
            }
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn reads_actions_and_skips_the_rest() {
        let config = r#"
model = "large-v3-turbo"  # not an action

[[action]]
name = "Copy to Obsidian"
command = "~/.local/bin/meeting-to-obsidian"

[[action]]
name = 'Publish'   # a comment
command = 'publish "$1" --secret'

[[action]]
name = "No command, ignored"

[other]
name = "not an action"
"#;
        assert_eq!(
            parse(config),
            [
                Action {
                    name: "Copy to Obsidian".into(),
                    command: "~/.local/bin/meeting-to-obsidian".into(),
                },
                Action {
                    name: "Publish".into(),
                    command: "publish \"$1\" --secret".into(),
                },
            ]
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn a_link_goes_behind_the_button_and_out_of_the_message() {
        let dir = std::env::temp_dir().join(format!("mr-action-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = Manifest {
            title: "Weekly".into(),
            started_at: 0,
            duration_secs: 60,
            format: crate::export::Format::Mono,
            language: "en".into(),
            speakers: vec!["Maya".into(), "Tom".into()],
            labels: Vec::new(),
            imported: None,
            speaker_count: None,
            model: None,
            chapters: Vec::new(),
            chapters_by: None,
        };
        let action = |command: &str| Action {
            name: "Test".into(),
            command: command.into(),
        };
        let saved = run(
            &action("echo busy; echo Saved obsidian://open?file=Weekly"),
            &dir,
            &manifest,
        )
        .unwrap();
        assert_eq!(saved.message, "Saved");
        assert_eq!(saved.url.as_deref(), Some("obsidian://open?file=Weekly"));
        let named = run(
            &action(r#"echo "$MEETING_TITLE by $(echo "$MEETING_SPEAKERS" | head -1)""#),
            &dir,
            &manifest,
        )
        .unwrap();
        assert_eq!(named.message, "Weekly by Maya");
        assert!(named.url.is_none());
        // The shell runs inside the meeting: a relative argument must not
        // make MEETING_TRANSCRIPT resolve relative to that folder again.
        let relative = std::path::PathBuf::from("target")
            .join(format!("relative-action-{}", std::process::id()));
        std::fs::create_dir_all(&relative).unwrap();
        std::fs::write(relative.join("transcript.md"), "Relative meeting").unwrap();
        let read = run(&action("cat \"$MEETING_TRANSCRIPT\""), &relative, &manifest).unwrap();
        assert_eq!(read.message, "Relative meeting");
        std::fs::remove_dir_all(&relative).unwrap();
        let failed = run(&action("echo nope >&2; exit 3"), &dir, &manifest);
        assert_eq!(failed.err().as_deref(), Some("nope"));
        let noisy = run(
            &action("(yes o | head -c 2097152); (yes e | head -c 2097152 >&2); echo Finished"),
            &dir,
            &manifest,
        )
        .unwrap();
        assert_eq!(noisy.message, "Finished");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn a_link_goes_behind_the_button_and_out_of_the_message() {
        let dir = std::env::temp_dir().join(format!("mr-action-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = Manifest {
            title: "Weekly".into(),
            started_at: 0,
            duration_secs: 60,
            format: crate::export::Format::Mono,
            language: "en".into(),
            speakers: vec!["Maya".into(), "Tom".into()],
            labels: Vec::new(),
            imported: None,
            speaker_count: None,
            model: None,
            chapters: Vec::new(),
            chapters_by: None,
        };
        let action = |command: &str| Action {
            name: "Test".into(),
            command: command.into(),
        };
        let saved = run(
            &action("echo Saved obsidian://open?file=Weekly"),
            &dir,
            &manifest,
        )
        .unwrap();
        assert_eq!(saved.message, "Saved");
        assert_eq!(saved.url.as_deref(), Some("obsidian://open?file=Weekly"));
        let failed = run(&action("echo nope 1>&2 & exit /b 3"), &dir, &manifest);
        assert_eq!(failed.err().as_deref(), Some("nope"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_link_in_the_output_is_found() {
        assert_eq!(
            find_url("Published: https://gist.github.com/abc123."),
            Some("https://gist.github.com/abc123".into())
        );
        assert_eq!(
            find_url("Saved obsidian://open?vault=Writing&file=Meetings%2FWeekly"),
            Some("obsidian://open?vault=Writing&file=Meetings%2FWeekly".into())
        );
        assert_eq!(find_url("Copied to the vault"), None);
    }
}
