//! Live state for the bar widget.
//!
//! The app listens on a Unix socket in $XDG_RUNTIME_DIR and writes one JSON
//! line per tick to every connected client: 20 times a second while recording,
//! once a second otherwise. `omarchy-meeting-recorder watch` connects to it and
//! copies those lines to stdout, printing `{"state":"off"}` while the app is not
//! running, so the widget only has to read NDJSON from a process.
//!
//! A line looks like:
//! {"state":"recording","elapsed":754,"title":"Weekly","mic":0.62,"computer":0.31,"progress":0.0}
//! with `mic` and `computer` as meter levels from 0 to 1, and `progress` the
//! transcription progress from 0 to 1 while the state is "transcribing".
//!
//! Transport: a Unix socket in `$XDG_RUNTIME_DIR` on Linux, a named pipe
//! (`\\.\pipe\meeting-recorder-windows`, via the `interprocess` crate) on
//! Windows. The NDJSON protocol and the commands are identical on both.

#[cfg(target_os = "linux")]
use std::io::ErrorKind;
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(target_os = "linux")]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(target_os = "linux")]
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::APP_NAME;
use crate::audio::{Source, to_meter};

pub const MAX_LINE: usize = 4096;

#[derive(Clone, Default)]
pub struct Status {
    /// idle, recording, paused, stopping, transcribing or done
    pub state: &'static str,
    pub started_at: i64,
    /// Seconds spent paused so far, and when the current pause began (0: not paused).
    pub paused_secs: i64,
    pub pause_began: i64,
    pub title: String,
    pub progress: f64,
}

pub type SharedStatus = Arc<Mutex<Status>>;
/// One status per open window; the socket reports the busiest.
pub type Statuses = Arc<Mutex<Vec<SharedStatus>>>;

/// How much a window's state matters to the bar: the one recording first.
fn rank(state: &str) -> u8 {
    match state {
        "recording" | "paused" => 4,
        "stopping" => 3,
        "transcribing" => 2,
        "done" => 1,
        _ => 0,
    }
}

/// The status the bar widget shows: the busiest window's.
pub fn busiest(statuses: &Statuses) -> Status {
    statuses
        .lock()
        .unwrap()
        .iter()
        .map(|s| s.lock().unwrap().clone())
        .max_by_key(|s| rank(s.state))
        .unwrap_or(Status {
            state: "idle",
            ..Default::default()
        })
}

#[cfg(target_os = "linux")]
fn socket_path() -> PathBuf {
    crate::platform::runtime_dir().join(format!("{APP_NAME}.sock"))
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Commands a client may send, one per line. `start` may be followed by a
/// space and the meeting's name.
pub const COMMANDS: [&str; 5] = ["start", "stop", "compact", "pause", "new-window"];

/// A command from a client, and the name that came with `start` ("" without).
pub type Command = (&'static str, String);

/// Starts the socket server. Called once, from the primary instance. Clients
/// get the state lines of the busiest window; a line a client writes that
/// names one of `COMMANDS` is passed on to `commands`.
#[cfg(target_os = "linux")]
pub fn serve(
    statuses: Statuses,
    mic: Source,
    system: Source,
    commands: async_channel::Sender<Command>,
) {
    let path = socket_path();
    // A socket file left behind by a crash refuses new binds; nobody answers on it.
    if UnixStream::connect(&path).is_err() {
        let _ = std::fs::remove_file(&path);
    }
    let Ok(listener) = UnixListener::bind(&path) else {
        eprintln!("{APP_NAME}: could not listen on {}", path.display());
        return;
    };
    let clients: Arc<Mutex<Vec<UnixStream>>> = Arc::default();

    let accepted = clients.clone();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            // A write timeout rather than non-blocking mode: the flag would be
            // shared with the reading clone below.
            if stream
                .set_write_timeout(Some(Duration::from_millis(20)))
                .is_err()
            {
                continue;
            }
            if let Ok(reader) = stream.try_clone() {
                let commands = commands.clone();
                thread::spawn(move || read_commands(reader, &commands));
            }
            accepted.lock().unwrap().push(stream);
        }
    });

    thread::spawn(move || {
        loop {
            let snapshot = busiest(&statuses);
            let recording = snapshot.state == "recording";
            let taking = recording || snapshot.state == "paused";
            let until = if snapshot.pause_began > 0 {
                snapshot.pause_began
            } else {
                now()
            };
            let busy = recording || snapshot.state == "transcribing";
            let line = serde_json::json!({
                "state": snapshot.state,
                "elapsed": if taking { (until - snapshot.started_at - snapshot.paused_secs).max(0) } else { 0 },
                "title": snapshot.title,
                "mic": round(to_meter(mic.recent_peak(3))),
                "computer": round(to_meter(system.recent_peak(3))),
                "progress": round(snapshot.progress),
            })
            .to_string()
                + "\n";
            // A client that cannot keep up is dropped rather than waited for.
            clients
                .lock()
                .unwrap()
                .retain_mut(|client| match client.write_all(line.as_bytes()) {
                    Ok(()) => true,
                    Err(e) => e.kind() == ErrorKind::Interrupted,
                });
            thread::sleep(Duration::from_millis(if recording {
                50
            } else if busy {
                250
            } else {
                1000
            }));
        }
    });
}

fn read_commands(stream: impl Read, commands: &async_channel::Sender<Command>) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.by_ref().take(MAX_LINE as u64).read_line(&mut line) {
            Ok(0) | Err(_) => return,
            // Never interpret a truncated title, or its remaining bytes, as
            // commands. Close the connection at the first invalid frame.
            Ok(_) if !line.ends_with('\n') => return,
            Ok(_) => {
                if let Some(command) = parse_command(&line) {
                    let _ = commands.send_blocking(command);
                }
            }
        }
    }
}

fn parse_command(line: &str) -> Option<Command> {
    let line = line.trim();
    let (name, title) = match line.split_once(' ') {
        Some(("start", title)) => ("start", title.trim()),
        _ => (line, ""),
    };
    COMMANDS
        .iter()
        .find(|c| **c == name)
        .map(|c| (*c, title.to_owned()))
}

/// `omarchy-meeting-recorder start "Weekly"`: the line that starts a recording
/// with that name. Whitespace is collapsed, so a name can never add a line.
pub fn start_line(title: &[String]) -> String {
    let title = title
        .iter()
        .flat_map(|word| word.split_whitespace())
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        "start".to_owned()
    } else {
        format!("start {title}")
    }
}

/// `omarchy-meeting-recorder stop`: ask the running app to stop recording.
#[cfg(target_os = "linux")]
pub fn send(command: &str) -> bool {
    match UnixStream::connect(socket_path()) {
        Ok(mut stream) => stream.write_all(format!("{command}\n").as_bytes()).is_ok(),
        Err(_) => false,
    }
}

fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// `omarchy-meeting-recorder watch`: relay the app's state lines to stdout.
#[cfg(target_os = "linux")]
pub fn watch() {
    let mut stdout = std::io::stdout();
    loop {
        if let Ok(stream) = UnixStream::connect(socket_path()) {
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.by_ref().take(MAX_LINE as u64).read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if !line.ends_with('\n') => break, // over-long line: not ours
                    Ok(_) => {
                        if stdout
                            .write_all(line.as_bytes())
                            .and_then(|_| stdout.flush())
                            .is_err()
                        {
                            return; // the widget went away
                        }
                    }
                }
            }
        }
        if writeln!(stdout, r#"{{"state":"off"}}"#)
            .and_then(|_| stdout.flush())
            .is_err()
        {
            return;
        }
        thread::sleep(Duration::from_secs(1));
    }
}

// ---------------------------------------------------------------------------
// Windows transport: the same NDJSON protocol over a named pipe.
// `GenericNamespaced` maps the name below to
// `\\.\pipe\meeting-recorder-windows`; it is named after the binary, like the
// Unix socket is named after `APP_NAME`.
// ---------------------------------------------------------------------------

/// Pipe name for the live-state server on Windows (see above).
#[cfg(target_os = "windows")]
const PIPE_NAME: &str = if cfg!(feature = "ci-audio") {
    "meeting-recorder-windows-ci-audio"
} else {
    "meeting-recorder-windows"
};

/// The local socket name `serve` listens on and `send`/`watch` connect to.
#[cfg(target_os = "windows")]
fn pipe_name() -> interprocess::local_socket::Name<'static> {
    use interprocess::local_socket::{GenericNamespaced, prelude::*};
    PIPE_NAME
        .to_ns_name::<GenericNamespaced>()
        .expect("pipe name is a valid local socket name")
}

/// The default Windows pipe DACL grants read access to Everyone. Status
/// frames contain meeting titles, so restrict this pipe to its owner and
/// LocalSystem, with no inherited permissions.
#[cfg(target_os = "windows")]
fn private_pipe_security()
-> std::io::Result<interprocess::os::windows::security_descriptor::SecurityDescriptor> {
    use interprocess::os::windows::security_descriptor::{
        AsSecurityDescriptorExt, BorrowedSecurityDescriptor,
    };
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    let sddl: Vec<u16> = "D:P(A;;GA;;;OW)(A;;GA;;;SY)"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut raw = std::ptr::null_mut();
    // SAFETY: the string is NUL-terminated; the returned descriptor is
    // valid until LocalFree. Clone its contents before freeing it.
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut raw,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        let result = BorrowedSecurityDescriptor::from_ptr(raw).to_owned_sd();
        LocalFree(raw);
        result
    }
}

/// Starts the pipe server. Called once, from the primary instance. Clients
/// get the state lines of the busiest window; a line a client writes that
/// names one of `COMMANDS` is passed on to `commands`.
#[cfg(target_os = "windows")]
pub fn serve(
    statuses: Statuses,
    mic: Source,
    system: Source,
    commands: async_channel::Sender<Command>,
) {
    use interprocess::TryClone;
    use interprocess::local_socket::{ListenerOptions, prelude::*};
    use interprocess::os::windows::local_socket::ListenerOptionsExt;

    let listener = match private_pipe_security().and_then(|security| {
        ListenerOptions::new()
            .name(pipe_name())
            .security_descriptor(security)
            .create_sync()
    }) {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("{APP_NAME}: could not listen on \\\\.\\pipe\\{PIPE_NAME}: {e}");
            return;
        }
    };
    let clients: Arc<Mutex<Vec<std::sync::mpsc::SyncSender<Vec<u8>>>>> = Arc::default();

    let accepted = clients.clone();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if let Ok(reader) = stream.try_clone() {
                let commands = commands.clone();
                thread::spawn(move || read_commands(reader, &commands));
            }
            // Un thread d'écriture par client, nourri par un canal borné :
            // `try_send` ne bloque jamais le thread d'état.
            let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(4);
            thread::spawn(move || {
                let mut stream = stream;
                for line in rx {
                    if stream.write_all(&line).is_err() {
                        break;
                    }
                }
            });
            accepted.lock().unwrap().push(tx);
        }
    });

    thread::spawn(move || {
        loop {
            let snapshot = busiest(&statuses);
            let recording = snapshot.state == "recording";
            let taking = recording || snapshot.state == "paused";
            let until = if snapshot.pause_began > 0 {
                snapshot.pause_began
            } else {
                now()
            };
            let busy = recording || snapshot.state == "transcribing";
            let line = serde_json::json!({
                "state": snapshot.state,
                "elapsed": if taking { (until - snapshot.started_at - snapshot.paused_secs).max(0) } else { 0 },
                "title": snapshot.title,
                "mic": round(to_meter(mic.recent_peak(3))),
                "computer": round(to_meter(system.recent_peak(3))),
                "progress": round(snapshot.progress),
            })
            .to_string()
                + "\n";
            // Un client qui ne suit pas est abandonné au lieu d'être attendu :
            // `try_send` échoue dès que son canal borné est plein (~20 ms
            // de lignes), comme le timeout d'écriture côté Linux.
            let bytes = line.into_bytes();
            clients
                .lock()
                .unwrap()
                .retain(|client| client.try_send(bytes.clone()).is_ok());
            thread::sleep(Duration::from_millis(if recording {
                50
            } else if busy {
                250
            } else {
                1000
            }));
        }
    });
}

/// `meeting-recorder-windows stop`: ask the running app to stop recording.
#[cfg(target_os = "windows")]
pub fn send(command: &str) -> bool {
    use interprocess::local_socket::{Stream, prelude::*};
    match Stream::connect(pipe_name()) {
        Ok(mut stream) => stream.write_all(format!("{command}\n").as_bytes()).is_ok(),
        Err(_) => false,
    }
}

/// `meeting-recorder-windows watch`: relay the app's state lines to stdout.
#[cfg(target_os = "windows")]
pub fn watch() {
    use interprocess::local_socket::{Stream, prelude::*};
    let mut stdout = std::io::stdout();
    loop {
        if let Ok(stream) = Stream::connect(pipe_name()) {
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.by_ref().take(MAX_LINE as u64).read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if !line.ends_with('\n') => break, // over-long line: not ours
                    Ok(_) => {
                        if stdout
                            .write_all(line.as_bytes())
                            .and_then(|_| stdout.flush())
                            .is_err()
                        {
                            return; // the widget went away
                        }
                    }
                }
            }
        }
        if writeln!(stdout, r#"{{"state":"off"}}"#)
            .and_then(|_| stdout.flush())
            .is_err()
        {
            return;
        }
        thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn the_pipe_owner_can_read_status_and_send_commands() {
        use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, prelude::*};
        use interprocess::os::windows::local_socket::ListenerOptionsExt;
        let name = format!("{PIPE_NAME}-test-{}", std::process::id())
            .to_ns_name::<GenericNamespaced>()
            .unwrap()
            .into_owned();
        let listener = ListenerOptions::new()
            .name(name.clone())
            .security_descriptor(private_pipe_security().unwrap())
            .create_sync()
            .unwrap();
        let server = thread::spawn(move || {
            let mut stream = listener.accept().unwrap();
            stream.write_all(b"state\n").unwrap();
            let mut command = [0; 5];
            stream.read_exact(&mut command).unwrap();
            assert_eq!(&command, b"stop\n");
        });
        let mut client = Stream::connect(name).unwrap();
        let mut status = [0; 6];
        client.read_exact(&mut status).unwrap();
        assert_eq!(&status, b"state\n");
        client.write_all(b"stop\n").unwrap();
        server.join().unwrap();
    }

    #[test]
    fn truncated_frames_cannot_start_or_inject_commands() {
        let (tx, rx) = async_channel::unbounded();
        let long = format!("start {}stop\n", "x".repeat(MAX_LINE - 6));
        read_commands(long.as_bytes(), &tx);
        assert!(rx.try_recv().is_err());
        read_commands(b"stop".as_slice(), &tx);
        assert!(rx.try_recv().is_err());
        read_commands(b"start Weekly\npause\n".as_slice(), &tx);
        assert_eq!(rx.try_recv().unwrap(), ("start", "Weekly".into()));
        assert_eq!(rx.try_recv().unwrap(), ("pause", String::new()));
    }

    fn status(state: &'static str, title: &str) -> SharedStatus {
        Arc::new(Mutex::new(Status {
            state,
            title: title.into(),
            ..Default::default()
        }))
    }

    #[test]
    fn start_may_carry_a_name() {
        assert_eq!(parse_command("start\n"), Some(("start", String::new())));
        assert_eq!(
            parse_command("start  Product Review \n"),
            Some(("start", "Product Review".into()))
        );
        assert_eq!(parse_command("stop\n"), Some(("stop", String::new())));
        // Only start takes a name.
        assert_eq!(parse_command("stop now\n"), None);
        assert_eq!(parse_command("starting\n"), None);
    }

    #[test]
    fn a_name_cannot_add_a_command() {
        let args = ["Weekly\nstop".to_owned(), " sync ".to_owned()];
        assert_eq!(start_line(&args), "start Weekly stop sync");
        assert_eq!(start_line(&[]), "start");
        assert_eq!(start_line(&[" ".to_owned()]), "start");
    }

    #[test]
    fn the_bar_follows_the_busiest_window() {
        let statuses = Statuses::default();
        assert_eq!(busiest(&statuses).state, "idle");
        statuses.lock().unwrap().extend([
            status("done", "Old meeting"),
            status("transcribing", "Weekly"),
            status("idle", ""),
        ]);
        assert_eq!(busiest(&statuses).title, "Weekly");
        statuses
            .lock()
            .unwrap()
            .push(status("recording", "Standup"));
        assert_eq!(busiest(&statuses).title, "Standup");
    }
}
