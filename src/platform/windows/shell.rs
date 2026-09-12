//! Windows shell integration: the notification area icon.
//!
//! The app spends its life with no window on screen, so something has to
//! survive the main window being closed and bring it back. That is this icon,
//! and it is also the only Quit the app has.
//!
//! It lives on a thread of its own with a message-only window, in the same
//! spirit as the Linux tray: a notification icon is a conversation with
//! Explorer, conducted in window messages, and giving it its own pump keeps it
//! out of the way of the render loop — and keeps the render loop out of the
//! way of a menu the user is holding open.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use windows::Win32::Foundation::{
    GENERIC_READ, GENERIC_WRITE, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows::Win32::Globalization::CP_UTF8;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
    STD_OUTPUT_HANDLE, SetConsoleOutputCP, SetStdHandle,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, SHELLEXECUTEINFOW,
    Shell_NotifyIconW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, EnumWindows,
    GetCursorPos, GetMessageW, GetWindowThreadProcessId, HICON, HWND_MESSAGE, IDI_APPLICATION,
    IsIconic, IsWindowVisible, LoadIconW, MF_SEPARATOR, MF_STRING, MSG, RegisterClassW,
    RegisterWindowMessageW, SW_RESTORE, SW_SHOWNORMAL, SetForegroundWindow, ShowWindow,
    TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_APP, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW,
};
use windows::core::{PCWSTR, w};

/// The message Explorer sends this window when the icon is clicked.
const WM_TRAY: u32 = WM_APP + 1;

const ID_OPEN: u32 = 1;
const ID_QUIT: u32 = 2;

/// The icon's id within this window. There is only ever one.
const ICON_ID: u32 = 1;

/// Whether the icon is actually on the taskbar.
static INDICATOR: AtomicBool = AtomicBool::new(false);

/// Whether the attempt to put it there has finished, either way. The UI waits
/// for this before deciding whether it is safe to start with no window.
static SETTLED: AtomicBool = AtomicBool::new(false);

/// Set when a menu item is clicked, cleared once the UI has acted on it.
/// Plain flags rather than a channel: the window procedure runs on the tray
/// thread, which owns no viewport and has nobody to send to.
static OPEN_REQUESTED: AtomicBool = AtomicBool::new(false);
static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// The tray window, so the icon can be put back when Explorer restarts.
static TRAY_WINDOW: AtomicIsize = AtomicIsize::new(0);

/// The bubble's own window, so [`activate`] can tell it apart from the main
/// window when it goes looking for something to raise.
static BUBBLE_WINDOW: AtomicIsize = AtomicIsize::new(0);

/// Needed to wake the render loop from a menu click — the app idles at a
/// one-hour repaint interval while the bubble is hidden, so without this the
/// window would not appear until something else happened.
static WAKE: OnceLock<eframe::egui::Context> = OnceLock::new();

/// Whether something outside the app's own windows can bring it back.
pub fn has_indicator() -> bool {
    INDICATOR.load(Ordering::SeqCst)
}

/// Whether the icon's fate is decided. Adding one is a request to Explorer,
/// which can be busy or absent, so the answer arrives a moment after start.
pub fn indicator_settled() -> bool {
    SETTLED.load(Ordering::SeqCst)
}

pub fn take_open_request() -> bool {
    OPEN_REQUESTED.swap(false, Ordering::SeqCst)
}

pub fn take_quit_request() -> bool {
    QUIT_REQUESTED.swap(false, Ordering::SeqCst)
}

/// Asks the UI to show and raise the main window.
///
/// Public because a second copy of the binary asks for exactly this: launching
/// bubbleTranslate while it is already running should bring the window
/// forward, not start a rival translator. See [`crate::ipc`].
pub fn request_open() {
    request(&OPEN_REQUESTED, "open");
}

fn request(flag: &AtomicBool, what: &str) {
    crate::trace!("{what} requested");
    flag.store(true, Ordering::SeqCst);
    if let Some(ctx) = WAKE.get() {
        ctx.request_repaint();
    }
}

/// Borrows the console of the terminal that started this process, if there was
/// one.
///
/// The binary is built for the windows subsystem so that launching it from an
/// icon does not flash up a console — but `--check`, `--license` and
/// `--translate` exist to be read, and a subsystem choice made for the
/// interface must not make the diagnostics invisible. Attaching to the parent
/// console gives them somewhere to print when they were started from a
/// terminal, and quietly fails when they were not.
///
/// The standard handles then have to be pointed at it by hand, because a
/// process built this way starts with none — but only the ones that are
/// genuinely missing. A caller who redirected this command's output, or piped
/// it into something, handed us a perfectly good handle already, and replacing
/// that with the console would send the answer to the screen while the pipe
/// they are reading stays empty.
pub fn attach_console() {
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            return;
        }
        for (name, slot) in [
            (w!("CONIN$"), STD_INPUT_HANDLE),
            (w!("CONOUT$"), STD_OUTPUT_HANDLE),
            (w!("CONOUT$"), STD_ERROR_HANDLE),
        ] {
            if let Ok(existing) = GetStdHandle(slot)
                && !existing.is_invalid()
            {
                continue;
            }
            let handle = CreateFileW(
                name,
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            );
            if let Ok(handle) = handle {
                let _ = SetStdHandle(slot, handle);
            }
        }
        // The console's own code page, which is still a regional one on most
        // installs, decides how these bytes are read. A translator that prints
        // its answer as mojibake has failed at the one thing it does, so the
        // console is told what it is being sent.
        let _ = SetConsoleOutputCP(CP_UTF8);
    }
}

/// Nothing to do: a Windows process has no activation policy to switch
/// between, and the bubble keeps its hands off the foreground by being a
/// no-activate tool window rather than by the process being a background one.
pub fn set_foreground(_visible: bool) {}

/// Nothing to do, for the same reason.
pub fn run_in_background() {}

/// Nothing to hook. Launching the app a second time starts a second process
/// rather than notifying the first, which is handled where it happens — see
/// [`crate::ipc`] — and not by a callback from the window server.
pub fn hook_reopen() -> bool {
    true
}

/// Remembers which window is the bubble.
///
/// Called once the toolkit has created it. [`activate`] needs to know, because
/// "our other window" is how it finds the main window, and "our other window"
/// would otherwise sometimes be the bubble.
pub fn remember_bubble(window: isize) {
    BUBBLE_WINDOW.store(window, Ordering::SeqCst);
}

/// Pulls the main window to the front, restoring it if it was minimized.
///
/// The toolkit's own focus request is sent right after this and does most of
/// the work; what it will not do is un-minimize, and a window the user
/// minimized is exactly the window they are asking for when they click the
/// tray icon.
pub fn activate() {
    let Some(window) = main_window() else {
        return;
    };
    unsafe {
        if IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_RESTORE);
        }
        let _ = SetForegroundWindow(window);
    }
}

/// This process's main window: the one visible top-level window that is not
/// the bubble.
fn main_window() -> Option<HWND> {
    let mut found = HWND::default();
    // SAFETY: the callback below only writes through this pointer, and
    // EnumWindows does not outlive the call.
    let _ = unsafe {
        EnumWindows(
            Some(find_main_window),
            LPARAM(&mut found as *mut HWND as isize),
        )
    };
    (!found.is_invalid()).then_some(found)
}

unsafe extern "system" fn find_main_window(window: HWND, out: LPARAM) -> windows::core::BOOL {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    if pid != std::process::id()
        || !unsafe { IsWindowVisible(window) }.as_bool()
        || window.0 as isize == BUBBLE_WINDOW.load(Ordering::SeqCst)
    {
        return true.into();
    }
    unsafe { *(out.0 as *mut HWND) = window };
    // Stop: the first visible window of ours that is not the bubble is the one.
    false.into()
}

/// Opens a URL in whatever the user's browser is.
///
/// The one outward link the app has. Buying happens in a browser and nowhere
/// else: 3-D Secure needs one, and an app that never asks for a card number is
/// an app with nothing to leak.
pub fn open_url(url: &str) {
    let url: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        lpVerb: w!("open"),
        lpFile: PCWSTR(url.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // ShellExecuteEx rather than ShellExecute: the simpler call reports
    // failure as a small integer cast to a handle, which is a footgun, and
    // this one also lets the browser take the foreground from us — without
    // that, a browser that is already running comes up behind this window.
    if let Err(err) = unsafe { ShellExecuteExW(&mut info) } {
        eprintln!("bubbleTranslate: could not open the browser: {err}");
    }
}

/// Installs the notification area icon.
pub fn install(ctx: eframe::egui::Context) {
    let _ = WAKE.set(ctx);
    if let Err(err) = std::thread::Builder::new()
        .name("tray".into())
        .spawn(run_tray)
    {
        eprintln!("bubbleTranslate: could not start the tray icon: {err}");
        SETTLED.store(true, Ordering::SeqCst);
    }
}

fn run_tray() {
    let Some(window) = create_window() else {
        SETTLED.store(true, Ordering::SeqCst);
        return;
    };
    TRAY_WINDOW.store(window.0 as isize, Ordering::SeqCst);

    let added = add_icon(window);
    INDICATOR.store(added, Ordering::SeqCst);
    SETTLED.store(true, Ordering::SeqCst);
    crate::trace!("tray icon added={added}");

    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, Some(HWND::default()), 0, 0) }.as_bool() {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::TranslateMessage(&message);
            windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&message);
        }
    }

    // Explorer keeps showing an icon whose owner died until something makes it
    // look, so it is taken down deliberately on the way out.
    remove_icon(window);
}

fn create_window() -> Option<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("bubbleTranslateTray"),
            ..Default::default()
        };
        // A zero here means the class is already registered, which is fine and
        // cannot happen twice in one process anyway.
        RegisterClassW(&class);

        // Message-only: no pixels, never shown, not enumerated as a window of
        // this application — it exists purely to receive the icon's clicks.
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("bubbleTranslateTray"),
            w!("bubbleTranslate"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        )
        .ok()
    }
}

fn icon_data(window: HWND) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: window,
        uID: ICON_ID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: app_icon(),
        ..Default::default()
    };
    for (slot, ch) in data.szTip.iter_mut().zip("bubbleTranslate".encode_utf16()) {
        *slot = ch;
    }
    data
}

fn add_icon(window: HWND) -> bool {
    unsafe { Shell_NotifyIconW(NIM_ADD, &icon_data(window)) }.as_bool()
}

fn remove_icon(window: HWND) {
    let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &icon_data(window)) };
}

/// This executable's own icon, falling back to the stock application one.
///
/// Resource id 1 is where the icon linked into the binary lands, so the tray
/// shows the same picture Explorer shows for the file.
fn app_icon() -> HICON {
    unsafe {
        // A resource is named either by a string or by a number squeezed into
        // the pointer itself, which is what `MAKEINTRESOURCE` does in C and
        // what this is: the address is the id, not somewhere to read from.
        let by_number = PCWSTR(std::ptr::without_provenance(1));
        if let Ok(instance) = GetModuleHandleW(None)
            && let Ok(icon) = LoadIconW(Some(instance.into()), by_number)
        {
            return icon;
        }
        LoadIconW(None, IDI_APPLICATION).unwrap_or_default()
    }
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Explorer can be restarted, or can crash, and every icon on the taskbar
    // goes with it. This broadcast is how it says the taskbar is new; putting
    // the icon back is the difference between a hiccup and an app with no way
    // back to its window for the rest of the session.
    if message == taskbar_created() {
        let added = add_icon(window);
        INDICATOR.store(added, Ordering::SeqCst);
        crate::trace!("taskbar restarted; icon re-added={added}");
        return LRESULT(0);
    }

    if message == WM_TRAY {
        match lparam.0 as u32 {
            WM_LBUTTONUP => request_open(),
            WM_RBUTTONUP => show_menu(window),
            _ => {}
        }
        return LRESULT(0);
    }

    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// The message Explorer broadcasts when the taskbar is (re)created. Registered
/// once; the id is the same for every process that asks.
fn taskbar_created() -> u32 {
    static ID: OnceLock<u32> = OnceLock::new();
    *ID.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) })
}

fn show_menu(window: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            ID_OPEN as usize,
            w!("Open Bubble Translate"),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            ID_QUIT as usize,
            w!("Quit bubbleTranslate"),
        );

        let mut at = POINT::default();
        let _ = GetCursorPos(&mut at);
        // Required, and long documented as such: a menu belonging to a window
        // that is not in the foreground never receives the click that dismisses
        // it, and stays on screen over everything else.
        let _ = SetForegroundWindow(window);

        let chosen = TrackPopupMenu(
            menu,
            TPM_LEFTALIGN | TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            at.x,
            at.y,
            None,
            window,
            None,
        );
        let _ = DestroyMenu(menu);

        match chosen.0 as u32 {
            ID_OPEN => request_open(),
            ID_QUIT => request(&QUIT_REQUESTED, "quit"),
            _ => {}
        }
    }
}
