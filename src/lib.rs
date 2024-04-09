use std::ffi::c_void;
use std::{mem, ptr, thread};

use hudhook::hooks::dx11::ImguiDx11Hooks;
use hudhook::hooks::dx12::ImguiDx12Hooks;
use hudhook::hooks::dx9::ImguiDx9Hooks;
use hudhook::hooks::opengl3::ImguiOpenGl3Hooks;
use hudhook::imgui::{Condition, Ui};
use hudhook::mh::{MH_ApplyQueued, MH_Initialize, MhHook, MH_STATUS};
use hudhook::{Hudhook, HudhookBuilder, ImguiRenderLoop};

use once_cell::sync::{Lazy, OnceCell};
use parking_lot::Mutex;
use windows::core::{GUID, HRESULT};
use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::Win32::UI::Input::XboxController::{
    XINPUT_GAMEPAD, XINPUT_GAMEPAD_A, XINPUT_GAMEPAD_B, XINPUT_GAMEPAD_BACK,
    XINPUT_GAMEPAD_BUTTON_FLAGS, XINPUT_GAMEPAD_DPAD_DOWN, XINPUT_GAMEPAD_DPAD_LEFT,
    XINPUT_GAMEPAD_DPAD_RIGHT, XINPUT_GAMEPAD_DPAD_UP, XINPUT_GAMEPAD_LEFT_SHOULDER,
    XINPUT_GAMEPAD_LEFT_THUMB, XINPUT_GAMEPAD_RIGHT_SHOULDER, XINPUT_GAMEPAD_RIGHT_THUMB,
    XINPUT_GAMEPAD_START, XINPUT_GAMEPAD_X, XINPUT_GAMEPAD_Y,
};
use windows::{
    core::{s, w, PCWSTR},
    Win32::{
        Foundation::MAX_PATH,
        System::LibraryLoader::{GetProcAddress, LoadLibraryW},
        UI::Input::XboxController::XINPUT_STATE,
    },
};

type FDirectInput8Create = unsafe extern "stdcall" fn(
    hinst: HINSTANCE,
    dwversion: u32,
    riidltf: *const GUID,
    ppvout: *mut *mut c_void,
    punkouter: HINSTANCE,
) -> HRESULT;

static DIRECTINPUT8CREATE: Lazy<FDirectInput8Create> = Lazy::new(|| unsafe {
    let mut dinput8_path = [0u16; MAX_PATH as usize];
    let count = GetSystemDirectoryW(Some(&mut dinput8_path)) as usize;

    // If count == 0, this will be fun
    std::ptr::copy_nonoverlapping(w!("\\dinput8.dll").0, dinput8_path[count..].as_mut_ptr(), 12);

    let dinput8 = LoadLibraryW(PCWSTR(dinput8_path.as_ptr())).unwrap();

    std::mem::transmute(GetProcAddress(dinput8, s!("DirectInput8Create")))
});

#[no_mangle]
unsafe extern "stdcall" fn DirectInput8Create(
    hinst: HINSTANCE,
    dwversion: u32,
    riidltf: *const GUID,
    ppvout: *mut *mut c_void,
    punkouter: HINSTANCE,
) -> HRESULT {
    (DIRECTINPUT8CREATE)(hinst, dwversion, riidltf, ppvout, punkouter)
}

type FXInputGetState =
    unsafe extern "stdcall" fn(dw_user_index: u32, xinput_state: *mut XINPUT_STATE) -> u32;

static XINPUTGETSTATE_TRAMPOLINE: OnceCell<FXInputGetState> = OnceCell::new();
static XINPUT_CURRENT_STATE: Mutex<XINPUT_STATE> = Mutex::new(XINPUT_STATE {
    dwPacketNumber: 0,
    Gamepad: XINPUT_GAMEPAD {
        wButtons: XINPUT_GAMEPAD_BUTTON_FLAGS(0),
        bLeftTrigger: 0,
        bRightTrigger: 0,
        sThumbLX: 0,
        sThumbLY: 0,
        sThumbRX: 0,
        sThumbRY: 0,
    },
});

unsafe extern "stdcall" fn xinput_get_state_impl(
    dw_user_index: u32,
    xinput_state: *mut XINPUT_STATE,
) -> u32 {
    let r = (XINPUTGETSTATE_TRAMPOLINE.get().unwrap())(dw_user_index, xinput_state);

    if !xinput_state.is_null() {
        *XINPUT_CURRENT_STATE.lock() = *xinput_state;
    }

    r
}

unsafe fn hook() {
    let mut path = [0u16; MAX_PATH as usize];
    let count = GetSystemDirectoryW(Some(&mut path)) as usize;

    ptr::copy_nonoverlapping(w!("\\xinput1_3.dll").0, path[count..].as_mut_ptr(), 14);

    let lib = LoadLibraryW(PCWSTR(path.as_ptr())).unwrap();

    let xinput_get_state_addr = GetProcAddress(lib, s!("XInputGetState")).unwrap();

    match MH_Initialize() {
        MH_STATUS::MH_ERROR_ALREADY_INITIALIZED | MH_STATUS::MH_OK => {},
        status @ MH_STATUS::MH_ERROR_MEMORY_ALLOC => {
            eprintln!("MH_Initialize: {status:?}");
            return;
        },
        _ => unreachable!(),
    }

    let hook = match MhHook::new(
        xinput_get_state_addr as *mut c_void,
        xinput_get_state_impl as *mut c_void,
    ) {
        Ok(hook) => hook,
        Err(e) => {
            eprintln!("New hook: {e:?}");
            return;
        },
    };

    if let Err(e) = hook.queue_enable() {
        eprintln!("Hook queue enable: {e:?}");
        return;
    }

    if let Err(e) = MH_ApplyQueued().ok() {
        eprintln!("Hook apply queued: {e:?}");
        return;
    }

    if let Err(e) = XINPUTGETSTATE_TRAMPOLINE.set(mem::transmute(hook.trampoline())) {
        eprintln!("Set trampoline: {e:?}");
    }
}

#[derive(Default)]
struct XInputViewer {
    text: String,
}

impl ImguiRenderLoop for XInputViewer {
    fn render(&mut self, ui: &mut Ui) {
        let mut state = { XINPUT_CURRENT_STATE.lock().Gamepad };

        ui.window("XInput State")
            .position([16.0, 16.0], Condition::FirstUseEver)
            .size([530., 175.], Condition::FirstUseEver)
            .resizable(false)
            .movable(false)
            .collapsible(false)
            .title_bar(false)
            .build(|| {
                self.text.clear();

                let flag = |f, s| {
                    ui.checkbox(s, &mut state.wButtons.contains(f));
                };

                ui.columns(3, "##columns", false);

                ui.set_column_width(0, 150.);
                ui.set_column_width(1, 150.);
                ui.set_column_width(2, 230.);

                flag(XINPUT_GAMEPAD_DPAD_UP, "DPad Up");
                flag(XINPUT_GAMEPAD_DPAD_DOWN, "DPad Down");
                flag(XINPUT_GAMEPAD_DPAD_LEFT, "DPad Left");
                flag(XINPUT_GAMEPAD_DPAD_RIGHT, "DPad Right");
                flag(XINPUT_GAMEPAD_START, "Start");
                flag(XINPUT_GAMEPAD_LEFT_SHOULDER, "L Shoulder");
                flag(XINPUT_GAMEPAD_LEFT_THUMB, "Stick L Button");

                ui.next_column();

                flag(XINPUT_GAMEPAD_A, "A");
                flag(XINPUT_GAMEPAD_B, "B");
                flag(XINPUT_GAMEPAD_X, "X");
                flag(XINPUT_GAMEPAD_Y, "Y");
                flag(XINPUT_GAMEPAD_BACK, "Back");
                flag(XINPUT_GAMEPAD_RIGHT_SHOULDER, "R Shoulder");
                flag(XINPUT_GAMEPAD_RIGHT_THUMB, "Stick R Button");

                ui.next_column();

                ui.slider_config("L Horiz", i16::MIN, i16::MAX).build(&mut state.sThumbLX);
                ui.slider_config("L Vert", i16::MIN, i16::MAX).build(&mut state.sThumbLY);
                ui.slider_config("R Horiz", i16::MIN, i16::MAX).build(&mut state.sThumbRX);
                ui.slider_config("R Vert", i16::MIN, i16::MAX).build(&mut state.sThumbRY);
                ui.slider_config("L Trigger", u8::MIN, u8::MAX).build(&mut state.bLeftTrigger);
                ui.slider_config("R Trigger", u8::MIN, u8::MAX).build(&mut state.bRightTrigger);

                ui.next_column();

                ui.columns(1, "##columns1", false);
            });
    }
}

fn hudhook_detect_backend() -> HudhookBuilder {
    let xinput_viewer = XInputViewer::default();

    if unsafe { GetModuleHandleW(w!("D3D12Core.dll")) }.is_ok() {
        Hudhook::builder().with::<ImguiDx12Hooks>(xinput_viewer)
    } else if unsafe { GetModuleHandleW(w!("d3d11.dll")) }.is_ok() {
        Hudhook::builder().with::<ImguiDx11Hooks>(xinput_viewer)
    } else if unsafe { GetModuleHandleW(w!("d3d9.dll")) }.is_ok() {
        Hudhook::builder().with::<ImguiDx9Hooks>(xinput_viewer)
    } else if unsafe { GetModuleHandleW(w!("OPENGL32.dll")) }.is_ok() {
        Hudhook::builder().with::<ImguiOpenGl3Hooks>(xinput_viewer)
    } else {
        panic!("Couldn't determine backend");
    }
}

#[no_mangle]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "stdcall" fn DllMain(hmodule: HINSTANCE, reason: u32, _: *mut c_void) {
    if reason == DLL_PROCESS_ATTACH {
        Lazy::force(&DIRECTINPUT8CREATE);

        thread::spawn(move || {
            hook();

            if let Err(e) = hudhook_detect_backend().with_hmodule(hmodule).build().apply() {
                eprintln!("Couldn't apply hooks: {e:?}");
            }
        });
    }
}
