//! Injected chords. Nothing here swallows a physical key.

use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_LWIN,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::PCWSTR;

use crate::tour::{Chord, Key};
use crate::win::wide;

pub fn chord(chord: Chord) -> Result<(), String> {
    if chord.win {
        key(VK_LWIN, false, true)?;
    }
    if let Some(key_name) = chord.key {
        key(virtual_key(key_name), false, false)?;
    }
    thread::sleep(Duration::from_millis(40));
    if let Some(key_name) = chord.key {
        key(virtual_key(key_name), true, false)?;
    }
    if chord.win {
        key(VK_LWIN, true, true)?;
    }
    Ok(())
}

pub fn key(vk: VIRTUAL_KEY, up: bool, extended: bool) -> Result<(), String> {
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if up {
        flags.0 |= KEYEVENTF_KEYUP.0;
    }
    if extended {
        flags.0 |= KEYEVENTF_EXTENDEDKEY.0;
    }
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
    if sent == 1 {
        Ok(())
    } else {
        Err("injected key was not accepted".into())
    }
}

pub fn launch_uac(file: &str, params: &str) -> Result<(), String> {
    let file = wide(file);
    let params = wide(params);
    let verb = wide("runas");
    let result = unsafe {
        ShellExecuteW(
            None::<HWND>,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR(params.as_ptr()),
            PCWSTR::null(),
            SW_HIDE,
        )
    };
    let code = result.0 as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(format!(
            "the consent prompt returned {code}. Click No if it is still showing."
        ))
    }
}

fn virtual_key(key: Key) -> VIRTUAL_KEY {
    match key {
        Key::Tab => windows::Win32::UI::Input::KeyboardAndMouse::VK_TAB,
        Key::N => windows::Win32::UI::Input::KeyboardAndMouse::VK_N,
        Key::A => windows::Win32::UI::Input::KeyboardAndMouse::VK_A,
        Key::P => windows::Win32::UI::Input::KeyboardAndMouse::VK_P,
    }
}
