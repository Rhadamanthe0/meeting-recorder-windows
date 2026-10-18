//! Standalone test fixture: std only, no GTK or application initialization.
use std::process::{Command, Stdio};
use std::time::Duration;

fn stage(mode: &str, phase: &str) {
    std::fs::write("fixture.phase", format!("{mode}: {phase}")).unwrap();
}

fn main() {
    let mode = std::env::args().nth(1).expect("fixture mode");
    if mode == "--probe" {
        println!("action descendant fixture ready");
        return;
    }
    if mode == "leaf" {
        for _ in 0..3000 {
            println!("descendant stdout");
            eprintln!("descendant stderr");
            std::thread::sleep(Duration::from_millis(10));
        }
        return;
    }
    assert!(matches!(mode.as_str(), "running" | "exited"));
    stage(&mode, "helper entered");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .arg("leaf")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW; inherit action Job Object.
    }
    let mut child = command.spawn().unwrap_or_else(|error| {
        stage(&mode, &format!("leaf spawn failed: {error}"));
        panic!("leaf spawn failed: {error}");
    });
    stage(&mode, &format!("leaf spawned: {}", child.id()));
    std::fs::write("descendant.pid", child.id().to_string()).unwrap();
    stage(&mode, "PID published");
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    if mode == "running" {
        stage(&mode, "parent waiting");
        std::thread::sleep(Duration::from_secs(30));
    } else {
        stage(&mode, "parent exiting");
    }
}
