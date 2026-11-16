use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY,
};

use super::util::last_error;

const BATCH: usize = 200;

pub fn send_key_pairs(vk: VIRTUAL_KEY, extra: usize, pairs: u32) -> Result<u32, String> {
    if pairs == 0 {
        return Ok(0);
    }
    let mut pending = Vec::with_capacity(BATCH);
    let mut inserted = 0u32;
    for _ in 0..pairs {
        pending.push(key_event(vk, extra, false));
        pending.push(key_event(vk, extra, true));
        if pending.len() == BATCH {
            inserted = inserted.saturating_add(flush(&mut pending)?);
        }
    }
    if !pending.is_empty() {
        inserted = inserted.saturating_add(flush(&mut pending)?);
    }
    Ok(inserted)
}

fn flush(pending: &mut Vec<INPUT>) -> Result<u32, String> {
    let count = pending.len() as u32;
    let wrote = unsafe { SendInput(pending, std::mem::size_of::<INPUT>() as i32) };
    pending.clear();
    if wrote != count {
        return Err(if wrote == 0 {
            last_error("SendInput")
        } else {
            format!("SendInput inserted {wrote} of {count}")
        });
    }
    Ok(wrote)
}

fn key_event(vk: VIRTUAL_KEY, extra: usize, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    Default::default()
                },
                time: 0,
                dwExtraInfo: extra,
            },
        },
    }
}

pub fn send_alt_keyup() {
    let event = key_event(
        windows::Win32::UI::Input::KeyboardAndMouse::VK_MENU,
        0,
        true,
    );
    unsafe {
        let _ = SendInput(&[event], std::mem::size_of::<INPUT>() as i32);
    }
}
