/// Gives the window icons Windows can draw sharply.
///
/// Tauri sets one bitmap (the first entry of icon.ico) as both window icons, so
/// Windows stretches it to fit the taskbar. Loading from the exe's own icon
/// resource instead lets Windows pick the exact size it needs from icon.ico.

use tauri::WebviewWindow;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, LoadImageW, SendMessageW, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR,
    LR_SHARED, SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON, WM_SETICON,
};

/// The id tauri-build gives the app icon when embedding it in the exe.
const APP_ICON_RESOURCE: usize = 32512;

pub fn apply(window: &WebviewWindow) {
    let Ok(hwnd) = window.hwnd() else { return };
    let hwnd = hwnd.0 as windows_sys::Win32::Foundation::HWND;

    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        let resource = APP_ICON_RESOURCE as *const u16;

        // LR_SHARED on the big icon lets the shell go back to the resource and
        // take the exact taskbar size rather than scaling this one.
        let big = LoadImageW(
            module,
            resource,
            IMAGE_ICON,
            GetSystemMetrics(SM_CXICON),
            GetSystemMetrics(SM_CYICON),
            LR_SHARED,
        );
        if !big.is_null() {
            SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, big as isize);
        }

        let small = LoadImageW(
            module,
            resource,
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_DEFAULTCOLOR,
        );
        if !small.is_null() {
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, small as isize);
        }
    }
}
