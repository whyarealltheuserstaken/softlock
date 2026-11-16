//! Windows proof tool. The keyboard hook lives on this thread.
//! Foreground checks and the Hello call run on other threads.

mod consent;
mod foreground;
mod hooks;
mod password;
mod window;

use std::io::{self, Write};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use softlock_hello_spike::clamp_block_limit;

use hooks::{HELLO_RUNNING, HookGuard, STOP};

const COUNTDOWN: Duration = Duration::from_secs(5);
const OBSERVE: Duration = Duration::from_secs(15);
const SMOKE_LIMIT: Duration = Duration::from_secs(3);

pub fn run() -> i32 {
    hooks::install_crash_handlers();
    match parse_args() {
        Command::Help => {
            print_help();
            0
        }
        Command::Hello => hello_session(),
        Command::Password => password::run(),
        Command::Smoke => smoke_session(),
    }
}

enum Command {
    Help,
    Hello,
    Password,
    Smoke,
}

fn parse_args() -> Command {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        None | Some("help") | Some("--help") | Some("-h") => Command::Help,
        Some("hello") => Command::Hello,
        Some("password") => Command::Password,
        Some("smoke") => Command::Smoke,
        Some(other) => {
            eprintln!("unknown command: {other}");
            Command::Help
        }
    }
}

fn print_help() {
    eprintln!(
        "\
hello-spike hello
    Block physical keys and the mouse, show a topmost overlay, and call
    the Windows Hello consent verifier. Removes the block by itself.

hello-spike password
    Check the signed-in account's password with a network logon.
    Does not block the keyboard. Wrong attempts count toward lockout.

hello-spike smoke
    CI only. Injected input, a few seconds, then exit.

Escape while blocked: hold Ctrl, Alt, and Shift, then press Q.
Ctrl+Alt+Del still works. The block never lasts more than 3 minutes."
    );
}

fn hello_session() -> i32 {
    println!("This blocks YOUR keyboard and mouse.");
    println!("Escape: hold Ctrl, Alt, and Shift, then press Q.");
    println!("That escape does not need Windows Hello.");
    println!("Ctrl+Alt+Del still works, because Windows handles it first.");
    println!("If this tool crashes, it removes the block.");
    println!("The block ends by itself within 3 minutes.");
    println!("Starting in 5 seconds. Press Ctrl+C now to cancel.");
    let _ = io::stdout().flush();
    for left in (1..=COUNTDOWN.as_secs()).rev() {
        println!("{left}...");
        let _ = io::stdout().flush();
        thread::sleep(Duration::from_secs(1));
    }

    let limit = clamp_block_limit(softlock_hello_spike::MAX_BLOCK);
    let armed = match arm(limit) {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("could not install the input hook: {error}");
            return 2;
        }
    };
    let hwnd = match window::create_overlay() {
        Ok(hwnd) => hwnd,
        Err(error) => {
            eprintln!("could not create the overlay: {error}");
            drop(armed);
            return 2;
        }
    };
    window::set_text(window::TEXT_BLOCKED);
    window::raise_topmost(hwnd);

    let (tx, rx) = mpsc::channel();
    HELLO_RUNNING.store(true, Ordering::SeqCst);
    let hello_raw = hwnd.0 as isize;
    thread::spawn(move || {
        let report = consent::request(hello_raw);
        let _ = tx.send(report);
    });
    foreground::spawn_poller();

    let started = Instant::now();
    let mut saw_report = false;
    let mut observe_until: Option<Instant> = None;
    let mut last_raise = Instant::now();
    let mut showing_pass = false;
    let mut exit_code = 0;

    while started.elapsed() < limit && !STOP.load(Ordering::SeqCst) {
        window::pump();
        if hooks::escape_pressed() {
            println!("escape chord pressed");
            break;
        }
        if !saw_report && let Ok(report) = rx.try_recv() {
            saw_report = true;
            HELLO_RUNNING.store(false, Ordering::SeqCst);
            hooks::close_passthrough();
            foreground::stop_poller();
            println!("{report}");
            window::set_text(&format!("{report}\n\n{}", window::TEXT_OBSERVE));
            showing_pass = false;
            let remaining = limit.saturating_sub(started.elapsed());
            let observe = OBSERVE.min(remaining);
            observe_until = Some(Instant::now() + observe);
        }
        if let Some(until) = observe_until {
            if Instant::now() >= until {
                break;
            }
        } else {
            let open = hooks::passthrough_is_open();
            if open && !showing_pass {
                window::set_text(window::TEXT_PASS);
                showing_pass = true;
            } else if !open && showing_pass {
                window::set_text(window::TEXT_TIMEOUT);
                showing_pass = false;
            }
            if !open && last_raise.elapsed() >= Duration::from_millis(250) {
                window::raise_topmost(hwnd);
                last_raise = Instant::now();
            }
        }
        thread::sleep(Duration::from_millis(15));
    }

    if !saw_report && started.elapsed() >= limit {
        println!("stopped because the 3 minute limit was reached");
        exit_code = 3;
    }
    HELLO_RUNNING.store(false, Ordering::SeqCst);
    foreground::stop_poller();
    drop(armed);
    println!("keyboard and mouse restored");
    let _ = io::stdout().flush();
    exit_code
}

fn smoke_session() -> i32 {
    println!(
        "smoke: injected input only, limit {} seconds",
        SMOKE_LIMIT.as_secs()
    );
    let limit = clamp_block_limit(SMOKE_LIMIT);
    let armed = match arm(limit) {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("could not install the input hook: {error}");
            return 2;
        }
    };
    let hwnd = match window::create_probe() {
        Ok(hwnd) => hwnd,
        Err(error) => {
            eprintln!("could not create the probe window: {error}");
            drop(armed);
            return 2;
        }
    };
    window::focus(hwnd);

    let started = Instant::now();
    let mut sent_key = false;
    let mut sent_chord = false;
    let give_up = Duration::from_secs(2).min(limit);
    while started.elapsed() < give_up && !STOP.load(Ordering::SeqCst) {
        window::pump();
        let elapsed = started.elapsed();
        if !sent_key && elapsed >= Duration::from_millis(200) {
            window::send_key(0x41, true);
            window::send_key(0x41, false);
            sent_key = true;
        }
        if sent_key && !sent_chord && elapsed >= Duration::from_millis(600) {
            // Injected escape chord must not end the spike.
            for vk in [0xA2_u16, 0xA4, 0xA0, 0x51] {
                window::send_key(vk, true);
            }
            for vk in [0x51_u16, 0xA0, 0xA4, 0xA2] {
                window::send_key(vk, false);
            }
            sent_chord = true;
        }
        if sent_chord && window::probe_saw_key() && elapsed >= Duration::from_millis(900) {
            break;
        }
        thread::sleep(Duration::from_millis(15));
    }

    let passed = hooks::injected_passes();
    let saw = window::probe_saw_key();
    let escaped = hooks::escape_pressed();
    let ours = window::is_foreground(hwnd);
    drop(armed);

    println!("smoke injected passes: {passed}");
    println!("smoke probe saw key: {saw}");
    println!("smoke probe was foreground: {ours}");
    println!("smoke escape flag: {escaped}");
    // A runner that will not give us the foreground still proves the hook
    // passed injected keys. When the probe is foreground, the key has to land.
    if passed == 0 || escaped || !sent_chord || (ours && !saw) {
        eprintln!("smoke failed");
        return 4;
    }
    println!("smoke ok");
    0
}

fn arm(limit: Duration) -> windows::core::Result<HookGuard> {
    let guard = hooks::install()?;
    hooks::watchdog(limit);
    hooks::install_console_handler();
    if let Err(error) = hooks::raise_priority() {
        eprintln!("could not raise the hook thread priority: {error}");
    }
    Ok(guard)
}
