//! A simple startup splash window shown in the user's session: a borderless,
//! top-most window that displays the FleetView splash image for a few seconds
//! and then closes itself. Its purpose is transparency — the user sees, at
//! logon, that the management agent is running.
//!
//! Implemented directly on Win32/GDI so it is self-contained and does not fight
//! the tray's message loop (it runs on its own thread with its own loop).

use std::sync::OnceLock;

use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Decoded splash image, BGRA top-down, with dimensions.
struct SplashImage {
    width: i32,
    height: i32,
    bgra: Vec<u8>,
}

static IMAGE: OnceLock<SplashImage> = OnceLock::new();

const SPLASH_PNG: &[u8] = include_bytes!("../../installer/assets/splash.png");
const TIMER_ID: usize = 1;
const SHOW_MS: u32 = 3500;

fn image() -> &'static SplashImage {
    IMAGE.get_or_init(|| {
        match image::load_from_memory(SPLASH_PNG) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let (w, h) = (rgba.width() as i32, rgba.height() as i32);
                let mut bgra = rgba.into_raw();
                // RGBA -> BGRA in place.
                for px in bgra.as_chunks_mut::<4>().0 {
                    px.swap(0, 2);
                }
                SplashImage {
                    width: w,
                    height: h,
                    bgra,
                }
            }
            Err(_) => SplashImage {
                width: 480,
                height: 240,
                bgra: vec![32u8; 480 * 240 * 4],
            },
        }
    })
}

/// Show the splash and block until it closes. Call on a dedicated thread.
pub fn show_blocking() {
    unsafe {
        let hinstance = match GetModuleHandleW(None) {
            Ok(h) => h,
            Err(_) => return,
        };
        let class_name = w!("FleetViewSplash");
        let hinst = HINSTANCE(hinstance.0);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassExW(&wc);

        let img = image();
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);
        let x = (screen_w - img.width) / 2;
        let y = (screen_h - img.height) / 2;

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class_name,
            w!("FleetView"),
            WS_POPUP,
            x,
            y,
            img.width,
            img.height,
            HWND::default(),
            HMENU::default(),
            hinst,
            None,
        );
        let Ok(hwnd) = hwnd else { return };

        let _ = ShowWindow(hwnd, SW_SHOW);
        SetTimer(hwnd, TIMER_ID, SHOW_MS, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                let img = image();
                let bmi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: img.width,
                        biHeight: -img.height, // top-down
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                StretchDIBits(
                    hdc,
                    0,
                    0,
                    img.width,
                    img.height,
                    0,
                    0,
                    img.width,
                    img.height,
                    Some(img.bgra.as_ptr() as *const _),
                    &bmi,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_TIMER => {
                let _ = KillTimer(hwnd, TIMER_ID);
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
