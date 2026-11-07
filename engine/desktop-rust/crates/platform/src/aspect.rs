//! ウィンドウの縦横比の固定。
//!
//! winit 0.30 には縦横比を固定する API が無いので、OS の API を直接呼ぶ。
//!
//! - macOS: `NSWindow.contentAspectRatio`。角や辺をつまんで変えても比が保たれる。
//! - Windows: ウィンドウをサブクラス化し、`WM_SIZING` (大きさを変えている途中) で矩形を比に合わせて直す。
//! - Linux (X11 / Wayland): できない (winit がウィンドウマネージャへのヒントを出さず、Wayland には仕組みが無い)。
//!   中身はレターボックスで比を保つ (player 側) ので、表示は崩れない。
//!
//! 全画面では OS が大きさを決めるので、どれも効かない (そこもレターボックスで保つ)。

use raw_window_handle::HasWindowHandle;

/// `window` の中身 (クライアント領域) の縦横比を `width : height` に固定する。できなければ何もしない。
pub fn lock(window: &winit::window::Window, width: u32, height: u32) {
    let Ok(handle) = window.window_handle() else { return };
    if width == 0 || height == 0 {
        return;
    }
    match handle.as_raw() {
        #[cfg(target_os = "macos")]
        raw_window_handle::RawWindowHandle::AppKit(handle) => macos::lock(handle.ns_view.as_ptr(), width, height),
        #[cfg(windows)]
        raw_window_handle::RawWindowHandle::Win32(handle) => windows::lock(handle.hwnd.get(), width, height),
        _ => {}
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;

    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_foundation::NSSize;

    pub fn lock(ns_view: *mut c_void, width: u32, height: u32) {
        let view = ns_view.cast::<AnyObject>();
        // SAFETY: `ns_view` は winit が作った生きている NSView で、ここはメインスレッド (イベントループの中)。
        unsafe {
            let window: *mut AnyObject = msg_send![view, window];
            if window.is_null() {
                return;
            }
            let ratio = NSSize::new(f64::from(width), f64::from(height));
            let _: () = msg_send![window, setContentAspectRatio: ratio];
        }
    }
}

#[cfg(windows)]
mod windows {
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetWindowRect, WM_SIZING, WMSZ_BOTTOM, WMSZ_BOTTOMLEFT, WMSZ_BOTTOMRIGHT, WMSZ_LEFT,
        WMSZ_RIGHT, WMSZ_TOP, WMSZ_TOPLEFT, WMSZ_TOPRIGHT,
    };

    /// サブクラスの番号 (同じウィンドウに付けた他のサブクラスと区別する)。
    const SUBCLASS_ID: usize = 0xB1_4A;

    pub fn lock(hwnd: isize, width: u32, height: u32) {
        // 比は f32 のビットで渡す (64 bit でも 32 bit でも usize に収まる)。
        let ratio = (width as f32 / height as f32).to_bits() as usize;
        // SAFETY: `hwnd` は winit が作った生きているウィンドウで、ここはそれを作ったスレッド。
        unsafe {
            SetWindowSubclass(hwnd as HWND, Some(subclass), SUBCLASS_ID, ratio);
        }
    }

    unsafe extern "system" fn subclass(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        ratio: usize,
    ) -> LRESULT {
        if message == WM_SIZING && lparam != 0 {
            let ratio = f32::from_bits(ratio as u32);
            // SAFETY: WM_SIZING の lparam は、書き換えてよい RECT (枠を含むウィンドウの矩形) を指す。
            unsafe { fit(hwnd, wparam as u32, &mut *(lparam as *mut RECT), ratio) };
            return 1;
        }
        // SAFETY: 他のメッセージは元のウィンドウ手続き (winit) に回す。
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }

    /// つまんでいる辺・角 (`edge`) に応じて、`rect` のクライアント領域が `ratio` になるよう直す。
    unsafe fn fit(hwnd: HWND, edge: u32, rect: &mut RECT, ratio: f32) {
        // 枠 (タイトルバーなど) の大きさ = ウィンドウの矩形 - クライアント領域。大きさを変えても変わらない。
        let (mut window, mut client) = (RECT { left: 0, top: 0, right: 0, bottom: 0 }, RECT { left: 0, top: 0, right: 0, bottom: 0 });
        // SAFETY: 生きているウィンドウに対する読み取りだけ。
        unsafe {
            GetWindowRect(hwnd, &mut window);
            GetClientRect(hwnd, &mut client);
        }
        let frame_w = (window.right - window.left) - (client.right - client.left);
        let frame_h = (window.bottom - window.top) - (client.bottom - client.top);

        let client_w = (rect.right - rect.left - frame_w).max(1);
        let client_h = (rect.bottom - rect.top - frame_h).max(1);
        match edge {
            // 左右の辺: 幅に合わせて高さを決め、下へ伸ばす。
            WMSZ_LEFT | WMSZ_RIGHT => {
                rect.bottom = rect.top + (client_w as f32 / ratio).round() as i32 + frame_h;
            }
            // 上下の辺: 高さに合わせて幅を決め、右へ伸ばす。
            WMSZ_TOP | WMSZ_BOTTOM => {
                rect.right = rect.left + (client_h as f32 * ratio).round() as i32 + frame_w;
            }
            // 角: 幅に合わせて高さを決め、つまんでいる側の辺を動かす。
            WMSZ_TOPLEFT | WMSZ_TOPRIGHT => {
                rect.top = rect.bottom - (client_w as f32 / ratio).round() as i32 - frame_h;
            }
            WMSZ_BOTTOMLEFT | WMSZ_BOTTOMRIGHT => {
                rect.bottom = rect.top + (client_w as f32 / ratio).round() as i32 + frame_h;
            }
            _ => {}
        }
    }
}
