#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
//! Meeting Recorder: records a meeting in two tracks (mic and computer
//! audio), transcribes it with whisper.cpp after the call, and streams live
//! levels to a bar widget.

mod actions;
mod agent;
mod animation;
mod audio;
mod bar_widget;
mod chapters;
mod diarize;
mod export;
mod ipc;
mod meeting;
mod models;
mod nemotron;
mod platform;
mod player;
mod settings;
mod theme;
mod transcribe;
mod ui;

use gtk::glib;

pub const APP_ID: &str = "com.jankeesvw.OmarchyMeetingRecorder";
pub const APP_NAME: &str = "omarchy-meeting-recorder";

/// Whether `arg` opens a meeting: a `.meeting-recorder` file or a meeting
/// folder. The extension matches case-insensitively: the Windows shell
/// association is not case-sensitive, so `Weekly.MEETING-RECORDER` must open
/// instead of falling into unknown-command.
fn is_meeting_path(arg: &str) -> bool {
    std::path::Path::new(arg)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(meeting::EXTENSION))
        || std::path::Path::new(arg).is_dir()
}

// Rattache la console du processus parent (terminal/pipes CLI) quand elle
// existe. Double-clic GUI : aucune console créée (windows_subsystem).
// Erreurs ignorées silencieusement : pas de log, pas de panic.
#[cfg(windows)]
fn attach_parent_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

fn main() -> glib::ExitCode {
    #[cfg(windows)]
    attach_parent_console();
    match std::env::args().nth(1).as_deref() {
        None => ui::run(None),
        Some("--version" | "-V") => {
            let backend = if cfg!(feature = "vulkan") {
                "Vulkan support; CPU fallback"
            } else {
                "CPU-only"
            };
            println!("{APP_NAME} {} ({backend})", env!("CARGO_PKG_VERSION"));
            glib::ExitCode::SUCCESS
        }
        Some("watch") => {
            ipc::watch();
            glib::ExitCode::SUCCESS
        }
        Some(command @ ("start" | "stop" | "compact" | "pause")) => {
            let line = if command == "start" {
                ipc::start_line(&std::env::args().skip(2).collect::<Vec<_>>())
            } else {
                command.to_owned()
            };
            if ipc::send(&line) {
                glib::ExitCode::SUCCESS
            } else if command == "start" {
                // Not running: open the recorder, and start as soon as it listens.
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        if ipc::send(&line) {
                            return;
                        }
                    }
                });
                ui::run(None)
            } else {
                eprintln!("{APP_NAME}: the recorder is not running");
                glib::ExitCode::FAILURE
            }
        }
        Some("transcribe-file") => {
            transcribe::cli_file(&std::env::args().skip(2).collect::<Vec<_>>())
        }
        Some("diarize") => diarize::cli(&std::env::args().skip(2).collect::<Vec<_>>()),
        Some("transcribe") => transcribe::cli(&std::env::args().skip(2).collect::<Vec<_>>()),
        Some("action") => actions::cli(&std::env::args().skip(2).collect::<Vec<_>>()),
        // A new window in the running app, or the app itself when it is not running.
        Some("new-window") => {
            if ipc::send("new-window") {
                glib::ExitCode::SUCCESS
            } else {
                ui::run(None)
            }
        }
        Some("ask") => agent::cli(&std::env::args().skip(2).collect::<Vec<_>>()),
        Some("-h" | "--help") => {
            println!(
                "Usage: {APP_NAME} [start [name] | stop | pause | compact | watch | transcribe <mic> <computer> [--language xx]]"
            );
            println!();
            println!("  (no command)  open the recorder, ready to record");
            println!("  <meeting>     open a .meeting-recorder file or a meeting folder");
            println!("  start [name]  start recording, opening the recorder if needed");
            println!("  stop          stop the running recording (for a keybinding)");
            println!("  compact       switch the recording window between full and compact");
            println!("  pause         pause or resume the running recording");
            println!("  watch         stream the recorder state as NDJSON, for the bar widget");
            println!("  transcribe    transcribe two tracks and print the transcript as Markdown");
            println!(
                "  ask           run a prompt over stdin through the default agent, without tools"
            );
            println!("  action        run one of your actions on a meeting folder");
            println!(
                "  new-window    open another window, for a second meeting (Ctrl+N in the app)"
            );
            glib::ExitCode::SUCCESS
        }
        Some(path) if is_meeting_path(path) => ui::run(Some(path)),
        Some(other) => {
            eprintln!("{APP_NAME}: unknown command '{other}', see --help");
            glib::ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_meeting_path;

    #[test]
    fn meeting_file_matches_case_insensitively() {
        assert!(is_meeting_path("Weekly.meeting-recorder"));
        assert!(is_meeting_path("Weekly.MEETING-RECORDER"));
        assert!(is_meeting_path("Weekly.Meeting-Recorder"));
        assert!(!is_meeting_path("Weekly.txt"));
        assert!(!is_meeting_path("start"));
    }

    #[test]
    fn meeting_folder_still_opens() {
        assert!(is_meeting_path(&std::env::temp_dir().to_string_lossy()));
    }
}
