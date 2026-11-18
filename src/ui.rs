use crate::app::{self, Command, Shared};
use std::{
    cell::RefCell,
    io,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::{Arc, atomic::Ordering},
};
use windows_sys::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
    core::w,
};

struct Window {
    shared: Arc<Shared>,
    report: HWND,
    test: HWND,
    auto: HWND,
    last_text: String,
}
thread_local! { static WINDOW: RefCell<Option<Window>> = const { RefCell::new(None) }; }

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub fn show_error(text: &str) {
    // SAFETY: valid null-terminated strings for the duration of the modal call.
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(text).as_ptr(),
            w!("RAM Observer"),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn set_text(window: HWND, text: &str) {
    // SAFETY: UTF-16 string remains alive through the synchronous API call.
    unsafe {
        SetWindowTextW(window, wide(text).as_ptr());
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: Windows invokes this procedure on its owning thread with valid message arguments.
    unsafe {
        match msg {
            WM_GETMINMAXINFO => {
                let info = &mut *(lp as *mut MINMAXINFO);
                info.ptMinTrackSize = POINT { x: 620, y: 480 };
                return 0;
            }
            WM_SIZE => {
                WINDOW.with(|slot| {
                    if let Some(state) = slot.borrow().as_ref() {
                        let mut rect: RECT = zeroed();
                        GetClientRect(hwnd, &mut rect);
                        MoveWindow(state.test, 16, 14, 245, 34, 1);
                        MoveWindow(state.auto, 277, 14, 300, 34, 1);
                        MoveWindow(
                            state.report,
                            16,
                            62,
                            (rect.right - 32).max(1),
                            (rect.bottom - 78).max(1),
                            1,
                        );
                    }
                });
                return 0;
            }
            WM_COMMAND => {
                // Edit notifications can re-enter while the report is being updated.
                if wp >> 16 != 0 || !matches!(wp & 0xffff, 1 | 2) {
                    return 0;
                }
                WINDOW.with(|slot| {
                    if let Some(state) = slot.borrow().as_ref() {
                        match wp & 0xffff {
                            1 => {
                                if !state.shared.busy.swap(true, Ordering::SeqCst) {
                                    if state.shared.commands.try_send(Command::Experiment).is_ok() {
                                        set_text(state.test, "Running controlled test...");
                                        EnableWindow(state.test, 0);
                                    } else {
                                        state.shared.busy.store(false, Ordering::SeqCst);
                                    }
                                }
                            }
                            2 => {
                                let on = !state.shared.auto.fetch_xor(true, Ordering::SeqCst);
                                set_text(
                                    state.auto,
                                    if on {
                                        "&Auto (test process only): ON"
                                    } else {
                                        "&Auto (test process only): OFF"
                                    },
                                );
                            }
                            _ => {}
                        }
                    }
                });
                return 0;
            }
            WM_TIMER => {
                WINDOW.with(|slot| {
                    if let Some(state) = slot.borrow_mut().as_mut() {
                        if let Ok(text) = state.shared.text.try_lock()
                            && *text != state.last_text
                        {
                            set_text(state.report, &text);
                            state.last_text.clone_from(&text);
                        }
                        let busy = state.shared.busy.load(Ordering::SeqCst);
                        EnableWindow(state.test, i32::from(!busy));
                        set_text(
                            state.test,
                            if busy {
                                "Running controlled test..."
                            } else {
                                "Run controlled &test (64 MiB)"
                            },
                        );
                    }
                });
                return 0;
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                return 0;
            }
            WM_DESTROY => {
                KillTimer(hwnd, 1);
                WINDOW.with(|slot| {
                    if let Some(state) = slot.borrow_mut().take() {
                        state.shared.auto.store(false, Ordering::SeqCst);
                        state.shared.stop.store(true, Ordering::SeqCst);
                    }
                });
                PostQuitMessage(0);
                return 0;
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

pub fn run() -> io::Result<()> {
    // SAFETY: all windows/controls and message handling stay on this thread. Native controls
    // supply keyboard focus, high-contrast rendering and accessibility support.
    unsafe {
        SetProcessDPIAware();
        let instance = GetModuleHandleW(null());
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_BTNFACE + 1) as HBRUSH,
            lpszClassName: w!("RamObserverWindow"),
            ..zeroed()
        };
        if RegisterClassExW(&class) == 0 {
            return Err(io::Error::last_os_error());
        }
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class.lpszClassName,
            w!("RAM Observer - controlled experiment"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1000,
            820,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return Err(io::Error::last_os_error());
        }
        let test = CreateWindowExW(
            0,
            w!("BUTTON"),
            w!("Run controlled &test (64 MiB)"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            16,
            14,
            245,
            34,
            hwnd,
            1 as HMENU,
            instance,
            null(),
        );
        let auto = CreateWindowExW(
            0,
            w!("BUTTON"),
            w!("&Auto (test process only): OFF"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            277,
            14,
            300,
            34,
            hwnd,
            2 as HMENU,
            instance,
            null(),
        );
        let report = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            w!("EDIT"),
            w!("Reading memory..."),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_VSCROLL
                | ES_MULTILINE as u32
                | ES_READONLY as u32
                | ES_AUTOVSCROLL as u32,
            16,
            62,
            950,
            680,
            hwnd,
            3 as HMENU,
            instance,
            null(),
        );
        if test.is_null() || auto.is_null() || report.is_null() {
            let error = io::Error::last_os_error();
            DestroyWindow(hwnd);
            return Err(error);
        }
        let font = GetStockObject(DEFAULT_GUI_FONT);
        for control in [test, auto, report] {
            SendMessageW(control, WM_SETFONT, font as usize, 1);
        }
        SendMessageW(report, 0x00c5 /* EM_SETLIMITTEXT */, 65535, 0);
        WINDOW.with(|slot| {
            *slot.borrow_mut() = Some(Window {
                shared: app::start(),
                report,
                test,
                auto,
                last_text: String::new(),
            })
        });
        if SetTimer(hwnd, 1, 250, None) == 0 {
            let error = io::Error::last_os_error();
            DestroyWindow(hwnd);
            return Err(error);
        }
        ShowWindow(hwnd, SW_SHOW);
        SendMessageW(hwnd, WM_SIZE, 0, 0);
        SetFocus(test);
        let mut msg: MSG = zeroed();
        loop {
            let status = GetMessageW(&mut msg, null_mut(), 0, 0);
            if status == -1 {
                let error = io::Error::last_os_error();
                DestroyWindow(hwnd);
                return Err(error);
            }
            if status == 0 {
                break;
            }
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        Ok(())
    }
}
