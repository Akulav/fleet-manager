//! Custom notification toasts shown to the logged-in user.
//!
//! Replaces the old `MessageBoxW` popups, which appeared behind the active
//! window and looked like a system error. A toast here is a small, rounded,
//! top-most card in the bottom-right corner with the FleetView icon, a bold
//! title and a short body. It never steals focus (`WS_EX_NOACTIVATE`), closes
//! itself after a few seconds, and dismisses on click. Several toasts stack
//! upward. Drawn directly on Win32/GDI so it needs no extra dependency and
//! doesn't disturb the tray's own message loop (each toast runs on its thread).

use std::sync::{Mutex, OnceLock};

use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    SelectObject, SetBkMode, SetTextColor, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_QUALITY, DIB_RGB_COLORS, DT_END_ELLIPSIS,
    DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_TOP, DT_WORDBREAK, FF_DONTCARE, FW_BOLD, FW_NORMAL,
    HFONT, OUT_DEFAULT_PRECIS, PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Decoded icon (BGRA top-down, square).
struct IconPix {
    size: i32,
    bgra: Vec<u8>,
}
static ICON: OnceLock<Option<IconPix>> = OnceLock::new();

const TRAY_PNG: &[u8] = include_bytes!("../../installer/assets/tray.png");

const WIDTH: i32 = 380;
const HEIGHT: i32 = 104;
const MARGIN: i32 = 12;
const SHOW_MS: u32 = 8000;
const TIMER_CLOSE: usize = 1;

const BG: (u8, u8, u8) = (28, 30, 36);
const ACCENT: (u8, u8, u8) = (40, 180, 120);
const TITLE_RGB: (u8, u8, u8) = (245, 246, 248);
const BODY_RGB: (u8, u8, u8) = (200, 204, 212);

/// Occupied stacking positions (bit n = slot n). A toast takes the lowest free
/// slot and frees it on close, so toasts closing out of order never make a new
/// one overlap a still-visible one.
static SLOTS: Mutex<u32> = Mutex::new(0);
const MAX_SLOTS: i32 = 6;

fn claim_slot() -> i32 {
    let mut used = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    let slot = (0..MAX_SLOTS)
        .find(|i| *used & (1 << i) == 0)
        .unwrap_or(MAX_SLOTS - 1);
    *used |= 1 << slot;
    slot
}

fn release_slot(slot: i32) {
    let mut used = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    *used &= !(1 << slot);
}

fn rgb(c: (u8, u8, u8)) -> COLORREF {
    COLORREF((c.0 as u32) | ((c.1 as u32) << 8) | ((c.2 as u32) << 16))
}

fn icon() -> Option<&'static IconPix> {
    ICON.get_or_init(|| {
        let img = image::load_from_memory(TRAY_PNG).ok()?.to_rgba8();
        let size = img.width().min(img.height()) as i32;
        let mut bgra = img.into_raw();
        for px in bgra.as_chunks_mut::<4>().0 {
            px.swap(0, 2); // RGBA -> BGRA
        }
        Some(IconPix { size, bgra })
    })
    .as_ref()
}

/// Boxed strings handed to the window via its userdata pointer.
struct ToastText {
    title: Vec<u16>,
    body: Vec<u16>,
    slot: i32,
}

/// Show a toast. Returns immediately; the toast lives on its own thread.
pub fn show(title: &str, body: &str) {
    let title = title.to_string();
    let body = body.to_string();
    std::thread::spawn(move || unsafe { run(&title, &body) });
}

unsafe fn run(title: &str, body: &str) {
    let Ok(hmodule) = GetModuleHandleW(None) else {
        return;
    };
    let hinstance = windows::Win32::Foundation::HINSTANCE(hmodule.0);
    let class = w!("FleetViewToast");
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wndproc),
        hInstance: hinstance,
        lpszClassName: class,
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        ..Default::default()
    };
    // Registering the same class twice is harmless (second call just fails).
    RegisterClassExW(&wc);

    let slot = claim_slot();
    let (x, y) = position(slot);

    let text = Box::into_raw(Box::new(ToastText {
        title: wide(title),
        body: wide(body),
        slot,
    }));

    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        class,
        w!("FleetView"),
        WS_POPUP,
        x,
        y,
        WIDTH,
        HEIGHT,
        None,
        None,
        hinstance,
        Some(text as *const _),
    );
    let Ok(hwnd) = hwnd else {
        // No window, so no WM_DESTROY: reclaim the text and the slot here.
        drop(Box::from_raw(text));
        release_slot(slot);
        return;
    };

    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    SetTimer(hwnd, TIMER_CLOSE, SHOW_MS, None);

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

/// Bottom-right of the primary work area, stacking upward by slot.
fn position(slot: i32) -> (i32, i32) {
    let mut wa = RECT::default();
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut wa as *mut _ as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
    };
    let (right, bottom) = if ok {
        (wa.right, wa.bottom)
    } else {
        (1920, 1040)
    };
    let x = right - WIDTH - MARGIN;
    let y = bottom - (HEIGHT + MARGIN) * (slot + 1);
    (x, y.max(MARGIN))
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                let cs = lparam.0 as *const CREATESTRUCTW;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*cs).lpCreateParams as isize);
                LRESULT(0)
            }
            WM_PAINT => {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const ToastText;
                if ptr.is_null() {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
                paint(hwnd, &*ptr);
                LRESULT(0)
            }
            WM_LBUTTONUP | WM_RBUTTONUP => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_TIMER => {
                let _ = KillTimer(hwnd, TIMER_CLOSE);
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ToastText;
                if !ptr.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    let text = Box::from_raw(ptr);
                    release_slot(text.slot);
                }
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe fn paint(hwnd: HWND, text: &ToastText) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let full = RECT {
        left: 0,
        top: 0,
        right: WIDTH,
        bottom: HEIGHT,
    };

    // Background + accent stripe on the left edge.
    let bg = CreateSolidBrush(rgb(BG));
    FillRect(hdc, &full, bg);
    let _ = DeleteObject(bg);
    let accent = CreateSolidBrush(rgb(ACCENT));
    FillRect(
        hdc,
        &RECT {
            left: 0,
            top: 0,
            right: 5,
            bottom: HEIGHT,
        },
        accent,
    );
    let _ = DeleteObject(accent);

    // Icon on the left.
    let icon_box = 48;
    let icon_x = 20;
    let icon_y = (HEIGHT - icon_box) / 2;
    if let Some(ic) = icon() {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: ic.size,
                biHeight: -ic.size, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        StretchDIBits(
            hdc,
            icon_x,
            icon_y,
            icon_box,
            icon_box,
            0,
            0,
            ic.size,
            ic.size,
            Some(ic.bgra.as_ptr() as *const _),
            &bmi,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
    }

    let text_left = icon_x + icon_box + 16;
    SetBkMode(hdc, TRANSPARENT);

    // Title (bold).
    let title_font = make_font(20, FW_BOLD.0 as i32);
    let old = SelectObject(hdc, title_font);
    SetTextColor(hdc, rgb(TITLE_RGB));
    let mut title_rect = RECT {
        left: text_left,
        top: 16,
        right: WIDTH - 14,
        bottom: 44,
    };
    let mut title = text.title.clone();
    DrawTextW(
        hdc,
        title.as_mut_slice(),
        &mut title_rect,
        DT_LEFT | DT_TOP | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
    );
    SelectObject(hdc, old);
    let _ = DeleteObject(title_font);

    // Body (wraps to two lines).
    let body_font = make_font(16, FW_NORMAL.0 as i32);
    let old = SelectObject(hdc, body_font);
    SetTextColor(hdc, rgb(BODY_RGB));
    let mut body_rect = RECT {
        left: text_left,
        top: 46,
        right: WIDTH - 14,
        bottom: HEIGHT - 12,
    };
    let mut body = text.body.clone();
    DrawTextW(
        hdc,
        body.as_mut_slice(),
        &mut body_rect,
        DT_LEFT | DT_TOP | DT_WORDBREAK | DT_END_ELLIPSIS | DT_NOPREFIX,
    );
    SelectObject(hdc, old);
    let _ = DeleteObject(body_font);

    let _ = EndPaint(hwnd, &ps);
}

unsafe fn make_font(height: i32, weight: i32) -> HFONT {
    CreateFontW(
        -height,
        0,
        0,
        0,
        weight,
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

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
