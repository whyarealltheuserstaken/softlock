use std::collections::HashMap;
use std::io::{BufReader, ErrorKind};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::UI::Input::KeyboardAndMouse::{VK_F13, VK_F24};

use crate::guard;
use crate::protocol::{self, Command, Hello, ROLE_ELEVATED, ROLE_MEDIUM, ROLE_SYSTEM, Reply};
use crate::verdict::{self, EXTRA_LOAD, EXTRA_MARKER, LOAD_EVENTS, Sample, VISIBILITY_KEYS};

use super::args::Args;
use super::identity;
use super::input;
use super::service;
use super::util::{self, say};

struct Peer {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    hello: Hello,
}

pub fn run(args: &Args) -> i32 {
    if let Err(error) = guard::service_install_allowed(
        std::env::var("GITHUB_ACTIONS").ok().as_deref(),
        args.allow_local_service,
    ) {
        eprintln!("{error}");
        return 2;
    }
    if let Err(error) = std::fs::create_dir_all(&args.output_dir) {
        eprintln!("create {}: {error}", args.output_dir.display());
        return 1;
    }
    let outcome = run_proof(args);
    service::stop();
    match outcome {
        Ok(code) => code,
        Err(error) => {
            say(&format!("proof error: {error}"));
            print_logs(&args.output_dir);
            say("result=fail");
            1
        }
    }
}

fn run_proof(args: &Args) -> Result<i32, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| format!("bind: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("listen: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("local address: {error}"))?
        .port();
    say(&format!(
        "supervisor port={port} image={}",
        args.runner_image
    ));
    let (own_timeout, own_raw) = identity::low_level_hooks_timeout();
    say(&format!(
        "supervisor-timeout-ms={own_timeout} supervisor-timeout-raw={own_raw}"
    ));
    service::start(args, port)?;
    let launch = service::wait_for_launch(&args.output_dir, Duration::from_secs(60))?;
    say(&format!(
        "launch console={} user-il={} elevation-type={} linked-il={:?} error={}",
        launch.console_session,
        launch.user_il,
        launch.elevation_type,
        launch.linked_il,
        launch.error.as_deref().unwrap_or("none")
    ));
    for note in &launch.notes {
        say(&format!("launch-note={note}"));
    }
    for child in &launch.children {
        say(&format!(
            "launch-child role={} pid={} token={} il={:?} error={}",
            child.role,
            child
                .pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "-".to_string()),
            child.token,
            child.il,
            child.error.as_deref().unwrap_or("none")
        ));
    }
    if let Some(error) = &launch.error {
        print_logs(&args.output_dir);
        return Err(error.clone());
    }
    let mut peers = accept_roles(&listener, Duration::from_secs(45))?;
    for peer in peers.values() {
        let hello = &peer.hello;
        say(&format!(
            "hello role={} pid={} il={} user={} session={} station={} desktop={} timeout={} raw={}",
            hello.role,
            hello.pid,
            hello.il,
            hello.user,
            hello.session,
            hello.station,
            hello.desktop,
            hello.timeout_ms,
            hello.timeout_raw
        ));
    }
    let control = visibility_phase(&mut peers, ROLE_MEDIUM)?;
    say(&format!(
        "control system={} medium={} elevated-hook={} window={}",
        control.system, control.medium, control.elevated, control.window
    ));
    let elevated = visibility_phase(&mut peers, ROLE_ELEVATED)?;
    say(&format!(
        "elevated system={} injected={} medium={} elevated-hook={} window={}",
        elevated.system, elevated.injected, elevated.medium, elevated.elevated, elevated.window
    ));
    let (load_sent, load_seen, alive) = load_phase(&mut peers)?;
    say(&format!(
        "load sent={load_sent} seen={load_seen} alive-after={alive}"
    ));
    let timeout_ms = timeout_for_probe(&peers, own_timeout);
    say(&format!("slow-timeout-ms={timeout_ms}"));
    let liveness = liveness_phase(&mut peers, timeout_ms)?;
    say(&format!(
        "slow healthy-before={} entered={} detect-ms={} removed={} timeout={} blocked-ms={} error={}",
        liveness.healthy_before,
        liveness.entered,
        liveness.detect_ms,
        liveness.removed,
        timeout_ms,
        liveness.blocked_ms,
        liveness.error.as_deref().unwrap_or("none")
    ));
    quit_all(&mut peers);
    let system = &peers[ROLE_SYSTEM].hello;
    let medium = &peers[ROLE_MEDIUM].hello;
    let elevated_hello = &peers[ROLE_ELEVATED].hello;
    let sample = Sample {
        system_il: system.il,
        medium_il: medium.il,
        elevated_il: elevated_hello.il,
        system_session: system.session,
        console_session: launch.console_session,
        system_user_is_system: system.user.eq_ignore_ascii_case("SYSTEM"),
        system_on_default_desktop: on_default_desktop(&system.station, &system.desktop),
        elevated_token: token_source(&launch, ROLE_ELEVATED),
        medium_token: token_source(&launch, ROLE_MEDIUM),
        control_system_seen: control.system,
        control_medium_seen: control.medium,
        control_window: control.window,
        elevated_system_seen: elevated.system,
        elevated_system_injected: elevated.injected,
        elevated_medium_seen: elevated.medium,
        elevated_hook_seen: elevated.elevated,
        elevated_window: elevated.window,
        load_sent,
        load_seen,
        hook_alive_after_load: alive,
        slow_entered: liveness.entered,
        liveness_healthy_before: liveness.healthy_before,
        liveness_detect_ms: liveness.detect_ms,
        liveness_removed: liveness.removed,
        timeout_ms,
    };
    let decision = verdict::judge(&sample);
    let report = decision.report(&sample);
    for line in report.lines() {
        say(line);
    }
    let _ = std::fs::write(args.output_dir.join("result.txt"), &report);
    if !decision.pass {
        print_logs(&args.output_dir);
    }
    Ok(if decision.pass { 0 } else { 1 })
}

struct PhaseCounts {
    system: u32,
    injected: u32,
    medium: u32,
    elevated: u32,
    window: u32,
}

fn visibility_phase(
    peers: &mut HashMap<String, Peer>,
    target: &str,
) -> Result<PhaseCounts, String> {
    let mut last = PhaseCounts {
        system: 0,
        injected: 0,
        medium: 0,
        elevated: 0,
        window: 0,
    };
    for attempt in 1..=3 {
        reset_all(peers)?;
        let foreground = ask(
            peers.get_mut(target).ok_or("probe missing")?,
            &Command::Foreground,
        )?;
        let Reply::Foreground { ok, rect, error } = foreground else {
            return Err("probe did not answer foreground".to_string());
        };
        if !ok {
            say(&format!(
                "foreground attempt {attempt} target={target} error={}",
                error.unwrap_or_else(|| "unknown".to_string())
            ));
            continue;
        }
        if let Some(rect) = rect {
            say(&format!(
                "foreground attempt {attempt} target={target} rect={}x{}+{}+{}",
                rect.width, rect.height, rect.x, rect.y
            ));
        }
        thread::sleep(Duration::from_millis(150));
        let inserted = input::send_key_pairs(VK_F13, crate::verdict::EXTRA_VIS, VISIBILITY_KEYS)?;
        say(&format!("visibility attempt {attempt} inserted={inserted}"));
        last = wait_counts(peers, target, Duration::from_secs(2))?;
        let window_ok = last.window == VISIBILITY_KEYS;
        let hooks_ok = if target == ROLE_ELEVATED {
            last.system == VISIBILITY_KEYS
                && last.injected == VISIBILITY_KEYS
                && last.medium == 0
                && last.elevated == VISIBILITY_KEYS
        } else {
            last.system == VISIBILITY_KEYS
                && last.medium == VISIBILITY_KEYS
                && last.elevated == VISIBILITY_KEYS
        };
        if window_ok && hooks_ok {
            return Ok(last);
        }
        say(&format!(
            "visibility attempt {attempt} target={target} system={} injected={} medium={} elevated={} window={}",
            last.system, last.injected, last.medium, last.elevated, last.window
        ));
    }
    Ok(last)
}

fn wait_counts(
    peers: &mut HashMap<String, Peer>,
    target: &str,
    timeout: Duration,
) -> Result<PhaseCounts, String> {
    let start = Instant::now();
    let mut last = read_counts(peers, target)?;
    while start.elapsed() < timeout {
        if last.window == VISIBILITY_KEYS {
            break;
        }
        thread::sleep(Duration::from_millis(50));
        last = read_counts(peers, target)?;
    }
    // Hooks run before the window message. Read once more after the window settles.
    thread::sleep(Duration::from_millis(50));
    read_counts(peers, target)
}

fn read_counts(peers: &mut HashMap<String, Peer>, target: &str) -> Result<PhaseCounts, String> {
    let system = counts_of(peers, ROLE_SYSTEM)?;
    let medium = counts_of(peers, ROLE_MEDIUM)?;
    let elevated = counts_of(peers, ROLE_ELEVATED)?;
    let window = if target == ROLE_MEDIUM {
        medium.window_keys
    } else {
        elevated.window_keys
    };
    Ok(PhaseCounts {
        system: saturate(system.vis_seen),
        injected: saturate(system.vis_injected),
        medium: saturate(medium.vis_seen),
        elevated: saturate(elevated.vis_seen),
        window: saturate(window),
    })
}

struct Liveness {
    healthy_before: bool,
    entered: bool,
    detect_ms: u64,
    removed: bool,
    blocked_ms: u64,
    error: Option<String>,
}

fn liveness_phase(peers: &mut HashMap<String, Peer>, timeout_ms: u64) -> Result<Liveness, String> {
    let armed = ask(
        peers.get_mut(ROLE_SYSTEM).ok_or("system hook missing")?,
        &Command::Slow { timeout_ms },
    )?;
    let Reply::Slow {
        healthy_before,
        sleep_ms,
        error,
        ..
    } = armed
    else {
        return Err("system hook did not arm the slow probe".to_string());
    };
    if !healthy_before {
        return Ok(Liveness {
            healthy_before: false,
            entered: false,
            detect_ms: 0,
            removed: false,
            blocked_ms: 0,
            error,
        });
    }
    // The marker has to come from this process. Injecting it from the hook
    // process blocks inside the callback and Windows does not remove the hook.
    let t_send = Instant::now();
    let sender = thread::spawn(|| {
        let started = Instant::now();
        let result = input::send_key_pairs(VK_F24, EXTRA_MARKER, 1);
        (started.elapsed().as_millis() as u64, result)
    });
    let removal = t_send + Duration::from_millis(timeout_ms);
    while Instant::now() < removal {
        thread::sleep(Duration::from_millis(20));
    }
    thread::sleep(Duration::from_millis(50));
    let before = counts_of(peers, ROLE_SYSTEM)?.marker_seen;
    let _ = input::send_key_pairs(VK_F24, EXTRA_MARKER, 1);
    let mut saw = counts_of(peers, ROLE_SYSTEM)?.marker_seen > before;
    let poll_start = Instant::now();
    while !saw && poll_start.elapsed() < Duration::from_millis(40) {
        thread::sleep(Duration::from_millis(10));
        saw = counts_of(peers, ROLE_SYSTEM)?.marker_seen > before;
    }
    let detect_ms = removal.elapsed().as_millis() as u64;
    let return_deadline = Instant::now() + Duration::from_millis(sleep_ms.saturating_add(2_000));
    let mut entered = false;
    let mut returned = false;
    while Instant::now() < return_deadline {
        let counts = counts_of(peers, ROLE_SYSTEM)?;
        entered = counts.slow_entered;
        returned = counts.slow_returned;
        if returned {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    thread::sleep(Duration::from_millis(300));
    let before_confirm = counts_of(peers, ROLE_SYSTEM)?.marker_seen;
    let _ = input::send_key_pairs(VK_F24, EXTRA_MARKER, 1);
    thread::sleep(Duration::from_millis(250));
    let still_dead = counts_of(peers, ROLE_SYSTEM)?.marker_seen == before_confirm;
    let (blocked_ms, marker_result) = sender
        .join()
        .unwrap_or((0, Err("sender thread panicked".to_string())));
    if let Err(failure) = &marker_result {
        say(&format!("slow-marker error={failure}"));
    }
    Ok(Liveness {
        healthy_before: true,
        entered,
        detect_ms,
        removed: entered && !saw && still_dead && returned,
        blocked_ms,
        error: None,
    })
}

struct HookCounts {
    vis_seen: u64,
    vis_injected: u64,
    load_seen: u64,
    marker_seen: u64,
    window_keys: u64,
    slow_entered: bool,
    slow_returned: bool,
}

fn counts_of(peers: &mut HashMap<String, Peer>, role: &str) -> Result<HookCounts, String> {
    let reply = ask(peers.get_mut(role).ok_or("role missing")?, &Command::Counts)?;
    match reply {
        Reply::Counts {
            vis_seen,
            vis_injected,
            load_seen,
            marker_seen,
            window_keys,
            slow_entered,
            slow_returned,
        } => Ok(HookCounts {
            vis_seen,
            vis_injected,
            load_seen,
            marker_seen,
            window_keys,
            slow_entered,
            slow_returned,
        }),
        other => Err(format!("{role} counts reply was {other:?}")),
    }
}

fn load_phase(peers: &mut HashMap<String, Peer>) -> Result<(u32, u32, bool), String> {
    reset_all(peers)?;
    let pairs = LOAD_EVENTS / 2;
    let inserted = input::send_key_pairs(VK_F13, EXTRA_LOAD, pairs)?;
    let start = Instant::now();
    let mut seen = 0u64;
    while start.elapsed() < Duration::from_secs(60) {
        let load_seen = counts_of(peers, ROLE_SYSTEM)?.load_seen;
        seen = load_seen;
        if seen >= u64::from(inserted) {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let before = counts_of(peers, ROLE_SYSTEM)?.marker_seen;
    let marker_inserted = input::send_key_pairs(VK_F24, EXTRA_MARKER, 1)?;
    let marker_deadline = Instant::now() + Duration::from_millis(500);
    let mut marker = before;
    while Instant::now() < marker_deadline {
        marker = counts_of(peers, ROLE_SYSTEM)?.marker_seen;
        if marker > before {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    say(&format!(
        "load-marker inserted={marker_inserted} seen-delta={}",
        marker.saturating_sub(before)
    ));
    Ok((inserted, saturate(seen), marker > before))
}

fn reset_all(peers: &mut HashMap<String, Peer>) -> Result<(), String> {
    for role in [ROLE_MEDIUM, ROLE_ELEVATED, ROLE_SYSTEM] {
        let reply = ask(peers.get_mut(role).ok_or("role missing")?, &Command::Reset)?;
        if !matches!(reply, Reply::Reset) {
            return Err(format!("{role} did not reset"));
        }
    }
    Ok(())
}

fn ask(peer: &mut Peer, command: &Command) -> Result<Reply, String> {
    protocol::write_msg(&mut peer.writer, command)?;
    protocol::read_msg(&mut peer.reader)
}

fn accept_roles(
    listener: &TcpListener,
    timeout: Duration,
) -> Result<HashMap<String, Peer>, String> {
    let start = Instant::now();
    let mut peers = HashMap::new();
    while peers.len() < 3 && start.elapsed() < timeout {
        match listener.accept() {
            Ok((stream, _)) => {
                // Windows copies the listener's nonblocking mode onto the
                // accepted socket. The hello can already be buffered, and the
                // next reply is not, so a later read returns WSAEWOULDBLOCK.
                stream
                    .set_nonblocking(false)
                    .map_err(|error| format!("blocking socket: {error}"))?;
                let _ = stream.set_nodelay(true);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(180)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
                let writer = stream
                    .try_clone()
                    .map_err(|error| format!("clone socket: {error}"))?;
                let mut reader = BufReader::new(stream);
                let hello: Hello = protocol::read_msg(&mut reader)?;
                peers.insert(
                    hello.role.clone(),
                    Peer {
                        reader,
                        writer,
                        hello,
                    },
                );
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(format!("accept: {error}")),
        }
    }
    for role in [ROLE_MEDIUM, ROLE_ELEVATED, ROLE_SYSTEM] {
        if !peers.contains_key(role) {
            return Err(format!("missing {role} hook process"));
        }
    }
    Ok(peers)
}

fn quit_all(peers: &mut HashMap<String, Peer>) {
    for peer in peers.values_mut() {
        let _ = protocol::write_msg(&mut peer.writer, &Command::Quit);
        let _: Result<Reply, String> = protocol::read_msg(&mut peer.reader);
    }
}

fn timeout_for_probe(peers: &HashMap<String, Peer>, own: u64) -> u64 {
    let mut timeout = own;
    for peer in peers.values() {
        timeout = timeout.max(peer.hello.timeout_ms);
    }
    timeout.clamp(50, 5_000)
}

fn token_source(launch: &protocol::LaunchReport, role: &str) -> String {
    launch
        .children
        .iter()
        .find(|child| child.role == role)
        .map(|child| child.token.clone())
        .unwrap_or_else(|| "missing".to_string())
}

fn on_default_desktop(station: &str, desktop: &str) -> bool {
    station.eq_ignore_ascii_case("WinSta0") && desktop.eq_ignore_ascii_case("Default")
}

fn saturate(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn print_logs(output: &Path) {
    for name in ["service.log", "medium.log", "elevated.log", "system.log"] {
        let path = output.join(name);
        let Some(text) = util::read_text(&path) else {
            continue;
        };
        say(&format!("--- {} ---", path.display()));
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.len().saturating_sub(80);
        for line in &lines[start..] {
            say(line);
        }
    }
}
