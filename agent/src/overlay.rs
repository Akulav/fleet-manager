//! On-screen "your screen is being viewed" indicator.
//!
//! A transparency feature: whenever the master is capturing or live-viewing the
//! desktop, the user sees a red frame around the whole (virtual) screen and a
//! red bar at the top with a watermark. It is click-through and never takes
//! focus, so it doesn't get in the user's way; it only makes remote viewing
//! impossible to miss.
//!
//! One background thread owns a borderless, top-most, layered, click-through
//! window covering all monitors. The rest of the helper just flips two atomics:
//! [`set_viewing`] while a live stream runs, and [`flash`] for a one-off
//! screenshot. A 150 ms timer in the window reflects them.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    InvalidateRect, SelectObject, SetBkMode, SetTextColor, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
    DEFAULT_QUALITY, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FF_DONTCARE, FW_BOLD,
    HFONT, OUT_DEFAULT_PRECIS, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

static VIEWING: AtomicBool = AtomicBool::new(false);
/// Unix-millis until which the flash (one screenshot) stays visible.
static FLASH_UNTIL: AtomicU64 = AtomicU64::new(0);
static STARTED: OnceLock<()> = OnceLock::new();
static WATERMARK: OnceLock<Vec<u16>> = OnceLock::new();

const BORDER: i32 = 6;
const BAR_H: i32 = 38;
const RED: COLORREF = COLORREF(0x00_30_30_E0); // BGR: a strong red
const KEY: COLORREF = COLORREF(0x00_FF_00_FF); // magenta → transparent
const TIMER: usize = 1;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Start the overlay thread once, with the watermark text to display.
pub fn init(watermark: &str) {
    let _ = WATERMARK.set(watermark.encode_utf16().chain(std::iter::once(0)).collect());
    STARTED.get_or_init(|| {
        std::thread::spawn(|| unsafe { run() });
    });
}

/// Show/hide the indicator for an ongoing live view.
pub fn set_viewing(on: bool) {
    VIEWING.store(on, Ordering::Relaxed);
}

/// Flash the indicator briefly for a single screenshot.
pub fn flash() {
    FLASH_UNTIL.store(now_ms() + 1500, Ordering::Relaxed);
}

fn wanted() -> bool {
    VIEWING.load(Ordering::Relaxed) || now_ms() < FLASH_UNTIL.load(Ordering::Relaxed)
}

unsafe fn run() {
    let Ok(hmodule) = GetModuleHandleW(None) else {
        return;
    };
    let hinstance = windows::Win32::Foundation::HINSTANCE(hmodule.0);
    let class = w!("FleetViewOverlay");
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wndproc),
        hInstance: hinstance,
        lpszClassName: class,
        ..Default::default()
    };
    RegisterClassExW(&wc);

    let (x, y, w, h) = virtual_screen();
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT, // click-through
        class,
        w!("FleetView"),
        WS_POPUP,
        x,
        y,
        w,
        h,
        None,
        None,
        hinstance,
        None,
    );
    let Ok(hwnd) = hwnd else { return };
    // Magenta becomes fully transparent; everything we paint red stays opaque.
    let _ = SetLayeredWindowAttributes(hwnd, KEY, 0, LWA_COLORKEY);
    SetTimer(hwnd, TIMER, 150, None);

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

fn virtual_screen() -> (i32, i32, i32, i32) {
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN).max(640),
            GetSystemMetrics(SM_CYVIRTUALSCREEN).max(480),
        )
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER => {
                let show = wanted();
                let visible = IsWindowVisible(hwnd).as_bool();
                if show && !visible {
                    // Re-cover the current virtual screen (resolution/monitors
                    // may have changed) and come back to the top.
                    let (x, y, w, h) = virtual_screen();
                    let _ = SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE);
                    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                    let _ = InvalidateRect(hwnd, None, true);
                } else if show {
                    // Keep asserting top-most over full-screen apps.
                    let _ = SetWindowPos(
                        hwnd,
                        HWND_TOPMOST,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                    );
                } else if visible {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                LRESULT(0)
            }
            WM_PAINT => {
                paint(hwnd);
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

unsafe fn paint(hwnd: HWND) {
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let (w, h) = (rc.right, rc.bottom);

    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);

    // Whole surface transparent (magenta key), then paint only the frame.
    let key = CreateSolidBrush(KEY);
    FillRect(hdc, &rc, key);
    let _ = DeleteObject(key);

    let red = CreateSolidBrush(RED);
    let strips = [
        RECT {
            left: 0,
            top: 0,
            right: w,
            bottom: BAR_H,
        }, // top bar (covers top border)
        RECT {
            left: 0,
            top: h - BORDER,
            right: w,
            bottom: h,
        }, // bottom
        RECT {
            left: 0,
            top: 0,
            right: BORDER,
            bottom: h,
        }, // left
        RECT {
            left: w - BORDER,
            top: 0,
            right: w,
            bottom: h,
        }, // right
    ];
    for s in &strips {
        FillRect(hdc, s, red);
    }
    let _ = DeleteObject(red);

    // Watermark text, centered in the top bar.
    if let Some(text) = WATERMARK.get() {
        let font = make_font(22);
        let old = SelectObject(hdc, font);
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, COLORREF(0x00_FF_FF_FF)); // white
        let mut bar = RECT {
            left: 0,
            top: 0,
            right: w,
            bottom: BAR_H,
        };
        let mut t = text.clone();
        DrawTextW(
            hdc,
            t.as_mut_slice(),
            &mut bar,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        SelectObject(hdc, old);
        let _ = DeleteObject(font);
    }

    let _ = EndPaint(hwnd, &ps);
}

unsafe fn make_font(height: i32) -> HFONT {
    CreateFontW(
        -height,
        0,
        0,
        0,
        FW_BOLD.0 as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET.0 as u32,
        OUT_DEFAULT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        DEFAULT_QUALITY.0 as u32,
        FF_DONTCARE.0 as u32,
        w!("Segoe UI"),
    )
}
