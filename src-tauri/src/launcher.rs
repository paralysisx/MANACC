/// League of Legends launcher: signs in to the Riot Client, then starts the game.
///
/// Riot's sign-in needs a captcha token, so credentials can't be sent through the
/// client's local API. They are typed into the client's own sign-in form instead,
/// which runs its captcha as usual. The local API is only read, to know when the
/// form is ready and how the sign-in ended.

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

// Prevents console windows from flashing when spawning child processes on Windows.
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const CLIENT_START_TIMEOUT: Duration = Duration::from_secs(60);
const FORM_TIMEOUT: Duration = Duration::from_secs(20);
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(45);
/// How long to keep asking the client to launch League.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
/// One launch request stays open until League has started.
const LAUNCH_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// How long to look for the form before bringing the client to the front.
const QUIET_FORM_TIMEOUT: Duration = Duration::from_secs(2);

const LAUNCH_LEAGUE: &str = "/product-launcher/v1/products/league_of_legends/patchlines/live";
const POLL: Duration = Duration::from_millis(500);

const NEEDS_CODE: &str = "Riot wants a verification code. Enter it in the Riot Client, then press Play again.";

pub struct LaunchAccount {
    pub username: String,
    pub password: String,
    pub riot_id: String,
}

impl Drop for LaunchAccount {
    fn drop(&mut self) {
        // Zero bytes are valid UTF-8, so this can't leave the String invalid.
        unsafe { self.password.as_bytes_mut().fill(0) };
    }
}

// ─── Path discovery ───────────────────────────────────────────────────────────

fn find_riot_client_path() -> Option<PathBuf> {
    crate::riot_install::riot_client_exe()
}

/// The folder a running program was started from, by its file name.
pub fn folder_of_running(program: &str) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    return ui::process_folder(program);
    #[cfg(not(target_os = "windows"))]
    return { let _ = program; None };
}

/// The running Riot Client's local port and password. Both change every time it starts.
pub(crate) fn riot_client_local() -> Option<(u16, String)> {
    let lockfile = dirs::data_local_dir()?
        .join("Riot Games").join("Riot Client").join("Config").join("lockfile");
    parse_lockfile(&std::fs::read_to_string(lockfile).ok()?)
}

/// Opens the Riot Client, or asks the running one to launch League.
fn start_riot_client(exe: &PathBuf, launch_league: bool) -> Result<(), String> {
    let mut cmd = Command::new(exe);
    if launch_league {
        cmd.args(["--launch-product=league_of_legends", "--launch-patchline=live"]);
    }
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to start Riot Client: {e}"))
}

// ─── Riot Client local API (read-only, plus sign-out) ─────────────────────────

struct LocalApi {
    http: reqwest::blocking::Client,
}

impl LocalApi {
    fn new() -> Result<Self, String> {
        reqwest::blocking::Client::builder()
            // The client serves a self-signed certificate on localhost.
            .danger_accept_invalid_certs(true)
            // Never through a system proxy or VPN: the client is on this PC.
            .no_proxy()
            .build()
            .map(|http| LocalApi { http })
            .map_err(|e| format!("HTTP client error: {e}"))
    }

    fn request(&self, method: reqwest::Method, path: &str, json: Option<&str>) -> Option<(u16, Value)> {
        self.request_within(Duration::from_secs(5), method, path, json)
    }

    fn request_within(&self, timeout: Duration, method: reqwest::Method, path: &str, json: Option<&str>) -> Option<(u16, Value)> {
        let (port, password) = riot_client_local()?;
        let mut request = self.http
            .request(method, format!("https://127.0.0.1:{port}{path}"))
            .timeout(timeout)
            .basic_auth("riot", Some(password));
        if let Some(json) = json {
            request = request.header("Content-Type", "application/json").body(json.to_string());
        }
        let response = request.send().ok()?;
        let status = response.status().as_u16();
        let body = response.text().ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
        Some((status, body))
    }

    fn get(&self, path: &str) -> Option<(u16, Value)> {
        self.request(reqwest::Method::GET, path, None)
    }

    fn sign_in_state(&self) -> SignIn {
        sign_in_state(self.get("/rso-auth/v1/session"))
    }

    /// The sign-in form's last result: when it was produced, its type and its error.
    fn form_result(&self) -> Option<(String, String, String)> {
        let (200, body) = self.get("/rso-authenticator/v1/authentication")? else { return None };
        let text = |key: &str| match body.get(key) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        };
        Some((text("timestamp"), text("type"), text("error")))
    }

    fn signed_in_riot_id(&self) -> Option<String> {
        let (200, body) = self.get("/riot-client-auth/v1/userinfo")? else { return None };
        riot_id_from_userinfo(&body)
    }
}

/// Lockfile format: name:pid:port:password:protocol
fn parse_lockfile(contents: &str) -> Option<(u16, String)> {
    let parts: Vec<&str> = contents.trim().split(':').collect();
    if parts.len() < 5 {
        return None;
    }
    Some((parts[2].parse().ok()?, parts[3].to_string()))
}

#[derive(Debug, PartialEq)]
enum SignIn {
    /// The client isn't running, or its window isn't up yet.
    NotReady,
    NeedsCredentials,
    Authenticated,
    NeedsCode,
    Other,
}

fn sign_in_state(session: Option<(u16, Value)>) -> SignIn {
    let Some((200, body)) = session else { return SignIn::NotReady };
    match body.get("type").and_then(Value::as_str) {
        Some("needs_credentials") => SignIn::NeedsCredentials,
        Some("authenticated") => SignIn::Authenticated,
        Some("needs_multifactor_verification") => SignIn::NeedsCode,
        _ => SignIn::Other,
    }
}

/// Userinfo comes as an object, or as a JSON string inside one.
fn riot_id_from_userinfo(body: &Value) -> Option<String> {
    let parsed;
    let info = match body.get("userInfo").unwrap_or(body) {
        Value::String(text) => {
            parsed = serde_json::from_str::<Value>(text).ok()?;
            &parsed
        }
        other => other,
    };
    let acct = info.get("acct")?;
    Some(format!("{}#{}", acct.get("game_name")?.as_str()?, acct.get("tag_line")?.as_str()?))
}

pub(crate) fn same_riot_id(a: &str, b: &str) -> bool {
    let normalize = |id: &str| {
        id.split('#').map(|part| part.trim().to_lowercase()).collect::<Vec<_>>().join("#")
    };
    normalize(a) == normalize(b)
}

fn describe_sign_in_error(error: &str) -> String {
    match error {
        "auth_failure" => "Riot rejected the username or password.".to_string(),
        "rate_limited" => "Too many sign-in attempts. Wait a few minutes and try again.".to_string(),
        other => format!("The Riot Client couldn't sign in ({other})."),
    }
}

// ─── Main launch orchestration ────────────────────────────────────────────────

/// Whether the League client is open.
pub fn league_running() -> bool {
    #[cfg(target_os = "windows")]
    return ui::process_running("LeagueClient.exe");
    #[cfg(not(target_os = "windows"))]
    return false;
}

/// Returns a message to show the user. A request for a 2FA code is not an error.
pub fn launch_with_account(account: &LaunchAccount) -> Result<String, String> {
    let riot_exe = find_riot_client_path()
        .ok_or("Riot Client not found. Make sure League of Legends is installed.")?;
    sign_in_and_launch(&riot_exe, account)
}

#[cfg(not(target_os = "windows"))]
fn sign_in_and_launch(_: &PathBuf, _: &LaunchAccount) -> Result<String, String> {
    Err("Signing in to the Riot Client is only supported on Windows.".to_string())
}

#[cfg(target_os = "windows")]
fn sign_in_and_launch(riot_exe: &PathBuf, account: &LaunchAccount) -> Result<String, String> {
    if ui::process_running("LeagueClient.exe") {
        return Err("League of Legends is already open. Close it first to switch accounts.".to_string());
    }
    let api = LocalApi::new()?;

    // 1. Open the Riot Client and wait until it says what it needs.
    if api.sign_in_state() == SignIn::NotReady {
        start_riot_client(riot_exe, false)?;
    }
    let mut signed_out = false;
    let deadline = Instant::now() + CLIENT_START_TIMEOUT;
    loop {
        match api.sign_in_state() {
            SignIn::NeedsCredentials => break,
            SignIn::NeedsCode => return Ok(NEEDS_CODE.to_string()),
            SignIn::Authenticated => {
                if api.signed_in_riot_id().is_some_and(|id| same_riot_id(&id, &account.riot_id)) {
                    return Ok(launch_league(&api, riot_exe, "Already signed in."));
                }
                if !signed_out {
                    println!("[Launcher] Signing out of another account");
                    api.request(reqwest::Method::DELETE, "/rso-auth/v1/session", None);
                    signed_out = true;
                }
            }
            SignIn::NotReady | SignIn::Other => {}
        }
        if Instant::now() > deadline {
            return Err(if signed_out {
                "Couldn't sign out of the other account. Sign out in the Riot Client, then press Play again."
            } else {
                "The Riot Client didn't get ready in time. Try again."
            }.to_string());
        }
        std::thread::sleep(POLL);
    }

    // 2. Put the credentials into the sign-in form.
    let before = api.form_result().map(|(timestamp, ..)| timestamp);
    {
        let _com = ui::Com::new()?;
        let previous = ui::foreground_window();
        // The form can only be read once its window has been shown, so the client
        // may have to come to the front for a moment.
        let form = match ui::wait_for_form(&ui::RIOT_CLIENT, QUIET_FORM_TIMEOUT) {
            Some(form) => form,
            None => {
                ui::bring_to_front(&ui::RIOT_CLIENT)?;
                ui::wait_for_form(&ui::RIOT_CLIENT, FORM_TIMEOUT)
                    .ok_or("Couldn't find the Riot Client's sign-in form. Make sure its window is open, then try again.")?
            }
        };
        // First choice: set the fields and press the button directly, which needs no
        // keyboard or focus. If the form doesn't take that, type instead.
        if let Err(reason) = form.sign_in_quietly(&account.username, &account.password) {
            println!("[Launcher] Typing instead of filling directly: {reason}");
            form.fill_username(&account.username)?;
            form.fill_password(&account.password)?;
            form.submit()?;
        }
        ui::give_back_focus(previous);
    }

    // 3. See how it ended.
    let deadline = Instant::now() + SIGN_IN_TIMEOUT;
    loop {
        std::thread::sleep(POLL);
        match api.sign_in_state() {
            SignIn::Authenticated => return Ok(launch_league(&api, riot_exe, "Signed in.")),
            SignIn::NeedsCode => return Ok(NEEDS_CODE.to_string()),
            _ => {}
        }
        // Only a result newer than the one from before typing counts.
        if let Some((timestamp, kind, error)) = api.form_result() {
            if Some(&timestamp) != before.as_ref() {
                if kind == "multifactor" {
                    return Ok(NEEDS_CODE.to_string());
                }
                if !error.is_empty() {
                    return Err(describe_sign_in_error(&error));
                }
            }
        }
        if Instant::now() > deadline {
            return Err("The Riot Client didn't finish signing in. Check its window: it may be asking for a captcha or another step.".to_string());
        }
    }
}

/// Presses Play for League through the client. Returns the message to show.
///
/// The request only answers once League has started, which takes several seconds.
/// Right after signing in the client refuses with 464 for a moment while it loads
/// the player, so that is retried. 423 means League is already launching or open.
#[cfg(target_os = "windows")]
fn launch_league(api: &LocalApi, riot_exe: &PathBuf, signed_in: &str) -> String {
    let launching = format!("{signed_in} Launching League of Legends.");
    let deadline = Instant::now() + LAUNCH_TIMEOUT;
    while Instant::now() < deadline {
        if ui::process_running("LeagueClient.exe") {
            return launching;
        }
        let body = Some(r#"{"customLaunchArgs":[]}"#);
        match api.request_within(LAUNCH_REQUEST_TIMEOUT, reqwest::Method::POST, LAUNCH_LEAGUE, body) {
            Some((status, _)) if launch_accepted(status) => return launching,
            Some((status, body)) => {
                let code = body.get("errorCode").and_then(Value::as_str).unwrap_or("");
                println!("[Launcher] Launch not accepted yet: HTTP {status} {code}");
            }
            None => println!("[Launcher] Launch request got no answer"),
        }
        std::thread::sleep(POLL);
    }
    if ui::process_running("LeagueClient.exe") {
        return launching;
    }
    // Asking through the command line at least opens League's page.
    let _ = start_riot_client(riot_exe, true);
    format!("{signed_in} If League doesn't start, press Play in the Riot Client.")
}

fn launch_accepted(status: u16) -> bool {
    (200..300).contains(&status) || status == 423
}

// ─── Filling in the sign-in form ──────────────────────────────────────────────

#[cfg(target_os = "windows")]
mod ui {
    use std::time::{Duration, Instant};
    use std::path::PathBuf;
    use windows::core::{BSTR, PWSTR};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::Foundation::{CloseHandle, HWND, RPC_E_CHANGED_MODE};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationInvokePattern,
        IUIAutomationValuePattern, TreeScope_Children, TreeScope_Descendants, UIA_AutomationIdPropertyId,
        UIA_ButtonControlTypeId, UIA_CheckBoxControlTypeId, UIA_ControlTypePropertyId, UIA_InvokePatternId,
        UIA_ValuePatternId, UIA_ValueValuePropertyId, UIA_CONTROLTYPE_ID,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
        KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_A, VK_CONTROL, VK_MENU, VK_RETURN,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    const USERNAME: &str = "username";
    const PASSWORD: &str = "password";
    const REMEMBER: &str = "remember-me";
    const KEY_GAP: Duration = Duration::from_millis(10);

    /// The window to type into: its process, and the start of its class and title.
    pub struct Target {
        pub process: &'static str,
        pub class: &'static str,
        pub title: &'static str,
    }

    pub const RIOT_CLIENT: Target = Target { process: "Riot Client.exe", class: "Chrome_WidgetWin_1", title: "Riot Client" };

    /// Keeps COM initialized on this thread while UI Automation is in use.
    pub struct Com {
        owned: bool,
    }

    impl Com {
        pub fn new() -> Result<Self, String> {
            let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if result == RPC_E_CHANGED_MODE {
                // Already initialized by someone else on this thread, which is fine.
                return Ok(Com { owned: false });
            }
            result.ok().map_err(|e| format!("Windows automation isn't available: {e}"))?;
            Ok(Com { owned: true })
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            if self.owned {
                unsafe { CoUninitialize() };
            }
        }
    }

    fn processes() -> Vec<(u32, String)> {
        let mut found = Vec::new();
        unsafe {
            let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return found };
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
            while more {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                found.push((entry.th32ProcessID, String::from_utf16_lossy(&entry.szExeFile[..len])));
                more = Process32NextW(snapshot, &mut entry).is_ok();
            }
            let _ = CloseHandle(snapshot);
        }
        found
    }

    pub fn process_running(name: &str) -> bool {
        processes().iter().any(|(_, exe)| exe.eq_ignore_ascii_case(name))
    }

    /// The folder a running program's file is in. Asking for the path needs only the
    /// lightest access, which Windows also gives for a program run as administrator.
    pub fn process_folder(name: &str) -> Option<PathBuf> {
        let pid = processes().into_iter().find(|(_, exe)| exe.eq_ignore_ascii_case(name))?.0;
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buffer = [0u16; 1024];
            let mut len = buffer.len() as u32;
            let asked = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut len);
            let _ = CloseHandle(process);
            asked.ok()?;
            PathBuf::from(String::from_utf16_lossy(&buffer[..len as usize])).parent().map(PathBuf::from)
        }
    }

    fn wait_until(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if done() {
                return true;
            }
            if Instant::now() > deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    // ── Keyboard ──

    fn key(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 },
            },
        }
    }

    fn send(inputs: &[INPUT]) {
        unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    }

    fn tap(vk: VIRTUAL_KEY) {
        send(&[key(vk, 0, KEYBD_EVENT_FLAGS(0)), key(vk, 0, KEYEVENTF_KEYUP)]);
    }

    fn chord(modifier: VIRTUAL_KEY, vk: VIRTUAL_KEY) {
        send(&[
            key(modifier, 0, KEYBD_EVENT_FLAGS(0)),
            key(vk, 0, KEYBD_EVENT_FLAGS(0)),
            key(vk, 0, KEYEVENTF_KEYUP),
            key(modifier, 0, KEYEVENTF_KEYUP),
        ]);
    }

    /// Types one UTF-16 unit as text, whatever the keyboard layout is.
    fn type_unit(unit: u16) {
        send(&[
            key(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE),
            key(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
        ]);
    }

    // ── The form ──

    pub struct Form {
        automation: IUIAutomation,
        window: HWND,
        pid: u32,
        username: IUIAutomationElement,
        password: IUIAutomationElement,
        /// Missing if the form's layout changes; typing and Enter still work then.
        submit: Option<IUIAutomationElement>,
    }

    fn automation() -> Option<IUIAutomation> {
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok()
    }

    fn find_window(automation: &IUIAutomation, target: &Target) -> Option<IUIAutomationElement> {
        let running = processes();
        unsafe {
            let any = automation.CreateTrueCondition().ok()?;
            let windows = automation.GetRootElement().ok()?.FindAll(TreeScope_Children, &any).ok()?;
            for index in 0..windows.Length().ok()? {
                let Ok(window) = windows.GetElement(index) else { continue };
                let pid = window.CurrentProcessId().unwrap_or(0) as u32;
                let title = window.CurrentName().map(|name| name.to_string()).unwrap_or_default();
                let class = window.CurrentClassName().map(|name| name.to_string()).unwrap_or_default();
                let owned = running.iter().any(|(p, exe)| *p == pid && exe.eq_ignore_ascii_case(target.process));
                if owned && title.starts_with(target.title) && class.starts_with(target.class) {
                    return Some(window);
                }
            }
        }
        None
    }

    /// The sign-in button has no name or id. It is the first button after the
    /// "Stay signed in" checkbox.
    fn find_submit(automation: &IUIAutomation, window: &IUIAutomationElement) -> Option<IUIAutomationElement> {
        unsafe {
            let of_type = |control: UIA_CONTROLTYPE_ID| {
                automation.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(control.0)).ok()
            };
            let either = automation
                .CreateOrCondition(&of_type(UIA_ButtonControlTypeId)?, &of_type(UIA_CheckBoxControlTypeId)?)
                .ok()?;
            let controls = window.FindAll(TreeScope_Descendants, &either).ok()?;
            let mut after_checkbox = false;
            for index in 0..controls.Length().ok()? {
                let Ok(control) = controls.GetElement(index) else { continue };
                if after_checkbox && control.CurrentControlType().is_ok_and(|kind| kind == UIA_ButtonControlTypeId) {
                    return Some(control);
                }
                if control.CurrentAutomationId().is_ok_and(|id| id.to_string() == REMEMBER) {
                    after_checkbox = true;
                }
            }
        }
        None
    }

    fn find_form(automation: &IUIAutomation, target: &Target) -> Option<Form> {
        let window = find_window(automation, target)?;
        unsafe {
            let field = |id: &str| {
                let by_id = automation.CreatePropertyCondition(UIA_AutomationIdPropertyId, &VARIANT::from(id)).ok()?;
                window.FindFirst(TreeScope_Descendants, &by_id).ok()
            };
            // Chromium only builds its accessibility tree once the window has been
            // shown, and then on first request. Until then the fields are missing.
            Some(Form {
                username: field(USERNAME)?,
                password: field(PASSWORD)?,
                submit: find_submit(automation, &window),
                pid: window.CurrentProcessId().unwrap_or(0) as u32,
                window: window.CurrentNativeWindowHandle().ok()?,
                automation: automation.clone(),
            })
        }
    }

    pub fn wait_for_form(target: &Target, timeout: Duration) -> Option<Form> {
        let automation = automation()?;
        let mut form = None;
        wait_until(timeout, || {
            form = find_form(&automation, target);
            form.is_some()
        });
        form
    }

    // ── Window focus ──

    pub fn foreground_window() -> HWND {
        unsafe { GetForegroundWindow() }
    }

    fn front(window: HWND) -> bool {
        unsafe {
            if IsIconic(window).as_bool() {
                let _ = ShowWindow(window, SW_RESTORE);
            }
            let _ = SetForegroundWindow(window);
            if GetForegroundWindow() != window {
                // Windows only lets a process take the foreground right after
                // user input. A tap of Alt counts as input.
                tap(VK_MENU);
                let _ = SetForegroundWindow(window);
            }
        }
        wait_until(Duration::from_secs(2), || unsafe { GetForegroundWindow() == window })
    }

    pub fn bring_to_front(target: &Target) -> Result<(), String> {
        let shown = automation()
            .and_then(|automation| find_window(&automation, target))
            .and_then(|window| unsafe { window.CurrentNativeWindowHandle() }.ok())
            .is_some_and(front);
        if shown {
            Ok(())
        } else {
            Err("Couldn't bring the Riot Client to the front. Click its window, then press Play again.".to_string())
        }
    }

    /// Returns to the window the user was in before the sign-in started.
    pub fn give_back_focus(previous: HWND) {
        if !previous.is_invalid() && unsafe { GetForegroundWindow() } != previous {
            front(previous);
        }
    }

    fn set_value(field: &IUIAutomationElement, text: &str) -> Result<(), String> {
        unsafe {
            field
                .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .and_then(|value| value.SetValue(&BSTR::from(text)))
        }
        .map_err(|e| format!("couldn't set a field: {e}"))
    }

    impl Form {
        fn in_front(&self) -> bool {
            unsafe { GetForegroundWindow() == self.window }
        }

        fn bring_to_front(&self) -> Result<(), String> {
            if front(self.window) {
                Ok(())
            } else {
                Err("Couldn't bring the Riot Client to the front. Click its window, then press Play again.".to_string())
            }
        }

        fn username_is(&self, expected: &str) -> bool {
            unsafe {
                self.username
                    .GetCurrentPropertyValue(UIA_ValueValuePropertyId)
                    .ok()
                    .and_then(|value| BSTR::try_from(&value).ok())
                    .is_some_and(|value| value.to_string() == expected)
            }
        }

        /// Sets both fields and presses the sign-in button through UI Automation.
        /// No keyboard input or focus is involved, so the user can keep working.
        pub fn sign_in_quietly(&self, username: &str, password: &str) -> Result<(), String> {
            let submit = self.submit.as_ref().ok_or("sign-in button not found")?;
            set_value(&self.username, username)?;
            set_value(&self.password, password)?;
            // The button only turns on once the form has really taken both values.
            let accepted = wait_until(Duration::from_secs(2), || {
                self.username_is(username) && unsafe { submit.CurrentIsEnabled() }.is_ok_and(|enabled| enabled.as_bool())
            });
            if !accepted {
                return Err("the form didn't accept the values".to_string());
            }
            unsafe {
                submit
                    .GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
                    .and_then(|button| button.Invoke())
            }
            .map_err(|e| format!("couldn't press the sign-in button: {e}"))
        }

        /// True when the keyboard focus is on this form's field with the given id.
        fn focused_on(&self, id: &str) -> bool {
            unsafe {
                let Ok(focused) = self.automation.GetFocusedElement() else { return false };
                focused.CurrentProcessId().is_ok_and(|pid| pid as u32 == self.pid)
                    && focused.CurrentAutomationId().is_ok_and(|found| found.to_string() == id)
            }
        }

        /// Replaces the field's contents by typing. Typing stops the moment the
        /// form isn't in front, so text can never land in another window.
        fn fill(&self, id: &str, field: &IUIAutomationElement, text: &str) -> Result<(), String> {
            self.bring_to_front()?;
            unsafe { field.SetFocus() }
                .map_err(|e| format!("Couldn't select the {id} field in the Riot Client: {e}"))?;
            if !wait_until(Duration::from_secs(2), || self.focused_on(id)) {
                return Err(format!("Couldn't select the {id} field in the Riot Client. Nothing was typed."));
            }
            chord(VK_CONTROL, VK_A);
            for (count, unit) in text.encode_utf16().enumerate() {
                if !self.in_front() || (count % 8 == 0 && !self.focused_on(id)) {
                    return Err("The Riot Client lost focus, so typing stopped. Press Play again.".to_string());
                }
                type_unit(unit);
                std::thread::sleep(KEY_GAP);
            }
            Ok(())
        }

        pub fn fill_username(&self, username: &str) -> Result<(), String> {
            self.fill(USERNAME, &self.username, username)?;
            // The username can be read back, so check it landed before typing a password.
            if wait_until(Duration::from_secs(2), || self.username_is(username)) {
                Ok(())
            } else {
                Err("The username didn't go into the Riot Client correctly, so the password wasn't typed.".to_string())
            }
        }

        pub fn fill_password(&self, password: &str) -> Result<(), String> {
            self.fill(PASSWORD, &self.password, password)
        }

        pub fn submit(&self) -> Result<(), String> {
            if !self.in_front() || !self.focused_on(PASSWORD) {
                return Err("The Riot Client lost focus before signing in. Press Play again.".to_string());
            }
            tap(VK_RETURN);
            Ok(())
        }

        /// Whether the sign-in button is turned on, or None if it wasn't found.
        #[cfg(test)]
        pub fn submit_enabled(&self) -> Option<bool> {
            let submit = self.submit.as_ref()?;
            unsafe { submit.CurrentIsEnabled() }.ok().map(|enabled| enabled.as_bool())
        }

        #[cfg(test)]
        pub fn window_title(&self) -> String {
            unsafe {
                self.automation
                    .ElementFromHandle(self.window)
                    .and_then(|window| window.CurrentName())
                    .map(|name| name.to_string())
                    .unwrap_or_default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_port_and_password_from_lockfile() {
        assert_eq!(parse_lockfile("Riot Client:64672:49227:secret:https\n"), Some((49227, "secret".to_string())));
        assert_eq!(parse_lockfile("garbage"), None);
    }

    #[test]
    fn maps_session_types() {
        let state = |kind: &str| sign_in_state(Some((200, json!({ "type": kind }))));
        assert_eq!(state("needs_credentials"), SignIn::NeedsCredentials);
        assert_eq!(state("authenticated"), SignIn::Authenticated);
        assert_eq!(state("needs_multifactor_verification"), SignIn::NeedsCode);
        assert_eq!(state("needs_password"), SignIn::Other);
        assert_eq!(sign_in_state(None), SignIn::NotReady);
        // The client answers 404 for everything while its window is closed.
        assert_eq!(sign_in_state(Some((404, json!({ "errorCode": "RESOURCE_NOT_FOUND" })))), SignIn::NotReady);
    }

    #[test]
    fn reads_riot_id_from_either_userinfo_shape() {
        let object = json!({ "acct": { "game_name": "SOLDIER BOY", "tag_line": "NZIS" } });
        assert_eq!(riot_id_from_userinfo(&object).as_deref(), Some("SOLDIER BOY#NZIS"));
        let string = json!({ "userInfo": r#"{"acct":{"game_name":"A","tag_line":"B"}}"# });
        assert_eq!(riot_id_from_userinfo(&string).as_deref(), Some("A#B"));
        assert_eq!(riot_id_from_userinfo(&json!({})), None);
    }

    #[test]
    fn compares_riot_ids_loosely() {
        assert!(same_riot_id("Soldier Boy#nzis", "SOLDIER BOY #NZIS"));
        assert!(!same_riot_id("Soldier Boy#NZIS", "Soldier Boy#EUNE"));
    }

    #[test]
    fn already_launching_counts_as_launched() {
        assert!(launch_accepted(200));
        assert!(launch_accepted(204));
        // 423 Locked: League is already launching or open. Asking again is pointless.
        assert!(launch_accepted(423));
        // 464: the player hasn't loaded yet, so ask again.
        assert!(!launch_accepted(464));
        assert!(!launch_accepted(404));
    }

    #[test]
    fn describes_known_sign_in_errors() {
        assert_eq!(describe_sign_in_error("auth_failure"), "Riot rejected the username or password.");
        assert!(describe_sign_in_error("something_new").contains("something_new"));
    }

    /// Looks at the real Riot Client without typing or changing anything. Needs its
    /// sign-in screen open: `cargo test -- --ignored live_riot_client --nocapture`
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore]
    fn live_riot_client_sign_in_form_is_found() {
        let api = LocalApi::new().unwrap();
        println!("state: {:?}", api.sign_in_state());
        println!("form result: {:?}", api.form_result());
        assert_eq!(api.sign_in_state(), SignIn::NeedsCredentials);
        let _com = ui::Com::new().unwrap();
        let form = ui::wait_for_form(&ui::RIOT_CLIENT, Duration::from_secs(15)).expect("sign-in form not found");
        // With empty fields, the sign-in button is there but turned off.
        println!("sign-in button enabled: {:?}", form.submit_enabled());
        assert_eq!(form.submit_enabled(), Some(false));
    }

    /// A small local window laid out like the sign-in form: two fields, a checkbox,
    /// and a button that only turns on once both fields have text. Submitting puts
    /// what it received into its title. No browser or account is involved.
    #[cfg(target_os = "windows")]
    fn test_window() -> std::process::Child {
        let script = r#"
Add-Type -AssemblyName System.Windows.Forms
$form = New-Object Windows.Forms.Form
$form.Text = 'VaultX sign-in test'; $form.Width = 420; $form.Height = 220; $form.StartPosition = 'CenterScreen'
$user = New-Object Windows.Forms.TextBox; $user.Name = 'username'; $user.Text = 'old text'; $user.SetBounds(20, 20, 360, 24)
$pass = New-Object Windows.Forms.TextBox; $pass.Name = 'password'; $pass.UseSystemPasswordChar = $true; $pass.SetBounds(20, 60, 360, 24)
$keep = New-Object Windows.Forms.CheckBox; $keep.Name = 'remember-me'; $keep.Text = 'Stay signed in'; $keep.SetBounds(20, 95, 200, 24)
$go = New-Object Windows.Forms.Button; $go.Text = 'Go'; $go.Enabled = $false; $go.SetBounds(20, 130, 80, 28)
$check = { $go.Enabled = ($user.Text.Length -gt 0 -and $pass.Text.Length -gt 0) }
$user.Add_TextChanged($check); $pass.Add_TextChanged($check)
$go.Add_Click({ $form.Text = 'SUBMITTED ' + $user.Text + '|' + $pass.Text })
$form.AcceptButton = $go
$form.Controls.AddRange(@($user, $pass, $keep, $go))
[Windows.Forms.Application]::Run($form)
"#;
        Command::new("powershell")
            .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", script])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .unwrap()
    }

    #[cfg(target_os = "windows")]
    const TEST_WINDOW: ui::Target = ui::Target { process: "powershell.exe", class: "WindowsForms", title: "VaultX sign-in test" };
    #[cfg(target_os = "windows")]
    const TEST_USERNAME: &str = "tëst-user_01";
    #[cfg(target_os = "windows")]
    const TEST_PASSWORD: &str = "p@ss wörd!#{}+^%~()κωδικός";

    /// Runs a sign-in against the test window and returns what the window received.
    #[cfg(target_os = "windows")]
    fn sign_in_to_test_window(sign_in: impl FnOnce(&ui::Form) -> Result<(), String>) -> String {
        let mut window = test_window();
        let outcome = (|| {
            let _com = ui::Com::new()?;
            let form = ui::wait_for_form(&TEST_WINDOW, Duration::from_secs(20)).ok_or("test window not found")?;
            sign_in(&form)?;
            std::thread::sleep(Duration::from_millis(600));
            Ok::<String, String>(form.window_title())
        })();
        let _ = window.kill();
        let _ = window.wait();
        outcome.unwrap()
    }

    /// Fills the test window without the keyboard:
    /// `cargo test -- --ignored signs_in_quietly --nocapture`
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore]
    fn signs_in_quietly_to_a_local_test_window() {
        let title = sign_in_to_test_window(|form| form.sign_in_quietly(TEST_USERNAME, TEST_PASSWORD));
        println!("window received: {title}");
        assert_eq!(title, format!("SUBMITTED {TEST_USERNAME}|{TEST_PASSWORD}"));
    }

    /// Types into the test window, taking the keyboard for a few seconds:
    /// `cargo test -- --ignored types_into_a_local_test_window --nocapture`
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore]
    fn types_into_a_local_test_window() {
        let title = sign_in_to_test_window(|form| {
            form.fill_username(TEST_USERNAME)?;
            form.fill_password(TEST_PASSWORD)?;
            form.submit()
        });
        println!("window received: {title}");
        assert_eq!(title, format!("SUBMITTED {TEST_USERNAME}|{TEST_PASSWORD}"));
    }
}
