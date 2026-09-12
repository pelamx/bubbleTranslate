//! Windows: one binary, one windowing system, and a selection that has to be
//! asked for rather than watched.
//!
//! Three facts shape everything below.
//!
//! **Selections.** Windows has no primary selection and no notion of a
//! selection published to the desktop: what is highlighted belongs to the
//! application that drew it, and stays there. So there is nothing to watch.
//! Text is *asked for* at the moment a gesture ends — over UI Automation where
//! the application answers, and over a synthetic Ctrl+C where it does not.
//! See [`capture`].
//!
//! **The pointer and the gesture.** Both are freely readable. A low-level hook
//! sees every mouse button and every key on the system, and `GetCursorPos`
//! answers at any time, from any thread, with no permission attached. That
//! makes the filtering in [`monitor`] exact, unlike the Wayland side where the
//! quiet after a selection is the only evidence there is.
//!
//! **The window.** A toplevel may place itself wherever it likes, so the
//! bubble simply goes to the cursor. The one thing that has to be asked for is
//! the promise never to take focus — `WS_EX_NOACTIVATE`, set in
//! [`mark_as_notification`] — without which clicking the bubble's copy button
//! would pull the foreground away from whatever is being read.

pub mod capture;
pub mod monitor;
pub mod shell;

use std::sync::atomic::{AtomicIsize, Ordering};

use eframe::egui;

use ::windows::Win32::Foundation::{HWND, POINT, RECT};
use ::windows::Win32::Graphics::Gdi::{
    CreateRoundRectRgn, GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromPoint, SetWindowRgn,
};
use ::windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use ::windows::Win32::UI::HiDpi::GetDpiForWindow;
use ::windows::Win32::UI::Shell::{IVirtualDesktopManager, VirtualDesktopManager};
use ::windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetCursorPos, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW,
    SetWindowPos, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

/// The bubble's window, once the toolkit has made one.
static BUBBLE: AtomicIsize = AtomicIsize::new(0);

fn bubble() -> Option<HWND> {
    match BUBBLE.load(Ordering::SeqCst) {
        0 => None,
        handle => Some(HWND(handle as *mut std::ffi::c_void)),
    }
}

/// Where the pointer is, in physical pixels on the virtual desktop.
///
/// Only the hotkey path asks: a selection's trigger carries its own anchor,
/// taken at the moment the gesture ended, and this is for the case where there
/// was no gesture to take one from.
pub fn cursor_position() -> Option<(f64, f64)> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.ok()?;
    Some((point.x as f64, point.y as f64))
}

/// How many of the toolkit's points one pixel is worth.
///
/// The two differ on any display that is not at 100%, and rather than
/// predicting the factor from a DPI — there are three of those on Windows and
/// they do not always agree — it is measured: the same monitor, described in
/// both units, is the whole conversion.
///
/// The monitor is the one the pointer is on; the points come from the one the
/// bubble is on. Those are the same monitor in every case but one, a layout
/// that mixes scalings, where the bubble can land off by the difference until
/// it has been shown once on the new display.
fn points_per_pixel(at: (f64, f64), monitor_points: Option<egui::Vec2>) -> f64 {
    let Some(points) = monitor_points else {
        return 1.0;
    };
    let Some(pixels) = monitor_size(at) else {
        return 1.0;
    };
    if pixels.0 <= 0.0 || points.x <= 0.0 {
        return 1.0;
    }
    f64::from(points.x) / pixels.0
}

/// The size in physical pixels of the monitor containing a point.
fn monitor_size(at: (f64, f64)) -> Option<(f64, f64)> {
    let point = POINT {
        x: at.0 as i32,
        y: at.1 as i32,
    };
    let monitor: HMONITOR = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return None;
    }
    let RECT {
        left,
        top,
        right,
        bottom,
    } = info.rcMonitor;
    Some(((right - left) as f64, (bottom - top) as f64))
}

/// Converts a pointer position into the toolkit's points.
pub fn to_points(at: (f64, f64), monitor_points: Option<egui::Vec2>) -> (f64, f64) {
    let scale = points_per_pixel(at, monitor_points);
    (at.0 * scale, at.1 * scale)
}

/// Whether the pointer is inside `rect`, which is in points.
///
/// Asked of the system rather than of egui because the bubble never takes
/// focus, and a window that is never activated is not reliably told the
/// pointer left it — egui's own pointer state can latch on the first crossing
/// and never clear, which would pause the selection monitor for good and stop
/// the bubble ever hiding itself. The question costs one syscall here, so
/// unlike the Linux side it needs no throttling.
pub fn pointer_over(rect: egui::Rect, monitor_points: Option<egui::Vec2>) -> Option<bool> {
    let at = cursor_position()?;
    let (x, y) = to_points(at, monitor_points);
    Some(rect.contains(egui::pos2(x as f32, y as f32)))
}

/// The zoom that makes the app's text match the rest of the desktop.
///
/// `None`: Windows tells the toolkit the scaling of the display the window is
/// on, per monitor and as it changes, and the toolkit already applies it.
/// There is no second opinion here to reconcile, as there is under XWayland.
pub fn preferred_zoom(_native_pixels_per_point: f32) -> Option<f32> {
    None
}

/// Declares the bubble a window that is never activated and never listed.
///
/// `WS_EX_NOACTIVATE` is the one that matters: without it, clicking the
/// bubble's copy button takes the foreground away from the application being
/// read, which on a text field means losing the selection the user is still
/// working with. `WS_EX_TOOLWINDOW` keeps it out of Alt-Tab, where a
/// borderless 320-pixel popup has no business appearing.
///
/// Best effort by nature; what it costs when it fails is a focus steal, not a
/// broken translator.
pub fn mark_as_notification(window: isize) {
    BUBBLE.store(window, Ordering::SeqCst);
    shell::remember_bubble(window);

    let window = HWND(window as *mut std::ffi::c_void);
    unsafe {
        let style = GetWindowLongPtrW(window, GWL_EXSTYLE);
        let wanted = style | (WS_EX_NOACTIVATE.0 as isize) | (WS_EX_TOOLWINDOW.0 as isize);
        if style != wanted {
            SetWindowLongPtrW(window, GWL_EXSTYLE, wanted);
            // An extended style is only read at certain moments; this is what
            // makes Windows notice one that changed after the fact.
            let _ = SetWindowPos(
                window,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }
    crate::trace!("bubble marked no-activate");
}

/// The corner radius the bubble's card is drawn with, in points. The window
/// underneath is cut to the same curve, so the two edges are one edge.
const BUBBLE_RADIUS: i32 = 10;

/// The size the window was last cut to, so the cut is made once per size
/// rather than once per frame.
static SHAPED: AtomicIsize = AtomicIsize::new(0);

/// Cuts the bubble's window to the shape of the card drawn inside it.
///
/// Without this the card is a rounded rectangle painted on a square window:
/// the corners outside the curve are still the window, still opaque, and still
/// drawn — four little square ears where the desktop should be. Transparency
/// is the usual answer and it is not available here (see
/// [`crate::ui::TRANSPARENT_BUBBLE`]), so the window is given the curve
/// instead, and the desktop manager stops drawing what falls outside it.
///
/// Called every frame the bubble is on screen, because the bubble resizes to
/// fit whatever was translated; it costs a rectangle comparison on the frames
/// where nothing changed.
pub fn shape_bubble() {
    let Some(window) = bubble() else {
        return;
    };
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(window, &mut rect) }.is_err() {
        return;
    }
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return;
    }

    // Both sizes in one word: two edges have to match, and comparing them
    // together is what makes this a single atomic read on a quiet frame.
    let shape = ((width as isize) << 32) | (height as isize & 0xffff_ffff);
    if SHAPED.swap(shape, Ordering::Relaxed) == shape {
        return;
    }

    let dpi = unsafe { GetDpiForWindow(window) };
    let scale = if dpi > 0 { dpi as f32 / 96.0 } else { 1.0 };
    // The region is measured in whole device pixels and its right and bottom
    // edges are exclusive, hence the extra pixel on each.
    let diameter = (BUBBLE_RADIUS as f32 * scale * 2.0).round() as i32;
    let region = unsafe { CreateRoundRectRgn(0, 0, width + 1, height + 1, diameter, diameter) };
    if region.is_invalid() {
        return;
    }
    // The window takes ownership of the region here, so it must not be
    // deleted; the next call replaces it, and Windows frees the old one.
    unsafe { SetWindowRgn(window, Some(region), true) };
}

/// Keeps the bubble on the desktop the user is actually looking at.
///
/// Windows pins a window to the virtual desktop it was created on, and hiding
/// and showing it again does not change that. Since the bubble is created once
/// and shown many times, a user who switches desktops would otherwise see the
/// translator keep working, the trace keep saying it produced a bubble, and
/// nothing appear — because the bubble is sitting on the desktop the app
/// started on.
///
/// There is no public "show on all desktops" flag, so the bubble is moved
/// instead, to wherever the foreground window is. That window is by definition
/// on the desktop in front of the user.
///
/// Returns false only while the answer is "not yet", so the caller retries;
/// true once the bubble is where it belongs or once there is nothing left to
/// try.
pub fn keep_on_all_workspaces() -> bool {
    let Some(bubble) = bubble() else {
        return false;
    };
    let Some(manager) = virtual_desktops() else {
        return true;
    };
    unsafe {
        match manager.IsWindowOnCurrentVirtualDesktop(bubble) {
            Ok(on_current) if on_current.as_bool() => return true,
            Err(err) => {
                crate::trace!("virtual desktop: cannot say where the bubble is ({err})");
                return true;
            }
            _ => {}
        }

        // The foreground window is on the desktop in front of the user, which
        // is the only way to name that desktop through the public interface.
        let foreground = GetForegroundWindow();
        let Ok(desktop) = manager.GetWindowDesktopId(foreground) else {
            return true;
        };
        match manager.MoveWindowToDesktop(bubble, &desktop) {
            Ok(()) => crate::trace!("bubble moved to the current virtual desktop"),
            Err(err) => crate::trace!("virtual desktop: could not move the bubble ({err})"),
        }
    }
    true
}

/// The virtual desktop manager, built once per thread and kept.
fn virtual_desktops() -> Option<IVirtualDesktopManager> {
    thread_local! {
        static MANAGER: Option<IVirtualDesktopManager> = unsafe {
            // Apartment-threaded to match the toolkit, which has already
            // initialised this thread for drag and drop; the status is ignored
            // because "already initialised" is the expected answer and is not
            // a failure.
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            CoCreateInstance(&VirtualDesktopManager, None, CLSCTX_ALL).ok()
        };
    }
    MANAGER.with(|manager| manager.clone())
}
