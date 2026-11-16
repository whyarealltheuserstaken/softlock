//! Load and unload the throwaway jobs. A drop guard bootouts even if a later step panics.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use crate::labels::{AGENT_LABEL, DAEMON_LABEL, DAEMON_PLIST};
use crate::launchctl::{PrintState, parse_pid, parse_print};
use crate::plist::{PlistOpts, agent_plist_path, job_arguments, job_plist};
use crate::report::{JobMarker, JobObservation, JobReport, job_from_observation};
use crate::text::one_line;

pub fn exercise(exe: &Path, output_dir: &Path) -> Vec<JobReport> {
    let uid = unsafe { libc::getuid() };
    let agent_plist = match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => agent_plist_path(Path::new(&home)),
        _ => {
            return vec![
                failed("agent", "Aqua", "plist path: HOME is unset"),
                install_daemon(exe, output_dir),
            ];
        }
    };
    let agent = install_user_job(exe, output_dir, uid, &agent_plist);
    let daemon = install_daemon(exe, output_dir);
    vec![agent, daemon]
}

fn install_user_job(exe: &Path, output_dir: &Path, uid: u32, plist_path: &Path) -> JobReport {
    let domain = format!("gui/{uid}");
    let target = format!("{domain}/{AGENT_LABEL}");
    let stdout_path = output_dir.join("agent.out");
    let stderr_path = output_dir.join("agent.err");
    let marker = output_dir.join("agent-job.json");
    let _guard = JobGuard {
        sudo: false,
        target: target.clone(),
        plist: plist_path.to_path_buf(),
    };
    let Some(exe_text) = path_text(exe) else {
        return failed("agent", "Aqua", "plist path: executable is not utf-8");
    };
    let Some(output_text) = path_text(output_dir) else {
        return failed("agent", "Aqua", "plist path: output directory is not utf-8");
    };
    let arguments = match job_arguments(exe_text, "agent", output_text) {
        Ok(arguments) => arguments,
        Err(error) => return failed("agent", "Aqua", &format!("plist: {error}")),
    };
    let aqua = render_plist(
        AGENT_LABEL,
        &arguments,
        &stdout_path,
        &stderr_path,
        Some("Aqua"),
    );
    let body = match aqua {
        Ok(body) => body,
        Err(error) => return failed("agent", "Aqua", &format!("plist: {error}")),
    };
    if let Err(error) = write_plist(plist_path, &body) {
        return failed("agent", "Aqua", &format!("plist: {error}"));
    }
    let _ = run("launchctl", &["bootout", &target]);
    let (ok, _stdout, stderr) = run(
        "launchctl",
        &[
            "bootstrap",
            &domain,
            path_text(plist_path).unwrap_or_default(),
        ],
    );
    if ok {
        return finish_job("agent", "Aqua", false, &target, &marker, &stderr_path, None);
    }
    let aqua_error = one_line(&stderr);
    let plain = match render_plist(AGENT_LABEL, &arguments, &stdout_path, &stderr_path, None) {
        Ok(body) => body,
        Err(error) => {
            return failed("agent", "Aqua", &format!("{aqua_error}; plist: {error}"));
        }
    };
    if let Err(error) = write_plist(plist_path, &plain) {
        return failed("agent", "Aqua", &format!("{aqua_error}; plist: {error}"));
    }
    let _ = run("launchctl", &["bootout", &target]);
    let (retry_ok, _stdout, retry_err) = run(
        "launchctl",
        &[
            "bootstrap",
            &domain,
            path_text(plist_path).unwrap_or_default(),
        ],
    );
    if !retry_ok {
        return failed(
            "agent",
            "none",
            &format!("{aqua_error}; {}", one_line(&retry_err)),
        );
    }
    finish_job(
        "agent",
        "none",
        false,
        &target,
        &marker,
        &stderr_path,
        Some(format!(
            "launchctl Aqua bootstrap failed: {aqua_error}; retried without LimitLoadToSessionType"
        )),
    )
}

fn install_daemon(exe: &Path, output_dir: &Path) -> JobReport {
    let target = format!("system/{DAEMON_LABEL}");
    let plist_path = PathBuf::from(DAEMON_PLIST);
    let staged = output_dir.join("daemon.plist");
    let stdout_path = output_dir.join("daemon.out");
    let stderr_path = output_dir.join("daemon.err");
    let marker = output_dir.join("daemon-job.json");
    let _guard = JobGuard {
        sudo: true,
        target: target.clone(),
        plist: plist_path.clone(),
    };
    let Some(exe_text) = path_text(exe) else {
        return failed("daemon", "none", "plist path: executable is not utf-8");
    };
    let Some(output_text) = path_text(output_dir) else {
        return failed(
            "daemon",
            "none",
            "plist path: output directory is not utf-8",
        );
    };
    let arguments = match job_arguments(exe_text, "daemon", output_text) {
        Ok(arguments) => arguments,
        Err(error) => return failed("daemon", "none", &format!("plist: {error}")),
    };
    let body = match render_plist(DAEMON_LABEL, &arguments, &stdout_path, &stderr_path, None) {
        Ok(body) => body,
        Err(error) => return failed("daemon", "none", &format!("plist: {error}")),
    };
    if let Err(error) = write_plist(&staged, &body) {
        return failed("daemon", "none", &format!("plist: {error}"));
    }
    let staged_text = path_text(&staged).unwrap_or_default().to_string();
    let (copied, _stdout, copy_err) =
        run("sudo", &["-n", "cp", staged_text.as_str(), DAEMON_PLIST]);
    if !copied {
        return failed(
            "daemon",
            "none",
            &format!("sudo cp failed: {}", one_line(&copy_err)),
        );
    }
    let _ = run("sudo", &["-n", "chown", "root:wheel", DAEMON_PLIST]);
    let _ = run("sudo", &["-n", "chmod", "644", DAEMON_PLIST]);
    let _ = run("sudo", &["-n", "launchctl", "bootout", &target]);
    let (ok, _stdout, stderr) = run(
        "sudo",
        &["-n", "launchctl", "bootstrap", "system", DAEMON_PLIST],
    );
    if !ok {
        return failed("daemon", "none", &one_line(&stderr));
    }
    finish_job("daemon", "none", true, &target, &marker, &stderr_path, None)
}

fn finish_job(
    kind: &str,
    session_limit: &str,
    sudo: bool,
    target: &str,
    marker_path: &Path,
    stderr_path: &Path,
    note: Option<String>,
) -> JobReport {
    let (print_state, pid_from_print, marker) = wait_for_marker(sudo, target, marker_path);
    if sudo {
        chmod_readable(stderr_path);
    }
    let stderr_tail = fs::read_to_string(stderr_path).unwrap_or_default();
    bootout(sudo, target);
    let absent = wait_until_absent(sudo, target);
    let mut report = job_from_observation(&JobObservation {
        kind: kind.to_string(),
        session_limit: session_limit.to_string(),
        keep_alive: true,
        bootstrap_ok: true,
        bootstrap_error: None,
        print_state,
        pid_from_print,
        absent_after_bootout: absent,
        stderr_tail,
        marker,
    });
    if let Some(note) = note {
        report.error = Some(match report.error {
            Some(existing) => format!("{existing}; {note}"),
            None => note,
        });
    }
    report
}

fn failed(kind: &str, session_limit: &str, detail: &str) -> JobReport {
    job_from_observation(&JobObservation {
        kind: kind.to_string(),
        session_limit: session_limit.to_string(),
        keep_alive: detail.contains("KeepAlive") || !detail.starts_with("plist"),
        bootstrap_ok: false,
        bootstrap_error: Some(detail.to_string()),
        print_state: PrintState::Absent,
        pid_from_print: None,
        absent_after_bootout: true,
        stderr_tail: String::new(),
        marker: None,
    })
}

fn render_plist(
    label: &str,
    arguments: &[String],
    stdout_path: &Path,
    stderr_path: &Path,
    session_limit: Option<&str>,
) -> Result<String, String> {
    let stdout = path_text(stdout_path).ok_or("stdout path is not utf-8")?;
    let stderr = path_text(stderr_path).ok_or("stderr path is not utf-8")?;
    job_plist(&PlistOpts {
        label,
        arguments,
        stdout_path: stdout,
        stderr_path: stderr,
        session_limit,
    })
}

fn write_plist(path: &Path, body: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    fs::write(path, body).map_err(|error| format!("write {}: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o644));
    }
    Ok(())
}

fn wait_for_marker(
    sudo: bool,
    target: &str,
    marker_path: &Path,
) -> (PrintState, Option<u32>, Option<JobMarker>) {
    let start = Instant::now();
    let mut state = PrintState::Absent;
    let mut pid = None;
    let mut marker = None;
    while start.elapsed() < Duration::from_secs(30) {
        let (ok, stdout, _stderr) = print_job(sudo, target);
        state = parse_print(ok, &stdout);
        pid = parse_pid(&stdout).or(pid);
        if marker.is_none() {
            if sudo {
                chmod_readable(marker_path);
            }
            marker = read_marker(marker_path);
        }
        if marker.is_some()
            && (state == PrintState::Running || start.elapsed() > Duration::from_secs(2))
        {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    (state, pid, marker)
}

fn wait_until_absent(sudo: bool, target: &str) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        let (ok, stdout, _stderr) = print_job(sudo, target);
        if parse_print(ok, &stdout) == PrintState::Absent {
            return true;
        }
        thread::sleep(Duration::from_millis(200));
    }
    false
}

fn bootout(sudo: bool, target: &str) {
    if sudo {
        let _ = run("sudo", &["-n", "launchctl", "bootout", target]);
    } else {
        let _ = run("launchctl", &["bootout", target]);
    }
}

fn print_job(sudo: bool, target: &str) -> (bool, String, String) {
    if sudo {
        run("sudo", &["-n", "launchctl", "print", target])
    } else {
        run("launchctl", &["print", target])
    }
}

fn read_marker(path: &Path) -> Option<JobMarker> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn path_text(path: &Path) -> Option<&str> {
    path.to_str()
}

fn chmod_readable(path: &Path) {
    let Some(text) = path.to_str() else {
        return;
    };
    let _ = run("sudo", &["-n", "chmod", "a+r", text]);
}

fn run(program: &str, args: &[&str]) -> (bool, String, String) {
    match Command::new(program).args(args).output() {
        Ok(output) => (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ),
        Err(error) => (false, String::new(), error.to_string()),
    }
}

struct JobGuard {
    sudo: bool,
    target: String,
    plist: PathBuf,
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        bootout(self.sudo, &self.target);
        if self.sudo {
            let plist = self.plist.to_string_lossy();
            let _ = run("sudo", &["-n", "rm", "-f", plist.as_ref()]);
        } else {
            let _ = fs::remove_file(&self.plist);
        }
    }
}
