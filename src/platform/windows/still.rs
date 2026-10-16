//! The translated screen: a picture shown in place of the screen until it is
//! put away.
//!
//! The last step of Ctrl+Shift+F. The screen was photographed, read, and every
//! paragraph on the photograph written over with its translation; what is left
//! is to put the photograph up where the screen was, at full brightness and
//! pixel for pixel, so that what the user sees is their own screen with the
//! words changed. Any key or any click takes it away.
//!
//! **Why this is not in [`super::overlay`].** That window is the rectangle
//! gesture: a translucent sheet, a crosshair, a frame that follows the drag,
//! and five statics holding where the drag began. This one is opaque, has no
//! pointer of its own, takes no input but the press that dismisses it, and
//! paints exactly once. Sharing a window procedure between the two would mean
//! a mode flag on every branch of it, and the two modes have no branch in
//! common.
//!
//! **Why GDI and not the toolkit.** The same reason the sheet gives: this must
//! appear at once, cover everything, and always be dismissable. A
//! `SetDIBitsToDevice` of bytes that are already in the layout a DIB wants is
//! one call, and needs no renderer, no context and no frame loop.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, DIB_RGB_COLORS, EndPaint, PAINTSTRUCT,
    SetDIBitsToDevice, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, IDC_ARROW,
    KillTimer, LoadCursorW, MSG, RegisterClassW, SW_SHOW, SetForegroundWindow, SetTimer,
    ShowWindow, TranslateMessage, WM_CHAR, WM_KEYDOWN, WM_LBUTTONDOWN, WM_MBUTTONDOWN,
    WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_TIMER, WM_XBUTTONDOWN, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use super::ocr::Frame;

/// The picture the window paints, left here for the window procedure.
///
/// A static rather than a pointer through `CREATESTRUCT`: only one translated
/// screen is ever up — the engine will not start a second while one is showing
/// — and the procedure runs on the same thread that put it there.
static PICTURE: Mutex<Option<Frame>> = Mutex::new(None);

/// Set by the key or the click that puts the picture away.
static FINISHED: AtomicBool = AtomicBool::new(false);

/// How long the picture stays up when nothing puts it away, in milliseconds.
///
/// Long enough to read a page of translated text and come back to it, short
/// enough that a screen nobody can see under is not left there: if the key was
/// pressed by accident and the user walked away, their screen comes back on
/// its own.
const PATIENCE: u32 = 600_000;

const TIMER: usize = 1;

const CLASS: PCWSTR = w!("bubbleTranslateTranslatedScreen");

/// Puts `picture` up at `at`, in physical pixels on the virtual desktop, and
/// blocks until a key or a click puts it away.
///
/// Blocking is the point: the screen underneath is covered, nothing else
/// should be happening, and the caller is the engine's worker rather than the
/// thread the interface is drawn on.
pub fn show(picture: Frame, at: (i32, i32)) {
    let (width, height) = (picture.width as i32, picture.height as i32);
    if width <= 0 || height <= 0 {
        return;
    }
    match PICTURE.lock() {
        Ok(mut slot) => *slot = Some(picture),
        Err(_) => {
            crate::trace!("page      the picture lock was poisoned");
            return;
        }
    }
    FINISHED.store(false, Ordering::SeqCst);

    // SAFETY: the window is destroyed before this returns on every path out,
    // and the class is registered once — a second registration fails
    // harmlessly and the class already there is the same one.
    let window = unsafe {
        let instance = match GetModuleHandleW(None) {
            Ok(instance) => instance,
            Err(error) => {
                crate::trace!("page      no module handle ({error})");
                return;
            }
        };
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            // An ordinary pointer. The picture is not a gesture and nothing on
            // it can be clicked; a crosshair or a wait cursor would each say
            // something untrue about it.
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassW(&class);

        match CreateWindowExW(
            // Topmost so it covers what it is a picture of, and a tool window
            // so it never joins the taskbar or Alt+Tab: it is a still, not
            // something anyone should be able to put behind another window and
            // forget.
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS,
            PCWSTR::null(),
            WS_POPUP,
            at.0,
            at.1,
            width,
            height,
            None,
            None,
            Some(instance.into()),
            None,
        ) {
            Ok(window) => window,
            Err(error) => {
                crate::trace!("page      the window would not be created ({error})");
                return;
            }
        }
    };

    // SAFETY: the window exists until it is destroyed below.
    unsafe {
        let _ = ShowWindow(window, SW_SHOW);
        let _ = SetForegroundWindow(window);
        let _ = UpdateWindow(window);
        SetTimer(Some(window), TIMER, PATIENCE, None);
    }

    pump();

    // SAFETY: the window and the timer both exist here, and neither is touched
    // again afterwards.
    unsafe {
        let _ = KillTimer(Some(window), TIMER);
        let _ = DestroyWindow(window);
    }
    if let Ok(mut slot) = PICTURE.lock() {
        *slot = None;
    }
}

/// The picture's own message loop, until something puts it away.
fn pump() {
    let mut message = MSG::default();
    loop {
        if FINISHED.load(Ordering::SeqCst) {
            break;
        }
        // SAFETY: an ordinary modal loop. Zero is WM_QUIT and negative is an
        // error; both end it, as in the canonical loop.
        unsafe {
            if GetMessageW(&mut message, None, 0, 0).0 <= 0 {
                break;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        // Any key, any button, the wheel, or the end of the patience above:
        // every one of them is somebody wanting their screen back, which is
        // the only thing this window listens for.
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN
        | WM_XBUTTONDOWN | WM_MOUSEWHEEL | WM_TIMER => {
            FINISHED.store(true, Ordering::SeqCst);
            LRESULT(0)
        }
        // Swallowed so dismissing it never beeps: a key pressed on a window
        // that does not handle it is a system sound.
        WM_CHAR => LRESULT(0),
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            // SAFETY: paired with EndPaint on the way out.
            let dc = unsafe { BeginPaint(window, &mut paint) };
            if let Ok(slot) = PICTURE.lock()
                && let Some(picture) = slot.as_ref()
            {
                // A negative height says the bytes are top-down: the first row
                // of the buffer is the top row of the picture, which is how
                // BitBlt handed them over and how the painter wrote into them.
                // Bottom-up is the DIB default and would show the screen
                // upside down.
                let info = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: picture.width as i32,
                        biHeight: -(picture.height as i32),
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                // SAFETY: the buffer is width * height * 4 bytes in exactly
                // the layout the header above describes, and it is borrowed
                // for the length of the call only.
                unsafe {
                    SetDIBitsToDevice(
                        dc,
                        0,
                        0,
                        picture.width,
                        picture.height,
                        0,
                        0,
                        0,
                        picture.height,
                        picture.bgrx.as_ptr() as *const std::ffi::c_void,
                        &info,
                        DIB_RGB_COLORS,
                    );
                }
            }
            // SAFETY: matches the BeginPaint above.
            let _ = unsafe { EndPaint(window, &paint) };
            LRESULT(0)
        }
        // WM_DESTROY is deliberately not handled, for the reason spelled out
        // in [`super::overlay`]: posting WM_QUIT from it leaves one in this
        // thread's queue for the next gesture to find, and the next press then
        // does nothing. The loop ends on FINISHED, and the caller destroys the
        // window.
        //
        // SAFETY: the default handler, for everything not spoken for.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}
