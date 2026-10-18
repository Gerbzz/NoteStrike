//! The macOS application menu ("NoteStrike" next to the Apple menu).
//!
//! winit's default menu wires Quit to `terminate:`, which exits the process
//! at once: unsaved tabs would only survive through the recovery journal.
//! This menu sends `performClose:` to the key window instead, so Cmd+Q goes
//! through the same close request as the window's close button and asks
//! about unsaved tabs first. NoteStrike has one window, so closing it quits.
//! Edit commands (Cmd+C, Cmd+V, ...) are handled by the editor itself.

use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem};
use objc2_foundation::NSString;

/// Install the menu. Must run on the main thread after the app launched
/// (eframe's app-creation callback); does nothing elsewhere.
pub fn install() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let menubar = NSMenu::new(mtm);

    let app_menu = NSMenu::new(mtm);
    let services = NSMenu::new(mtm);
    let services_item = item(mtm, "Services", None, "", None);
    services_item.setSubmenu(Some(&services));

    app_menu.addItem(&item(
        mtm,
        "About NoteStrike",
        Some(sel!(orderFrontStandardAboutPanel:)),
        "",
        None,
    ));
    app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    app_menu.addItem(&services_item);
    app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    app_menu.addItem(&item(mtm, "Hide NoteStrike", Some(sel!(hide:)), "h", None));
    app_menu.addItem(&item(
        mtm,
        "Hide Others",
        Some(sel!(hideOtherApplications:)),
        "h",
        Some(NSEventModifierFlags::Command | NSEventModifierFlags::Option),
    ));
    app_menu.addItem(&item(
        mtm,
        "Show All",
        Some(sel!(unhideAllApplications:)),
        "",
        None,
    ));
    app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    app_menu.addItem(&item(
        mtm,
        "Quit NoteStrike",
        Some(sel!(performClose:)),
        "q",
        None,
    ));
    let app_item = NSMenuItem::new(mtm);
    app_item.setSubmenu(Some(&app_menu));
    menubar.addItem(&app_item);

    let window_menu = NSMenu::new(mtm);
    window_menu.setTitle(&NSString::from_str("Window"));
    window_menu.addItem(&item(
        mtm,
        "Minimize",
        Some(sel!(performMiniaturize:)),
        "m",
        None,
    ));
    window_menu.addItem(&item(mtm, "Zoom", Some(sel!(performZoom:)), "", None));
    let window_item = item(mtm, "Window", None, "", None);
    window_item.setSubmenu(Some(&window_menu));
    menubar.addItem(&window_item);

    app.setServicesMenu(Some(&services));
    app.setWindowsMenu(Some(&window_menu));
    app.setMainMenu(Some(&menubar));
}

fn item(
    mtm: MainThreadMarker,
    title: &str,
    action: Option<Sel>,
    key: &str,
    modifiers: Option<NSEventModifierFlags>,
) -> Retained<NSMenuItem> {
    // SAFETY: every selector passed here is a standard AppKit action
    // message, dispatched through the responder chain.
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str(title),
            action,
            &NSString::from_str(key),
        )
    };
    if let Some(modifiers) = modifiers {
        item.setKeyEquivalentModifierMask(modifiers);
    }
    item
}
