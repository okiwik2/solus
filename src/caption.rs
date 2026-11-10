//! Native caption buttons (minimise / maximise / close).
//!
//! They are painted by the top-level window rather than the HTML UI so that Windows 11 Snap Layouts
//! work: the maximise button must answer WM_NCHITTEST with HTMAXBUTTON from the real window.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Btn {
    Min,
    Max,
    Close,
}

#[derive(Clone, Debug)]
pub struct CaptionState {
    pub hover: Option<Btn>,
    pub pressed: Option<Btn>,
    /// caption height in CSS pixels (the tab strip height)
    pub height_css: f64,
    pub bg: COLORREF,
    pub fg: COLORREF,
    pub hover_bg: COLORREF,
    pub inactive_fg: COLORREF,
    pub active: bool,
    pub visible: bool,
}

impl Default for CaptionState {
    fn default() -> Self {
        CaptionState {
            hover: None,
            pressed: None,
            height_css: 40.0,
            bg: rgb(0xe8, 0xeb, 0xf0),
            fg: rgb(0x20, 0x20, 0x20),
            hover_bg: rgb(0xd5, 0xd9, 0xe0),
            inactive_fg: rgb(0x80, 0x80, 0x80),
            active: true,
            visible: true,
        }
    }
}

pub const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | ((g as u32) << 8) | ((b as u32) << 16))
}

pub fn parse_color(s: &str) -> Option<COLORREF> {
    let s = s.trim().trim_start_matches('#');
    if s.len() == 6 {
        let v = u32::from_str_radix(s, 16).ok()?;
        return Some(rgb((v >> 16) as u8, (v >> 8) as u8, v as u8));
    }
    None
}

thread_local! {
    static STATES: RefCell<HashMap<isize, CaptionState>> = RefCell::new(HashMap::new());
}

pub fn with_state<R>(hwnd: HWND, f: impl FnOnce(&mut CaptionState) -> R) -> R {
    STATES.with(|s| {
        let mut m = s.borrow_mut();
        let st = m.entry(hwnd.0 as isize).or_default();
        f(st)
    })
}

pub fn remove(hwnd: HWND) {
    STATES.with(|s| {
        s.borrow_mut().remove(&(hwnd.0 as isize));
    });
}

pub fn scale(hwnd: HWND) -> f64 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        1.0
    } else {
        dpi as f64 / 96.0
    }
}

/// Physical-pixel rect of a button in client coordinates.
pub fn button_rect(hwnd: HWND, b: Btn) -> RECT {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    let s = scale(hwnd);
    let h = with_state(hwnd, |st| st.height_css);
    let bw = (46.0 * s).round() as i32;
    let bh = (h * s).round() as i32;
    let idx = match b {
        Btn::Min => 2,
        Btn::Max => 1,
        Btn::Close => 0,
    };
    let right = rc.right - idx * bw;
    RECT { left: right - bw, top: 0, right, bottom: bh }
}

pub fn buttons_rect(hwnd: HWND) -> RECT {
    let a = button_rect(hwnd, Btn::Min);
    let c = button_rect(hwnd, Btn::Close);
    RECT { left: a.left, top: 0, right: c.right, bottom: c.bottom }
}

pub fn hit(hwnd: HWND, pt: POINT) -> Option<Btn> {
    if !with_state(hwnd, |s| s.visible) {
        return None;
    }
    for b in [Btn::Min, Btn::Max, Btn::Close] {
        let r = button_rect(hwnd, b);
        if pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom {
            return Some(b);
        }
    }
    None
}

pub fn invalidate(hwnd: HWND) {
    let r = buttons_rect(hwnd);
    unsafe {
        let _ = InvalidateRect(Some(hwnd), Some(&r), false);
    }
}

pub fn paint(hwnd: HWND, hdc: HDC) {
    let st = with_state(hwnd, |s| s.clone());
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
        // whole client background in the title-bar colour (only visible where the UI is clipped away)
        let br = CreateSolidBrush(st.bg);
        FillRect(hdc, &rc, br);
        let _ = DeleteObject(br.into());
        if !st.visible {
            return;
        }
        let s = scale(hwnd);
        let font = CreateFontW(
            (-10.0 * s).round() as i32,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe Fluent Icons"),
        );
        let old = SelectObject(hdc, font.into());
        SetBkMode(hdc, TRANSPARENT);
        let maximized = IsZoomed(hwnd).as_bool();
        for b in [Btn::Min, Btn::Max, Btn::Close] {
            let mut r = button_rect(hwnd, b);
            let hot = st.hover == Some(b) || st.pressed == Some(b);
            if hot {
                let c = if b == Btn::Close { rgb(0xc4, 0x2b, 0x1c) } else { st.hover_bg };
                let hb = CreateSolidBrush(c);
                FillRect(hdc, &r, hb);
                let _ = DeleteObject(hb.into());
            }
            let fg = if hot && b == Btn::Close {
                rgb(255, 255, 255)
            } else if st.active {
                st.fg
            } else {
                st.inactive_fg
            };
            SetTextColor(hdc, fg);
            let glyph: &[u16] = match b {
                Btn::Min => &[0xE921],
                Btn::Max => {
                    if maximized {
                        &[0xE923]
                    } else {
                        &[0xE922]
                    }
                }
                Btn::Close => &[0xE8BB],
            };
            let mut g = glyph.to_vec();
            DrawTextW(hdc, &mut g, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        }
        SelectObject(hdc, old);
        let _ = DeleteObject(font.into());
    }
}
