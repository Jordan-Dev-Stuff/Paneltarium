//! Live modifier-key state via `GetAsyncKeyState`.

use pt_core::{KeyboardState, Modifiers};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};

pub struct WindowsKeyboard;

fn down(vk: VIRTUAL_KEY) -> bool {
    // The high bit is set while the key is physically down.
    (unsafe { GetAsyncKeyState(vk.0 as i32) } as u16) & 0x8000 != 0
}

impl KeyboardState for WindowsKeyboard {
    fn modifiers_held(&self) -> Option<Modifiers> {
        Some(Modifiers {
            ctrl: down(VK_CONTROL),
            alt: down(VK_MENU),
            shift: down(VK_SHIFT),
            win: down(VK_LWIN) || down(VK_RWIN),
        })
    }
}
