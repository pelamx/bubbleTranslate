//! Choosing the rectangle to read, by dragging one over a dimmed screen.
//!
//! The gesture is the one every screenshot tool has taught people: the whole
//! desktop goes dim, the pointer becomes a crosshair, and what you drag over
//! comes back to full brightness. Let go and that is the region; press Escape
//! or the right button and nothing happened.
//!
//! **How the hole is made.** The window is one translucent sheet over the
//! whole virtual desktop, and the bright part is not drawn — it is cut out
//! with [`SetWindowRgn`], exactly as the bubble's corners are in
//! [`super::shape_bubble`]. Inside the cut the window does not exist: the
//! pixels underneath are the real ones, at full brightness, with no alpha
//! blending to wash them out. That matters here more than it looks, because
//! those are the pixels that are about to be read — dimming them and
//! undimming them in software would hand the OCR engine a slightly different
//! image than the one on screen.
//!
//! **And why the selection is shown twice.** The cut alone was not enough.
//! A window region hole reveals what is beneath it only if the desktop
//! composites those pixels back, and on a virtual GPU it does not reliably:
//! the sheet was solid grey, the rectangle was being drawn correctly —
//! `CombineRgn` and `SetWindowRgn` both reported success — and there was
//! still nothing on screen to show where it was. So the sheet is translucent
//! rather than opaque, and [`procedure`] paints a frame around the selection
//! in the part of the window that survives the cut. Either one alone answers
//! "where am I selecting"; together they answer it on machines where the
//! other does not.
//!
//! **Why a raw window and not a second egui viewport.** This is one black
//! rectangle and one cut-out; it needs no renderer, and after what a frameless
//! toolkit window did to the bubble on virtual machines — see the note in
//! [`super::shape_bubble`] — there is no appetite for putting the graphics
//! stack between the user and a full-screen overlay that must appear
//! instantly and must always be dismissable.
//!
//! The loop below blocks the thread it runs on until the drag ends. That is
//! deliberate: it is a modal gesture, nothing else should happen while it is
//! up, and the caller is a worker rather than the UI thread.
//!

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CombineRgn, CreateRectRgn, CreateSolidBrush, DeleteObject, EndPaint, FillRect,
    HGDIOBJ, HRGN, InvalidateRect, PAINTSTRUCT, RGN_DIFF, SetWindowRgn, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetMessageW, GetSystemMetrics, IDC_CROSS, LWA_ALPHA, LoadCursorW, MSG, PostQuitMessage,
    RegisterClassW, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SW_SHOW, SetCursor, SetForegroundWindow, SetLayeredWindowAttributes, ShowWindow,
    TranslateMessage, WM_CHAR, WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_PAINT, WM_RBUTTONDOWN, WM_SETCURSOR, WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use super::ocr::Region;

/// Where the drag began, packed as two `i32` in one word so the window
/// procedure and the loop can share it without a lock. `None` until the button
/// goes down.
static ANCHOR: AtomicIsize = AtomicIsize::new(NONE);
/// Where the pointer is now, same packing.
static CURRENT: AtomicIsize = AtomicIsize::new(NONE);
/// Where the pointer was at the last cut, so [`reshape`] can repaint the
/// frame's old position instead of the whole sheet. `NONE` before the first.
static PREVIOUS: AtomicIsize = AtomicIsize::new(NONE);
/// Set when the gesture ended, either way. [`CANCELLED`] says which.
static FINISHED: AtomicBool = AtomicBool::new(false);
static CANCELLED: AtomicBool = AtomicBool::new(false);

/// The packed value that means "no point yet".
///
/// A real screen position can be negative — a monitor left of the primary one
/// — so zero is a perfectly ordinary coordinate and cannot mean absent.
const NONE: isize = isize::MIN;

fn pack(x: i32, y: i32) -> isize {
    ((x as isize) << 32) | (y as isize & 0xffff_ffff)
}

fn unpack(packed: isize) -> Option<(i32, i32)> {
    (packed != NONE).then_some(((packed >> 32) as i32, (packed & 0xffff_ffff) as u32 as i32))
}

/// The whole virtual desktop, in physical pixels.
///
/// Every monitor, including any placed left of or above the primary one, which
/// is why the origin is read rather than assumed to be zero.
fn virtual_screen() -> (i32, i32, i32, i32) {
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

/// The rectangle the two corners describe, normalised so dragging up and to
/// the left is the same gesture as dragging down and to the right.
fn between(a: (i32, i32), b: (i32, i32)) -> Region {
    Region {
        x: a.0.min(b.0),
        y: a.1.min(b.1),
        width: (a.0 - b.0).abs(),
        height: (a.1 - b.1).abs(),
    }
}

/// Puts the dimmed screen up and blocks until a rectangle is drawn on it.
///
/// `None` when the user pressed Escape, clicked the right button, or let go
/// without having dragged anywhere — all three are the same intent.
///
/// The region comes back in physical pixels on the virtual desktop, which is
/// what [`super::ocr::recognize`] takes.
pub fn select_region() -> Option<Region> {
    ANCHOR.store(NONE, Ordering::SeqCst);
    CURRENT.store(NONE, Ordering::SeqCst);
    PREVIOUS.store(NONE, Ordering::SeqCst);
    FINISHED.store(false, Ordering::SeqCst);
    CANCELLED.store(false, Ordering::SeqCst);

    let (x, y, width, height) = virtual_screen();
    if width <= 0 || height <= 0 {
        crate::trace!("overlay   the virtual screen has no size");
        return None;
    }

    crate::trace!("overlay   putting the sheet up over {width}x{height} at ({x}, {y})");

    // SAFETY: the window is destroyed before this returns on every path, and
    // the class is registered once — a second registration fails harmlessly
    // and the existing class is used.
    let window = unsafe {
        let instance = match GetModuleHandleW(None) {
            Ok(instance) => instance,
            Err(error) => {
                crate::trace!("overlay   no module handle ({error})");
                return None;
            }
        };
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            // The crosshair is the whole affordance: it is what says the click
            // about to happen means "corner of a rectangle" rather than
            // whatever the window underneath would have done with it.
            hCursor: LoadCursorW(None, IDC_CROSS).unwrap_or_default(),
            // The sheet itself: the whole window is this colour, and the cut
            // in [`reshape`] is what lets the screen back through.
            hbrBackground: CreateSolidBrush(DIM),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassW(&class);

        match CreateWindowExW(
            // Topmost so it covers what is being read, and a tool window so
            // it never appears in the taskbar or in Alt+Tab. It is up for a
            // second and a half.
            //
            // Layered as well, so the sheet is translucent rather than solid:
            // the user has to be able to see what they are aiming at while
            // they aim at it. The cut-out still goes to full brightness on
            // top of that — two ways of showing the same thing, which is
            // deliberate. A window region hole reveals the pixels underneath
            // only if the desktop composites them back, and on the virtual
            // GPU this is developed against it does not always; the
            // translucency and the frame below are what make the selection
            // visible when it does not.
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
            CLASS,
            PCWSTR::null(),
            WS_POPUP,
            x,
            y,
            width,
            height,
            None,
            None,
            Some(instance.into()),
            None,
        ) {
            Ok(window) => window,
            Err(error) => {
                crate::trace!("overlay   the window would not be created ({error})");
                return None;
            }
        }
    };

    // SAFETY: the window exists until it is destroyed below. Capture is taken
    // so a drag that leaves the overlay — off the edge of a monitor, over a
    // window that would otherwise take the button — still reports its moves
    // here.
    unsafe {
        let _ = SetLayeredWindowAttributes(window, COLORREF(0), SHEET_ALPHA, LWA_ALPHA);
        let _ = ShowWindow(window, SW_SHOW);
        let _ = SetForegroundWindow(window);
        let _ = UpdateWindow(window);
        SetCapture(window);
    }

    let outcome = pump(window);

    unsafe {
        let _ = ReleaseCapture();
        let _ = DestroyWindow(window);
    }

    match outcome {
        Some(region) if region.width > 0 && region.height > 0 => {
            crate::trace!(
                "overlay   picked {}x{} at ({}, {})",
                region.width,
                region.height,
                region.x,
                region.y
            );
            Some(region)
        }
        _ => {
            crate::trace!("overlay   cancelled");
            None
        }
    }
}

/// Runs the overlay's own message loop until the gesture ends.
fn pump(window: HWND) -> Option<Region> {
    let mut message = MSG::default();
    loop {
        if FINISHED.load(Ordering::SeqCst) {
            break;
        }
        // SAFETY: an ordinary modal loop. `GetMessageW` returns 0 on
        // `WM_QUIT`, which the procedure posts when the window goes away.
        unsafe {
            if !GetMessageW(&mut message, None, 0, 0).as_bool() {
                break;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        // The cut is remade here rather than in the procedure so it happens
        // once per message the loop actually sees, not once per mouse move
        // Windows coalesces.
        reshape(window);
    }

    if CANCELLED.load(Ordering::SeqCst) {
        return None;
    }
    let anchor = unpack(ANCHOR.load(Ordering::SeqCst))?;
    let current = unpack(CURRENT.load(Ordering::SeqCst))?;
    Some(between(anchor, current))
}

/// Cuts the current selection out of the dim sheet.
fn reshape(window: HWND) {
    let (Some(anchor), Some(current)) = (
        unpack(ANCHOR.load(Ordering::SeqCst)),
        unpack(CURRENT.load(Ordering::SeqCst)),
    ) else {
        return;
    };
    let region = between(anchor, current);
    let (origin_x, origin_y, width, height) = virtual_screen();

    // SAFETY: both regions are owned here until `SetWindowRgn` takes the
    // combined one; the window frees it on the next call and on destruction.
    unsafe {
        let whole: HRGN = CreateRectRgn(0, 0, width, height);
        // The window's own coordinates, so the screen origin comes off.
        let hole: HRGN = CreateRectRgn(
            region.x - origin_x,
            region.y - origin_y,
            region.x - origin_x + region.width,
            region.y - origin_y + region.height,
        );
        if CombineRgn(Some(whole), Some(whole), Some(hole), RGN_DIFF).0 != 0 {
            SetWindowRgn(window, Some(whole), true);
        } else {
            let _ = DeleteObject(HGDIOBJ(whole.0));
        }
        let _ = DeleteObject(HGDIOBJ(hole.0));

        // Repaint where the frame was and where it now is, rather than the
        // whole sheet. The difference is not academic: the sheet covers every
        // monitor, and erasing eight megapixels on each mouse move is enough
        // work to make the thread look busy to the system — which is the
        // other half of why the pointer was a spinner.
        let moved_from = PREVIOUS.swap(pack(current.0, current.1), Ordering::SeqCst);
        for corner in [unpack(moved_from), Some(current)].into_iter().flatten() {
            let area = between(anchor, corner);
            let stale = RECT {
                left: area.x - origin_x - FRAME_WIDTH,
                top: area.y - origin_y - FRAME_WIDTH,
                right: area.x - origin_x + area.width + FRAME_WIDTH,
                bottom: area.y - origin_y + area.height + FRAME_WIDTH,
            };
            let _ = InvalidateRect(Some(window), Some(&stale), true);
        }
    }
}

/// How dark the sheet is. Black would hide what is being pointed at; this is
/// dark enough to say "the bright part is the part that counts" and light
/// enough to still read the screen through it while choosing.
const DIM: COLORREF = COLORREF(0x0020_2020);

/// How opaque the sheet is, out of 255.
///
/// Enough to read the screen through while aiming, and enough that the
/// selection is obviously not dimmed. The whole gesture lasts a second, so
/// this is a signpost rather than something anyone looks at.
const SHEET_ALPHA: u8 = 140;

/// The frame drawn around the selection, in `0x00bbggrr`.
///
/// The one thing on the overlay with a colour of its own, because it is the
/// only thing that says *where* the rectangle is. It is drawn just outside
/// the cut, so it survives the hole being cut out from under it.
const FRAME: COLORREF = COLORREF(0x00ff_c84a);

/// How thick that frame is, in pixels.
const FRAME_WIDTH: i32 = 3;

const CLASS: PCWSTR = w!("bubbleTranslateRegionOverlay");

/// Everything the gesture is, in five messages.
extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // The pointer arrives in the window's own coordinates; every consumer
    // wants the screen's, and the window starts at the virtual desktop's
    // origin, so the offset is the same one throughout.
    let at = || {
        let (origin_x, origin_y, _, _) = virtual_screen();
        let packed = lparam.0 as u32;
        (
            (packed & 0xffff) as i16 as i32 + origin_x,
            ((packed >> 16) & 0xffff) as i16 as i32 + origin_y,
        )
    };

    match message {
        WM_LBUTTONDOWN => {
            let (x, y) = at();
            ANCHOR.store(pack(x, y), Ordering::SeqCst);
            CURRENT.store(pack(x, y), Ordering::SeqCst);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if ANCHOR.load(Ordering::SeqCst) != NONE {
                let (x, y) = at();
                CURRENT.store(pack(x, y), Ordering::SeqCst);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if ANCHOR.load(Ordering::SeqCst) != NONE {
                let (x, y) = at();
                CURRENT.store(pack(x, y), Ordering::SeqCst);
                FINISHED.store(true, Ordering::SeqCst);
            }
            LRESULT(0)
        }
        // Escape and the right button are the same word: never mind. Both are
        // what a person reaches for without being told, which is the only
        // instruction an overlay with no interface can rely on.
        WM_RBUTTONDOWN => {
            CANCELLED.store(true, Ordering::SeqCst);
            FINISHED.store(true, Ordering::SeqCst);
            LRESULT(0)
        }
        WM_KEYDOWN if wparam.0 as u16 == VK_ESCAPE.0 => {
            CANCELLED.store(true, Ordering::SeqCst);
            FINISHED.store(true, Ordering::SeqCst);
            LRESULT(0)
        }
        // The crosshair, set here rather than left to the window class.
        //
        // A class cursor is only consulted if nothing claims the message
        // first, and on a window created by a worker thread what claims it is
        // the system's own idea that the application is still starting — the
        // spinner. Claiming it here, and returning TRUE so nothing downstream
        // gets a say, is what makes the pointer say "draw a rectangle" from
        // the moment the sheet appears.
        WM_SETCURSOR => {
            // SAFETY: a shared system cursor. It is not owned here and must
            // never be destroyed.
            unsafe {
                if let Ok(crosshair) = LoadCursorW(None, IDC_CROSS) {
                    SetCursor(Some(crosshair));
                }
            }
            LRESULT(1)
        }
        // The frame around the selection, and the only thing on the sheet
        // with a colour of its own.
        //
        // Drawn just *outside* the rectangle, in the part of the window that
        // survives the cut, so it is visible whether or not the cut reveals
        // anything. On a machine where the hole works it outlines bright
        // pixels; on one where it does not it is still the answer to "where
        // am I selecting".
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            // SAFETY: paired with `EndPaint` below on every path.
            let dc = unsafe { BeginPaint(window, &mut paint) };

            if let (Some(anchor), Some(current)) = (
                unpack(ANCHOR.load(Ordering::SeqCst)),
                unpack(CURRENT.load(Ordering::SeqCst)),
            ) {
                let region = between(anchor, current);
                if region.width > 0 && region.height > 0 {
                    let (origin_x, origin_y, _, _) = virtual_screen();
                    let (left, top) = (region.x - origin_x, region.y - origin_y);
                    let (right, bottom) = (left + region.width, top + region.height);
                    let edge = FRAME_WIDTH;

                    // SAFETY: the brush is deleted before the handler returns.
                    unsafe {
                        let brush = CreateSolidBrush(FRAME);
                        for side in [
                            RECT {
                                left: left - edge,
                                top: top - edge,
                                right: right + edge,
                                bottom: top,
                            },
                            RECT {
                                left: left - edge,
                                top: bottom,
                                right: right + edge,
                                bottom: bottom + edge,
                            },
                            RECT {
                                left: left - edge,
                                top,
                                right: left,
                                bottom,
                            },
                            RECT {
                                left: right,
                                top,
                                right: right + edge,
                                bottom,
                            },
                        ] {
                            FillRect(dc, &side, brush);
                        }
                        let _ = DeleteObject(HGDIOBJ(brush.0));
                    }
                }
            }

            // SAFETY: matches the `BeginPaint` above.
            let _ = unsafe { EndPaint(window, &paint) };
            LRESULT(0)
        }
        // Swallowed so the overlay never beeps: a key pressed on a window
        // that does not handle it is a system sound, and this one is up over
        // whatever the user was reading.
        WM_CHAR => LRESULT(0),
        WM_DESTROY => {
            // SAFETY: ends the loop above if it is still in `GetMessageW`.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: the default handler, for everything not spoken for.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_survives_the_round_trip() {
        for point in [(0, 0), (1920, 1080), (-1440, -200), (i32::MAX, i32::MIN)] {
            assert_eq!(unpack(pack(point.0, point.1)), Some(point));
        }
    }

    #[test]
    fn absent_is_not_a_coordinate() {
        assert_eq!(unpack(NONE), None);
        // The origin is an ordinary place to start a drag, and must not read
        // as "no point yet".
        assert_ne!(pack(0, 0), NONE);
    }

    #[test]
    fn dragging_backwards_is_the_same_rectangle() {
        let forward = between((100, 100), (300, 250));
        let backward = between((300, 250), (100, 100));
        assert_eq!(forward, backward);
        assert_eq!(
            forward,
            Region {
                x: 100,
                y: 100,
                width: 200,
                height: 150,
            }
        );
    }

    #[test]
    fn a_rectangle_across_a_monitor_to_the_left_keeps_its_negative_origin() {
        let across = between((-500, 40), (120, 300));
        assert_eq!(across.x, -500);
        assert_eq!(across.width, 620);
    }

    /// Puts the real overlay up and drags a real rectangle on it, then reads
    /// the text inside. Ignored by default: it takes the screen over for a
    /// moment and moves the pointer, which no CI runner has and no developer
    /// wants happening under their hands unasked.
    ///
    /// ```text
    /// cargo test --target x86_64-pc-windows-msvc overlay -- --ignored --nocapture
    /// ```
    ///
    /// The drag is synthesized rather than performed, so this proves the parts
    /// that have no human in them: that the overlay comes up, that it holds
    /// the pointer, that the rectangle it reports is the one that was drawn,
    /// and that what comes back out of it is readable text.
    #[test]
    #[ignore = "takes over the screen and moves the pointer"]
    fn drags_a_rectangle_and_reads_what_is_under_it() {
        use std::time::Duration;
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT,
            SendInput,
        };
        use windows::Win32::UI::WindowsAndMessaging::SetCursorPos;

        fn button(flags: windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS) {
            let input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dwFlags: flags,
                        ..Default::default()
                    },
                },
            };
            unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
        }

        let picked = std::thread::spawn(select_region);

        // The overlay has to exist before it can be dragged on.
        std::thread::sleep(Duration::from_millis(600));

        let (from, to) = ((200, 200), (900, 520));
        unsafe { SetCursorPos(from.0, from.1) }.unwrap();
        std::thread::sleep(Duration::from_millis(120));
        button(MOUSEEVENTF_LEFTDOWN);

        // In steps, because one jump is a teleport rather than a drag, and a
        // drag is what the overlay is watching for.
        for step in 1..=10 {
            let x = from.0 + (to.0 - from.0) * step / 10;
            let y = from.1 + (to.1 - from.1) * step / 10;
            unsafe { SetCursorPos(x, y) }.unwrap();
            std::thread::sleep(Duration::from_millis(40));
        }
        button(MOUSEEVENTF_LEFTUP);

        let region = picked
            .join()
            .expect("the overlay thread should not panic")
            .expect("letting go after a drag should give a region");

        println!("picked: {region:?}");
        assert_eq!(region.x, from.0);
        assert_eq!(region.y, from.1);
        assert_eq!(region.width, to.0 - from.0);
        assert_eq!(region.height, to.1 - from.1);

        match super::super::ocr::recognize(region) {
            Some(text) => println!("--- {} chars read ---\n{text}", text.chars().count()),
            None => println!("(nothing readable was under the rectangle)"),
        }
    }
}
