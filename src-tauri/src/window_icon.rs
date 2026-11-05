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

/// Makes Windows show the current icon after an update.
///
/// Windows keeps a program's icon in a cache, so after an update that changes the icon
/// the taskbar and Start menu go on showing the old one. The first start of each
/// version tells Windows the icon may have changed. It runs on its own thread, so
/// start-up never waits for it.
pub fn refresh_after_update(version: &str) {
    let Some(marker) = marker_file() else { return };
    let seen = std::fs::read_to_string(&marker).ok();
    if !needs_refresh(seen.as_deref(), version) {
        return;
    }
    let version = version.to_string();
    std::thread::spawn(move || {
        notify_icon_changed();
        if let Some(dir) = marker.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&marker, &version);
        println!("[Icon] Asked Windows to refresh the icon for version {version}");
    });
}

/// Where the last version that refreshed the icon is kept.
fn marker_file() -> Option<std::path::PathBuf> {
    #[cfg(debug_assertions)]
    if let Ok(dir) = std::env::var("VAULTX_DEV_DATA_DIR") {
        return Some(std::path::PathBuf::from(dir).join("icon-version"));
    }
    dirs::config_dir().map(|dir| dir.join("VaultX").join("icon-version"))
}

fn needs_refresh(seen: Option<&str>, version: &str) -> bool {
    seen.map(str::trim) != Some(version)
}

/// Tells the shell that this program and its Start menu shortcut changed, then that
/// icons in general may have: the same notice an installer sends.
fn notify_icon_changed() {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNE_UPDATEITEM, SHCNF_FLUSH, SHCNF_IDLIST, SHCNF_PATHW,
    };

    let mut changed = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        changed.push(exe);
    }
    if let Ok(program_data) = std::env::var("ProgramData") {
        changed.push(
            std::path::PathBuf::from(program_data).join(r"Microsoft\Windows\Start Menu\Programs\VaultX.lnk"),
        );
    }
    unsafe {
        for path in changed {
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
            SHChangeNotify(SHCNE_UPDATEITEM as i32, SHCNF_PATHW, wide.as_ptr().cast(), std::ptr::null());
        }
        SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST | SHCNF_FLUSH, std::ptr::null(), std::ptr::null());
    }
}

#[cfg(test)]
mod tests {
    use super::needs_refresh;

    #[test]
    fn the_icon_is_refreshed_once_per_version() {
        assert!(needs_refresh(None, "1.0.15"));
        assert!(needs_refresh(Some("1.0.14"), "1.0.15"));
        assert!(!needs_refresh(Some("1.0.15"), "1.0.15"));
        assert!(!needs_refresh(Some("1.0.15\n"), "1.0.15"));
    }
}
