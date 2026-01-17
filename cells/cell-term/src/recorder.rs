//! Terminal session recording via PTY

use cell_term_proto::RecordConfig;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::parser;
use crate::renderer;

/// Record a terminal session, optionally auto-executing a command
pub async fn record_session(
    command: Option<String>,
    config: RecordConfig,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Run the blocking PTY work on a thread pool
    let result = tokio::task::spawn_blocking(move || record_session_blocking(command, config))
        .await
        .map_err(|e| format!("Task join error: {e}"))??;

    Ok(result)
}

fn record_session_blocking(
    command: Option<String>,
    config: RecordConfig,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let pty_system = native_pty_system();

    // Get terminal size or use defaults
    let (cols, rows) = term_size::dimensions().unwrap_or((80, 24));

    let pair = pty_system.openpty(PtySize {
        rows: rows as u16,
        cols: cols as u16,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    // Determine shell
    let shell = config
        .shell
        .unwrap_or_else(|| std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string()));

    let mut cmd = CommandBuilder::new(&shell);

    // Disable pagers
    cmd.env("PAGER", "/bin/cat");
    cmd.env("GIT_PAGER", "/bin/cat");
    cmd.env("DELTA_PAGER", "/bin/cat");

    // Set current directory
    if let Ok(cwd) = std::env::current_dir() {
        cmd.cwd(cwd);
    }

    let mut child = pair.slave.spawn_command(cmd)?;
    let child_pid = child.process_id();

    let mut reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;

    let (output_tx, output_rx) = std::sync::mpsc::channel::<Option<Vec<u8>>>();

    let last_activity = Arc::new(Mutex::new(Instant::now()));

    // Thread: read from PTY, send to channel
    let last_activity_clone = last_activity.clone();
    let output_tx_clone = output_tx.clone();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = output_tx_clone.send(None);
                    break;
                }
                Ok(n) => {
                    *last_activity_clone.lock().unwrap() = Instant::now();
                    let _ = output_tx_clone.send(Some(buf[..n].to_vec()));
                }
                Err(_) => {
                    let _ = output_tx_clone.send(None);
                    break;
                }
            }
        }
    });

    // Thread: collect output and echo to stdout
    let collect_handle = {
        let mut collected = Vec::new();
        std::thread::spawn(move || {
            // Put terminal in raw mode for interactive display
            #[cfg(unix)]
            let _raw_guard = RawModeGuard::new();

            let mut stdout = std::io::stdout();
            for msg in output_rx {
                match msg {
                    Some(data) => {
                        collected.extend_from_slice(&data);
                        let _ = stdout.write_all(&data);
                        let _ = stdout.flush();
                    }
                    None => break,
                }
            }
            collected
        })
    };

    // Thread: handle stdin
    let stdin_handle = {
        let last_activity = last_activity.clone();
        let mut writer = writer;

        std::thread::spawn(move || {
            if let Some(cmd_str) = command {
                // Auto-execute mode: wait for shell to be ready, then send command
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                    let elapsed = last_activity.lock().unwrap().elapsed();
                    if elapsed.as_millis() > 350 {
                        break;
                    }
                }

                // Send the command
                let cmd_with_newline = format!("{cmd_str}\n");
                let _ = writer.write_all(cmd_with_newline.as_bytes());

                // Wait for command to finish
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(80));
                    let elapsed = last_activity.lock().unwrap().elapsed();

                    // Check if shell has children (command still running)
                    #[cfg(unix)]
                    let has_children = if let Some(pid) = child_pid {
                        nix::sys::wait::waitpid(
                            nix::unistd::Pid::from_raw(-(pid as i32)),
                            Some(nix::sys::wait::WaitPidFlag::WNOHANG),
                        )
                        .is_err()
                    } else {
                        false
                    };

                    #[cfg(not(unix))]
                    let has_children = false;

                    if elapsed.as_millis() > 150 && !has_children {
                        // Send Ctrl+D to exit
                        let _ = writer.write_all(&[0x04]);
                        break;
                    }
                }
            } else {
                // Interactive mode: forward stdin to PTY
                let mut stdin = std::io::stdin();
                let mut buf = vec![0u8; 1024];
                loop {
                    match stdin.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if writer.write_all(&buf[..n]).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
        })
    };

    // Wait for child to exit
    let _ = child.wait();
    let _ = output_tx.send(None);

    // Wait for threads
    let _ = stdin_handle.join();
    let collected = collect_handle
        .join()
        .map_err(|_| "Failed to join collect thread")?;

    // Parse and render
    let performer = parser::parse(&collected);

    // Remove trailing prompt lines (look for ❯ character)
    let mut lines = performer.screen.lines;
    if let Some((i, _)) = lines
        .iter()
        .enumerate()
        .rev()
        .find(|(_, line)| line.iter().any(|sc| sc.c == '❯'))
    {
        lines.truncate(i.saturating_sub(2));
    }

    // Create a new screen with truncated lines
    let screen = parser::Screen {
        lines,
        ..Default::default()
    };

    let html = renderer::render(&screen);

    // Copy to clipboard
    if let Err(e) = copy_to_clipboard(&html) {
        eprintln!("Warning: failed to copy to clipboard: {e}");
    }

    // Write to /tmp/ddc-term
    if let Err(e) = std::fs::write("/tmp/ddc-term", &html) {
        eprintln!("Warning: failed to write to /tmp/ddc-term: {e}");
    }

    eprintln!("\n✂️ Output copied to clipboard and saved to /tmp/ddc-term");

    Ok(html)
}

fn copy_to_clipboard(text: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use arboard::Clipboard;
    let mut clipboard = Clipboard::new()?;
    clipboard.set_text(text)?;
    Ok(())
}

/// RAII guard for raw terminal mode on Unix
#[cfg(unix)]
struct RawModeGuard {
    original: Option<nix::sys::termios::Termios>,
}

#[cfg(unix)]
impl RawModeGuard {
    fn new() -> Self {
        use nix::sys::termios::{SetArg, tcgetattr, tcsetattr};
        use std::os::fd::AsFd;

        let stdin = std::io::stdin();
        let original = tcgetattr(stdin.as_fd()).ok();

        if let Some(ref orig) = original {
            let mut raw = orig.clone();
            nix::sys::termios::cfmakeraw(&mut raw);
            let _ = tcsetattr(stdin.as_fd(), SetArg::TCSANOW, &raw);
        }

        Self { original }
    }
}

#[cfg(unix)]
impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if let Some(ref orig) = self.original {
            use nix::sys::termios::{SetArg, tcsetattr};
            use std::os::fd::AsFd;
            let stdin = std::io::stdin();
            let _ = tcsetattr(stdin.as_fd(), SetArg::TCSANOW, orig);
        }
    }
}
