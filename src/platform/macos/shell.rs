//! macOS shell integration: the menu bar icon and the activation policy.
//!
//! The app runs as an accessory (no Dock icon, so the bubble never steals
//! focus), which means closing the main window would otherwise leave no way
//! back to it. This status item is that way back, and the only Quit affordance.

use std::ffi::CString;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::encode::Encode;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, Sel};
use objc2::{MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSApplicationActivationPolicy, NSMenu,
    NSMenuItem, NSRunningApplication, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{MainThreadMarker, NSObject, NSString};

/// Set when the menu item is clicked, cleared once the UI has acted on it.
/// A plain flag rather than a channel: the Objective-C callback runs on the
/// main thread between egui frames, so there is no one to receive on.
static OPEN_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Needed to wake the render loop from the menu callback — the app idles at a
/// one-hour repaint interval when the bubble is hidden, so without this the
/// window would not appear until something else happened.
static WAKE: OnceLock<eframe::egui::Context> = OnceLock::new();

/// Whether something outside the app's own windows can bring it back.
///
/// True here: the menu bar item survives the main window being closed, which
/// is what makes closing it mean "get out of the way" rather than "quit".
pub fn has_indicator() -> bool {
    true
}

/// True exactly once per click on "Open Bubble Translate".
pub fn take_open_request() -> bool {
    OPEN_REQUESTED.swap(false, Ordering::SeqCst)
}

/// Always settled: the status item is installed synchronously and AppKit does
/// not refuse it, so there is never a moment where its presence is in doubt.
pub fn indicator_settled() -> bool {
    true
}

/// Never true here: the menu bar's Quit item is wired straight to
/// `NSApplication`'s own `terminate:`, so nothing has to be relayed through
/// the UI loop. The Linux tray has no such thing and does relay it, which is
/// why this exists on both.
pub fn take_quit_request() -> bool {
    false
}

/// Asks the UI to show and raise the main window.
///
/// Activation deliberately does not happen here: an accessory app cannot be
/// activated at all, so the policy has to change first. That ordering lives in
/// the UI loop, which calls [`show_in_dock`] and then [`activate`].
fn request_open() {
    crate::trace!("open requested");
    OPEN_REQUESTED.store(true, Ordering::SeqCst);
    if let Some(ctx) = WAKE.get() {
        ctx.request_repaint();
    }
}

/// Switches between a normal app and a background one.
///
/// This is what makes the main window reachable. Under the cooperative
/// activation model of recent macOS versions an accessory app simply cannot be
/// brought to the front — `activateIgnoringOtherApps:` is deprecated to a
/// no-op — so the app becomes a regular, Dock-visible app for as long as the
/// main window is open, and drops back to accessory when it closes. Accessory
/// is the important state: it is what stops the bubble from stealing focus
/// from whatever is being read.
pub fn set_foreground(visible: bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let wanted = if visible {
        NSApplicationActivationPolicy::Regular
    } else {
        NSApplicationActivationPolicy::Accessory
    };
    if app.activationPolicy() != wanted {
        crate::trace!("activation policy -> {:?}", wanted);
        app.setActivationPolicy(wanted);
    }
}

/// Pulls the process to the front. Only effective once [`show_in_dock`] has
/// made this a regular app.
///
/// `ActivateAllWindows` is required, not decorative: activating with no
/// options leaves the app exactly where it was. It only affects windows that
/// are actually in the window list, so the ordered-out bubble stays hidden.
pub fn activate() {
    let activated = NSRunningApplication::currentApplication()
        .activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
    crate::trace!("activate -> {activated}");
}

/// Handles the Finder/Dock/`open` "reopen" event.
///
/// Launching an already-running app does not start a second process: macOS
/// sends this to the existing one instead. Without it, opening bubbleTranslate again
/// while it runs does nothing at all — no window, no activation.
extern "C-unwind" fn handle_reopen(
    _this: *mut AnyObject,
    _cmd: Sel,
    _sender: *mut AnyObject,
    _has_visible_windows: Bool,
) -> Bool {
    request_open();
    Bool::YES
}

/// Adds [`handle_reopen`] to whichever `NSApplicationDelegate` the windowing
/// backend installed.
///
/// Adding one method to the existing delegate leaves every other delegate
/// callback untouched, which substituting our own delegate would not.
/// `class_addMethod` refuses to overwrite an existing implementation, so this
/// cannot clobber the backend's own handler if it ever grows one.
///
/// Returns false while the delegate does not exist yet, so the caller can
/// retry on a later frame.
pub fn hook_reopen() -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    let Some(delegate) = app.delegate() else {
        return false;
    };

    // Built from objc2's own encodings rather than hardcoded: `BOOL` is a
    // `signed char` on some targets and a C `bool` on others, and the wrong
    // letter here would corrupt the call.
    let types = format!("{}@:@{}", Bool::ENCODING, Bool::ENCODING);
    let Ok(types) = CString::new(types) else {
        return false;
    };

    unsafe {
        let class: *const AnyClass = msg_send![&*delegate, class];
        let name: *const std::ffi::c_char = objc2::ffi::class_getName(class as *mut _);
        crate::trace!(
            "hooking reopen on delegate class {:?}",
            std::ffi::CStr::from_ptr(name)
        );
        objc2::ffi::class_addMethod(
            class as *mut _,
            sel!(applicationShouldHandleReopen:hasVisibleWindows:),
            std::mem::transmute::<
                extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, Bool) -> Bool,
                unsafe extern "C-unwind" fn(),
            >(handle_reopen),
            types.as_ptr(),
        );
    }
    crate::trace!("reopen hook installed");
    true
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this type does not
    // implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "BubbleTranslateStatusTarget"]
    struct StatusTarget;

    impl StatusTarget {
        #[unsafe(method(openMainWindow:))]
        fn open_main_window(&self, _sender: Option<&AnyObject>) {
            request_open();
        }
    }
);

impl StatusTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

/// Installs the menu bar item.
pub fn install(ctx: eframe::egui::Context) {
    // Dropping the status item removes the icon, so it is deliberately leaked:
    // it has to live exactly as long as the process.
    if let Some(item) = install_status_item(ctx) {
        std::mem::forget(item);
    }
}

/// Runs as an accessory app: no Dock icon, no menu bar of our own, and showing
/// a window does not activate us over whatever the user is reading.
pub fn run_in_background() {
    set_foreground(false);
}

fn install_status_item(ctx: eframe::egui::Context) -> Option<Retained<NSStatusItem>> {
    let mtm = MainThreadMarker::new()?;
    let _ = WAKE.set(ctx);

    let bar = NSStatusBar::systemStatusBar();
    let item = bar.statusItemWithLength(NSVariableStatusItemLength);

    let button = item.button(mtm)?;
    button.setTitle(&NSString::from_str("🌐"));

    let target = StatusTarget::new(mtm);
    let menu = NSMenu::new(mtm);

    let open = NSMenuItem::new(mtm);
    open.setTitle(&NSString::from_str("Open Bubble Translate"));
    unsafe {
        open.setAction(Some(sel!(openMainWindow:)));
        open.setTarget(Some(&target));
    }
    menu.addItem(&open);

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    let quit = NSMenuItem::new(mtm);
    quit.setTitle(&NSString::from_str("Quit bubbleTranslate"));
    // terminate: is handled by NSApplication itself, so this needs no target
    // of our own.
    unsafe {
        quit.setAction(Some(sel!(terminate:)));
        quit.setTarget(Some(&NSApplication::sharedApplication(mtm)));
    }
    menu.addItem(&quit);

    item.setMenu(Some(&menu));

    // The menu holds the items, but nothing holds the target, and an
    // Objective-C action target is a weak reference. Leaking it is the
    // simplest correct lifetime for something that lives as long as the app.
    std::mem::forget(target);

    Some(item)
}
