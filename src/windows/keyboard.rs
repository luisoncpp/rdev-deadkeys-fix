use crate::rdev::{EventType, Key, KeyboardState};
use crate::windows::common::{FALSE, TRUE, get_code, get_scan_code};
use crate::windows::keycodes::code_from_key;
use std::ptr::null_mut;
use winapi::shared::minwindef::{BYTE, HKL, LPARAM, UINT};
use winapi::um::processthreadsapi::GetCurrentThreadId;
use winapi::um::winuser;
use winapi::um::winuser::{
    GetForegroundWindow, GetKeyState, GetKeyboardLayout, GetKeyboardState,
    GetWindowThreadProcessId, ToUnicodeEx, VK_CAPITAL, VK_LSHIFT, VK_RSHIFT, VK_SHIFT,
};

const VK_SHIFT_: usize = VK_SHIFT as usize;
const VK_CAPITAL_: usize = VK_CAPITAL as usize;
const VK_LSHIFT_: usize = VK_LSHIFT as usize;
const VK_RSHIFT_: usize = VK_RSHIFT as usize;
const HIGHBIT: u8 = 0x80;
const TUE_NOCONSUME: u32 = 4;
const BUF_LEN: i32 = 32;

pub struct Keyboard {
    last_state: [BYTE; 256],
}

impl Keyboard {
    pub fn new() -> Option<Keyboard> {
        Some(Keyboard {
            last_state: [0; 256],
        })
    }

    pub(crate) unsafe fn get_name(&mut self, lpdata: LPARAM) -> Option<String> {
        unsafe {
            // https://gist.github.com/akimsko/2011327
            // https://www.experts-exchange.com/questions/23453780/LowLevel-Keystroke-Hook-removes-Accents-on-French-Keyboard.html
            let code = get_code(lpdata);
            let scan_code = get_scan_code(lpdata);

            self.set_global_state()?;
            self.get_code_name(code, scan_code)
        }
    }

    pub(crate) unsafe fn set_global_state(&mut self) -> Option<()> {
        unsafe {
            let mut state = [0_u8; 256];
            let state_ptr = state.as_mut_ptr();

            let _shift = GetKeyState(VK_SHIFT);
            let current_window_thread_id =
                GetWindowThreadProcessId(GetForegroundWindow(), null_mut());
            let thread_id = GetCurrentThreadId();
            // Attach to active thread so we can get that keyboard state
            let status =
                if winuser::AttachThreadInput(thread_id, current_window_thread_id, TRUE) == 1 {
                    // Current state of the modifiers in keyboard
                    let status = GetKeyboardState(state_ptr);

                    // Detach
                    winuser::AttachThreadInput(thread_id, current_window_thread_id, FALSE);
                    status
                } else {
                    // Could not attach, perhaps it is this process?
                    GetKeyboardState(state_ptr)
                };

            if status != 1 {
                return None;
            }
            self.last_state = state;
            Some(())
        }
    }

    pub(crate) unsafe fn get_code_name(&mut self, code: UINT, scan_code: UINT) -> Option<String> {
        unsafe {
            let current_window_thread_id =
                GetWindowThreadProcessId(GetForegroundWindow(), null_mut());
            let layout = GetKeyboardLayout(current_window_thread_id);
            self.translate_with_layout(code, scan_code, layout)
        }
    }

    pub(crate) unsafe fn translate_with_layout(
        &mut self,
        code: UINT,
        scan_code: UINT,
        layout: HKL,
    ) -> Option<String> {
        unsafe {
            let state_ptr = self.last_state.as_mut_ptr();

            let mut buff = [0_u16; BUF_LEN as usize];
            let buff_ptr = buff.as_mut_ptr();
            // Single ToUnicodeEx call with TUE_NOCONSUME: a low-level hook runs
            // before the foreground app translates the key, and any call WITHOUT
            // this flag mutates the kernel's dead-key buffer — a dead key would
            // then resolve immediately (dead+dead => "``") instead of composing.
            let len = ToUnicodeEx(
                code,
                scan_code,
                state_ptr,
                buff_ptr,
                BUF_LEN,
                TUE_NOCONSUME,
                layout,
            );

            if len == -1 {
                // TODO(debug-log): remove after the dead-key fix is confirmed.
                eprintln!("[rdev] dead key detected (vk={code}), reporting no character");
            }

            match len {
                len if len > 0 => String::from_utf16(&buff[..len as usize]).ok(),
                _ => None, // 0 = no translation, -1 = dead key pending composition
            }
        }
    }
}

impl KeyboardState for Keyboard {
    fn add(&mut self, event_type: &EventType) -> Option<String> {
        match event_type {
            EventType::KeyPress(key) => match key {
                Key::ShiftLeft => {
                    self.last_state[VK_SHIFT_] |= HIGHBIT;
                    self.last_state[VK_LSHIFT_] |= HIGHBIT;
                    None
                }
                Key::ShiftRight => {
                    self.last_state[VK_SHIFT_] |= HIGHBIT;
                    self.last_state[VK_RSHIFT_] |= HIGHBIT;
                    None
                }
                Key::CapsLock => {
                    self.last_state[VK_CAPITAL_] ^= 1;
                    None
                }
                key => {
                    let code = code_from_key(*key)?;
                    unsafe { self.get_code_name(code.into(), 0) }
                }
            },
            EventType::KeyRelease(key) => match key {
                Key::ShiftLeft => {
                    self.last_state[VK_SHIFT_] &= !HIGHBIT;
                    self.last_state[VK_LSHIFT_] &= !HIGHBIT;
                    None
                }
                Key::ShiftRight => {
                    self.last_state[VK_SHIFT_] &= !HIGHBIT;
                    self.last_state[VK_RSHIFT_] &= HIGHBIT;
                    None
                }
                _ => None,
            },

            _ => None,
        }
    }

    fn reset(&mut self) {
        self.last_state[16] = 0;
        self.last_state[20] = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use winapi::um::winuser::{LoadKeyboardLayoutW, MapVirtualKeyExW, VK_CONTROL, VK_MENU, VK_SPACE};

    const LATAM_KLID: &str = "0000080A"; // Spanish (Latin America) — AltGr+} is the dead grave `
    const MAPVK_VK_TO_VSC: u32 = 0;
    const DEAD_KEY: i32 = -1;
    const VK_A: UINT = 0x41;
    const FLUSH_ATTEMPTS: usize = 4;

    /// The kernel's dead-key buffer persists per layout across processes, so a
    /// previously planted dead key (e.g. by a buggy hook) would poison every
    /// assertion below. Consume any pending dead key before testing.
    fn flush_pending_dead_key(layout: HKL) {
        let scan_code = unsafe { MapVirtualKeyExW(VK_SPACE as u32, MAPVK_VK_TO_VSC, layout) };
        let mut state = [0_u8; 256];
        let mut buff = [0_u16; BUF_LEN as usize];
        for _ in 0..FLUSH_ATTEMPTS {
            let len = unsafe {
                ToUnicodeEx(
                    VK_SPACE as UINT,
                    scan_code,
                    state.as_mut_ptr(),
                    buff.as_mut_ptr(),
                    BUF_LEN,
                    /*flags=*/ 0,
                    layout,
                )
            };
            if len >= 0 {
                return;
            }
        }
    }

    fn load_latam_layout() -> HKL {
        let wide: Vec<u16> = LATAM_KLID.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe { LoadKeyboardLayoutW(wide.as_ptr(), 0) }
    }

    fn altgr_state() -> [BYTE; 256] {
        let mut state = [0_u8; 256];
        state[VK_CONTROL as usize] = HIGHBIT;
        state[VK_MENU as usize] = HIGHBIT;
        state
    }

    /// Probe (without consuming kernel state) for a VK that is a dead key under AltGr.
    fn find_dead_altgr_vk(layout: HKL) -> Option<(UINT, UINT)> {
        let mut state = altgr_state();
        for vk in 1..256_u32 {
            let scan_code = unsafe { MapVirtualKeyExW(vk, MAPVK_VK_TO_VSC, layout) };
            if scan_code == 0 {
                continue;
            }
            let mut buff = [0_u16; BUF_LEN as usize];
            let len = unsafe {
                ToUnicodeEx(
                    vk,
                    scan_code,
                    state.as_mut_ptr(),
                    buff.as_mut_ptr(),
                    BUF_LEN,
                    TUE_NOCONSUME,
                    layout,
                )
            };
            if len == DEAD_KEY {
                return Some((vk, scan_code));
            }
        }
        None
    }

    /// Regression test: pressing a dead key (e.g. AltGr+} = dead grave on the
    /// Latin American layout) must report no character AND must not consume the
    /// kernel's dead-key buffer. The old implementation called ToUnicodeEx
    /// without TUE_NOCONSUME first, which planted a pending dead key; combined
    /// with the second call it produced "``" and made the foreground app type
    /// two backticks immediately instead of composing.
    #[test]
    #[serial]
    fn dead_key_press_reports_nothing_and_leaves_kernel_state_intact() {
        let layout = load_latam_layout();
        assert!(!layout.is_null(), "could not load Latin American layout");
        flush_pending_dead_key(layout);
        let (dead_vk, dead_scan) =
            find_dead_altgr_vk(layout).expect("no dead AltGr key found in latam layout");

        let mut keyboard = Keyboard::new().unwrap();
        keyboard.last_state = altgr_state();

        let first = unsafe { keyboard.translate_with_layout(dead_vk, dead_scan, layout) };
        assert_eq!(first, None, "dead key press must not produce a character");

        // If the first call consumed/planted a dead key, this second translation
        // combines dead+dead and returns "``" — the exact reported bug.
        let mut second_keyboard = Keyboard::new().unwrap();
        second_keyboard.last_state = altgr_state();
        let second = unsafe { second_keyboard.translate_with_layout(dead_vk, dead_scan, layout) };
        assert_eq!(second, None, "kernel dead-key buffer was corrupted by previous translation");
    }

    /// Sanity check that the translation seam works at all for a plain key.
    #[test]
    #[serial]
    fn plain_key_translates_to_lowercase_letter() {
        let layout = load_latam_layout();
        assert!(!layout.is_null(), "could not load Latin American layout");
        flush_pending_dead_key(layout);
        let scan_code = unsafe { MapVirtualKeyExW(VK_A, MAPVK_VK_TO_VSC, layout) };

        let mut keyboard = Keyboard::new().unwrap();
        let name = unsafe { keyboard.translate_with_layout(VK_A, scan_code, layout) };
        assert_eq!(name.as_deref(), Some("a"));
    }
}
