use crate::session::SessionState;
use arboard::Clipboard;
use std::sync::Mutex;
use tauri::State;

#[tauri::command]
pub fn write_text(text: String, state: State<'_, Mutex<SessionState>>) -> Result<(), String> {
    let s = state.lock().unwrap();
    if !s.is_authenticated() { return Err("Not authenticated".to_string()); }
    drop(s);
    set_clipboard(&text)
}

#[tauri::command]
pub fn copy_password(id: String, state: State<'_, Mutex<SessionState>>) -> Result<(), String> {
    let s = state.lock().unwrap();
    if !s.is_authenticated() { return Err("Not authenticated".to_string()); }

    let password = s.vault.as_ref().unwrap().accounts.iter()
        .find(|a| a.id == id)
        .map(|a| a.password.clone())
        .ok_or("Account not found")?;
    drop(s);

    set_clipboard(&password)?;

    // Auto-clear the clipboard after 30 seconds if it still contains the password
    let pw_clone = password.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(30));
        if let Ok(mut cb) = Clipboard::new() {
            if let Ok(current) = cb.get_text() {
                if current == pw_clone {
                    let _ = cb.set_text("");
                }
            }
        }
    });

    Ok(())
}

fn set_clipboard(text: &str) -> Result<(), String> {
    let mut cb = Clipboard::new().map_err(|e| format!("Clipboard error: {e}"))?;
    cb.set_text(text).map_err(|e| format!("Clipboard write error: {e}"))?;
    Ok(())
}
