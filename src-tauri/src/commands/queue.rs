//! Queues for a ranked game once League is open: opens the lobby, picks the roles and
//! starts the search, through the League client's own local API.

use super::auto_accept::{auth_header, find_lockfile_path, read_lockfile};
use crate::{launcher, session::SessionState};
use reqwest::blocking::Client;
use reqwest::Method;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::State;

const SUMMONER: &str = "/lol-summoner/v1/current-summoner";
const PHASE: &str = "/lol-gameflow/v1/gameflow-phase";
const AVAILABILITY: &str = "/lol-gameflow/v1/availability";
const QUEUES: &str = "/lol-game-queues/v1/queues";
const LOBBY: &str = "/lol-lobby/v2/lobby";
/// A lobby that League closes right after opening is opened again, this many times at most.
const LOBBY_ATTEMPTS: u32 = 3;
/// Newer clients answer on v2, older ones on v1.
const ROLE_PATHS: [&str; 2] = [
    "/lol-lobby/v2/lobby/members/localMember/position-preferences",
    "/lol-lobby/v1/lobby/members/localMember/position-preferences",
];
const SEARCH: &str = "/lol-lobby/v2/lobby/matchmaking/search";
const SEARCH_STATE: &str = "/lol-lobby/v2/lobby/matchmaking/search-state";

const ROLES: [&str; 6] = ["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "UTILITY", "FILL"];

/// Only one queue attempt at a time.
static QUEUEING: AtomicBool = AtomicBool::new(false);

struct Timing {
    /// League's process appearing after Play.
    open: Duration,
    /// League finishing loading the account.
    load: Duration,
    /// League being ready to open a lobby.
    lobby: Duration,
    /// How long League must stay ready, and a new lobby must stay open, before going on.
    /// A lobby opened while League is still starting up gets closed by League
    /// ("you were removed from the party").
    calm: Duration,
    /// A change showing up in the client.
    settle: Duration,
    poll: Duration,
}

const TIMING: Timing = Timing {
    open: Duration::from_secs(90),
    load: Duration::from_secs(240),
    lobby: Duration::from_secs(60),
    calm: Duration::from_secs(3),
    settle: Duration::from_secs(6),
    poll: Duration::from_secs(1),
};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Queue {
    SoloDuo,
    Flex,
}

impl Queue {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "solo" => Some(Queue::SoloDuo),
            "flex" => Some(Queue::Flex),
            _ => None,
        }
    }

    fn id(self) -> u32 {
        match self {
            Queue::SoloDuo => 420,
            Queue::Flex => 440,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Queue::SoloDuo => "Ranked Solo/Duo",
            Queue::Flex => "Ranked Flex",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Roles {
    first: String,
    second: String,
}

impl Roles {
    /// No first role means "open the lobby only". Fill takes no second role.
    fn parse(first: &str, second: &str) -> Result<Option<Self>, String> {
        let first = first.trim().to_uppercase();
        let second = second.trim().to_uppercase();
        if first.is_empty() {
            return Ok(None);
        }
        if !ROLES.contains(&first.as_str()) {
            return Err(format!("Unknown role: {first}."));
        }
        if first == "FILL" {
            return Ok(Some(Roles { first, second: "UNSELECTED".to_string() }));
        }
        if !ROLES.contains(&second.as_str()) {
            return Err("Pick a second role.".to_string());
        }
        if first == second {
            return Err("The two roles must be different.".to_string());
        }
        Ok(Some(Roles { first, second }))
    }

    fn label(&self) -> String {
        if self.first == "FILL" {
            return role_name(&self.first).to_string();
        }
        format!("{} / {}", role_name(&self.first), role_name(&self.second))
    }

    /// Whether the lobby shows these roles for the local player.
    fn shown_in(&self, lobby: &Value) -> bool {
        let member = &lobby["localMember"];
        let first = member["firstPositionPreference"].as_str().unwrap_or_default();
        let second = member["secondPositionPreference"].as_str().unwrap_or_default();
        first == self.first && (self.first == "FILL" || second == self.second)
    }
}

fn role_name(role: &str) -> &str {
    match role {
        "TOP" => "Top",
        "JUNGLE" => "Jungle",
        "MIDDLE" => "Mid",
        "BOTTOM" => "Bot",
        "UTILITY" => "Support",
        "FILL" => "Fill",
        other => other,
    }
}

/// The League client's local API.
struct Lcu {
    client: Client,
    base: String,
    auth: String,
}

impl Lcu {
    fn from_lockfile(client: &Client) -> Option<Self> {
        let lockfile = read_lockfile(&find_lockfile_path()?)?;
        Some(Lcu {
            client: client.clone(),
            base: format!("{}://127.0.0.1:{}", lockfile.protocol, lockfile.port),
            auth: auth_header(&lockfile.password),
        })
    }

    /// The status and JSON body; `None` when the client didn't answer.
    fn send(&self, method: Method, path: &str, body: Option<&Value>) -> Option<(u16, Value)> {
        let mut request = self
            .client
            .request(method, format!("{}{}", self.base, path))
            .header("Authorization", &self.auth);
        if let Some(body) = body {
            request = request.header("Content-Type", "application/json").body(body.to_string());
        }
        let response = request.send().ok()?;
        let status = response.status().as_u16();
        let text = response.text().unwrap_or_default();
        Some((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
    }

    fn get(&self, path: &str) -> Option<Value> {
        match self.send(Method::GET, path, None)? {
            (status, body) if succeeded(status) => Some(body),
            _ => None,
        }
    }
}

fn succeeded(status: u16) -> bool {
    (200..300).contains(&status)
}

fn wait_until(timeout: Duration, poll: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(poll);
    }
}

fn summoner_riot_id(summoner: &Value) -> Option<String> {
    let name = summoner["gameName"].as_str().filter(|name| !name.is_empty())?;
    let tag = summoner["tagLine"].as_str().filter(|tag| !tag.is_empty())?;
    Some(format!("{name}#{tag}"))
}

/// Why nothing is queued when League is already doing something.
fn busy_message(phase: &str) -> String {
    match phase {
        "Lobby" => "League already has a lobby open, so nothing was changed.".to_string(),
        "Matchmaking" | "ReadyCheck" => "League is already in queue.".to_string(),
        "ChampSelect" => "League is already in champion select.".to_string(),
        "GameStart" | "InProgress" | "Reconnect" => {
            "This account has a game in progress, so nothing was queued.".to_string()
        }
        other => format!("League is busy ({other}), so nothing was queued."),
    }
}

/// League's own reason from an error answer, as ": reason" or nothing.
fn reason(body: &Value) -> String {
    match body["message"].as_str().filter(|message| !message.is_empty()) {
        Some(message) => format!(": {message}"),
        None => String::new(),
    }
}

/// What stops the search, e.g. a leaver penalty, from the search state.
fn search_error(state: &Value) -> Option<String> {
    let error = state["errors"].as_array()?.first()?;
    let what = error["message"]
        .as_str()
        .filter(|message| !message.is_empty())
        .or_else(|| error["errorType"].as_str())
        .unwrap_or("unknown reason");
    let wait = error["penaltyTimeRemaining"].as_f64().unwrap_or(0.0);
    Some(if wait > 0.0 {
        format!("{what} (wait {} min)", (wait / 60.0).ceil())
    } else {
        what.to_string()
    })
}

/// Waits for League to open and load the account.
fn wait_for_league(client: &Client, timing: &Timing, running: impl Fn() -> bool) -> Result<Lcu, String> {
    let started = Instant::now();
    let mut seen = false;
    loop {
        if running() {
            seen = true;
            if let Some(lcu) = Lcu::from_lockfile(client) {
                if lcu.get(SUMMONER).is_some() {
                    return Ok(lcu);
                }
            }
        } else if seen {
            return Err("League was closed, so nothing was queued.".to_string());
        } else if started.elapsed() >= timing.open {
            return Err("League didn't open, so nothing was queued.".to_string());
        }
        if started.elapsed() >= timing.load {
            return Err("League took too long to load, so nothing was queued.".to_string());
        }
        std::thread::sleep(timing.poll);
    }
}

/// The queue the open lobby is for; `None` when there is no lobby.
fn lobby_queue(lcu: &Lcu) -> Option<u32> {
    lcu.get(LOBBY)?["gameConfig"]["queueId"].as_u64().map(|id| id as u32)
}

/// Waits until League is idle, lets lobbies be opened and has loaded the queue, and has
/// stayed that way for a moment. Right after sign-in it accepts a lobby a few seconds
/// before it can keep one.
fn wait_until_ready(lcu: &Lcu, queue: Queue, timing: &Timing) -> Result<(), String> {
    let deadline = Instant::now() + timing.lobby;
    let mut ready_since: Option<Instant> = None;
    let mut queue_loaded = true;
    loop {
        match lcu.get(PHASE).as_ref().and_then(Value::as_str) {
            Some("None") => {
                let available = lcu.get(AVAILABILITY).is_some_and(|state| state["isAvailable"] == true);
                queue_loaded = lcu
                    .get(&format!("{QUEUES}/{}", queue.id()))
                    .is_some_and(|details| details["queueAvailability"] == "Available");
                if available && queue_loaded {
                    if ready_since.get_or_insert_with(Instant::now).elapsed() >= timing.calm {
                        return Ok(());
                    }
                } else {
                    ready_since = None;
                }
            }
            Some(phase) => return Err(busy_message(phase)),
            None => ready_since = None,
        }
        if Instant::now() >= deadline {
            return Err(if queue_loaded {
                "League isn't ready yet, so nothing was queued.".to_string()
            } else {
                format!("{} isn't available right now, so nothing was queued.", queue.name())
            });
        }
        std::thread::sleep(timing.poll);
    }
}

/// Whether a lobby just opened is still there, for the right queue, a moment later.
fn lobby_stays_open(lcu: &Lcu, queue: Queue, timing: &Timing) -> bool {
    let until = Instant::now() + timing.calm;
    while Instant::now() < until {
        if lcu.get(LOBBY).is_none() {
            return false;
        }
        std::thread::sleep(timing.poll);
    }
    lobby_queue(lcu) == Some(queue.id())
}

/// Opens the lobby, sets the roles and starts the search. Returns a message to show.
/// Stops before anything it can't confirm: wrong account, League busy, roles not set.
fn queue_up_in(
    lcu: &Lcu,
    queue: Queue,
    roles: Option<&Roles>,
    expected_riot_id: &str,
    timing: &Timing,
) -> Result<String, String> {
    let signed_in = lcu.get(SUMMONER).as_ref().and_then(summoner_riot_id);
    if let Some(riot_id) = signed_in {
        if !expected_riot_id.trim().is_empty() && !launcher::same_riot_id(&riot_id, expected_riot_id) {
            return Err(format!("League is signed in as {riot_id}, not this account, so nothing was queued."));
        }
    }

    let name = queue.name();
    wait_until_ready(lcu, queue, timing)?;

    // Open the lobby and set the roles. League can close a lobby it wasn't ready for,
    // so each step checks the lobby is still there, and a closed one is opened again.
    let lobby = json!({ "queueId": queue.id() });
    let preferences = roles.map(|roles| json!({ "firstPreference": roles.first, "secondPreference": roles.second }));
    let mut attempt = 0;
    loop {
        attempt += 1;
        let why = match lcu.send(Method::POST, LOBBY, Some(&lobby)) {
            Some((status, _)) if succeeded(status) => {
                if !lobby_stays_open(lcu, queue, timing) {
                    ": League closed it".to_string()
                } else if let (Some(roles), Some(preferences)) = (roles, &preferences) {
                    let sent = ROLE_PATHS.iter().any(|path| {
                        matches!(lcu.send(Method::PUT, path, Some(preferences)), Some((status, _)) if succeeded(status))
                    });
                    let set = sent
                        && wait_until(timing.settle, timing.poll, || {
                            lcu.get(LOBBY).is_some_and(|lobby| roles.shown_in(&lobby))
                        });
                    if set {
                        break;
                    }
                    if lobby_queue(lcu) == Some(queue.id()) {
                        return Ok(format!(
                            "{name} lobby is open, but the roles couldn't be set. Pick them and press Find Match."
                        ));
                    }
                    ": League closed it".to_string()
                } else {
                    break;
                }
            }
            Some((_, body)) => reason(&body),
            None => String::new(),
        };
        if attempt >= LOBBY_ATTEMPTS {
            return Err(format!("Couldn't open a {name} lobby{why}."));
        }
        println!("[Queue] Lobby attempt {attempt} failed{why}");
        std::thread::sleep(timing.calm);
    }

    let Some(roles) = roles else {
        return Ok(format!("{name} lobby is open. Pick your roles and press Find Match."));
    };

    match lcu.send(Method::POST, SEARCH, None) {
        Some((status, _)) if succeeded(status) => {}
        Some((_, body)) => return Err(format!("League wouldn't start the queue{}.", reason(&body))),
        None => return Err("League didn't answer when starting the queue.".to_string()),
    }

    let mut state = Value::Null;
    wait_until(timing.settle, timing.poll, || {
        state = lcu.get(SEARCH_STATE).unwrap_or(Value::Null);
        state["searchState"] == "Searching" || search_error(&state).is_some()
    });
    if let Some(error) = search_error(&state) {
        return Err(format!("League couldn't queue: {error}."));
    }
    if state["searchState"] == "Searching" {
        Ok(format!("In queue for {name} as {}.", roles.label()))
    } else {
        Ok(format!("Asked League to queue for {name} as {}. Check its window.", roles.label()))
    }
}

fn queue_up_blocking(queue: Queue, roles: Option<Roles>, riot_id: String) -> Result<String, String> {
    let client = Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| format!("Couldn't reach League: {e}"))?;
    let lcu = wait_for_league(&client, &TIMING, launcher::league_running)?;
    queue_up_in(&lcu, queue, roles.as_ref(), &riot_id, &TIMING)
}

/// Waits for League to open as this account, then queues for a ranked game.
/// `queue` is "solo" or "flex". With no first role, only the lobby is opened.
#[tauri::command]
pub async fn queue_up(
    id: String,
    queue: String,
    first_role: String,
    second_role: String,
    state: State<'_, Mutex<SessionState>>,
) -> Result<String, String> {
    let riot_id = {
        let s = state.lock().unwrap();
        if !s.is_authenticated() { return Err("Not authenticated".to_string()); }
        s.vault.as_ref().unwrap().accounts.iter()
            .find(|a| a.id == id)
            .ok_or("Account not found")?
            .riot_id.clone()
    };
    let queue = Queue::parse(&queue).ok_or("Unknown queue.")?;
    let roles = Roles::parse(&first_role, &second_role)?;

    if QUEUEING.swap(true, Ordering::SeqCst) {
        return Err("A queue attempt is already in progress.".to_string());
    }
    let result = tauri::async_runtime::spawn_blocking(move || queue_up_blocking(queue, roles, riot_id)).await;
    QUEUEING.store(false, Ordering::SeqCst);
    result.map_err(|e| format!("Queue task failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    /// A stand-in for the League client: answers the calls the queue step makes and
    /// records them. Nothing here touches the real client.
    struct FakeLeague {
        phase: &'static str,
        riot_id: (&'static str, &'static str),
        /// Whether a role change is kept.
        keeps_roles: bool,
        search_state: Value,
        /// How many availability checks say "not yet", as right after sign-in.
        busy_checks: usize,
        queue_availability: &'static str,
        /// How many lobbies League closes right after they open.
        lobbies_closed: usize,
        /// The queue of each lobby opened so far.
        lobbies: Mutex<Vec<u64>>,
        roles: Mutex<(String, String)>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeLeague {
        fn new() -> Self {
            FakeLeague {
                phase: "None",
                riot_id: ("Test Player", "EUW"),
                keeps_roles: true,
                search_state: json!({ "errors": [], "searchState": "Searching" }),
                busy_checks: 0,
                queue_availability: "Available",
                lobbies_closed: 0,
                lobbies: Mutex::new(Vec::new()),
                roles: Mutex::new(("UNSELECTED".to_string(), "UNSELECTED".to_string())),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn answer(&self, method: &str, path: &str, body: &str) -> (u16, String) {
            self.calls.lock().unwrap().push(format!("{method} {path} {body}").trim().to_string());
            match (method, path) {
                ("GET", SUMMONER) => {
                    (200, json!({ "gameName": self.riot_id.0, "tagLine": self.riot_id.1 }).to_string())
                }
                ("GET", PHASE) => (200, json!(self.phase).to_string()),
                ("GET", AVAILABILITY) => {
                    let checks = self.count(&format!("GET {AVAILABILITY}"));
                    (200, json!({ "isAvailable": checks > self.busy_checks }).to_string())
                }
                ("GET", path) if path.starts_with(QUEUES) => {
                    (200, json!({ "queueAvailability": self.queue_availability }).to_string())
                }
                ("POST", LOBBY) => {
                    let sent: Value = serde_json::from_str(body).unwrap();
                    self.lobbies.lock().unwrap().push(sent["queueId"].as_u64().unwrap());
                    (200, "{}".to_string())
                }
                // Like an older client: roles only on v1.
                ("PUT", path) if path == ROLE_PATHS[0] => (404, json!({ "message": "not found" }).to_string()),
                ("PUT", path) if path == ROLE_PATHS[1] => {
                    if self.keeps_roles && self.open_lobby().is_some() {
                        let sent: Value = serde_json::from_str(body).unwrap();
                        *self.roles.lock().unwrap() = (
                            sent["firstPreference"].as_str().unwrap().to_string(),
                            sent["secondPreference"].as_str().unwrap().to_string(),
                        );
                    }
                    (204, String::new())
                }
                ("GET", LOBBY) => match self.open_lobby() {
                    Some(queue) => {
                        let roles = self.roles.lock().unwrap();
                        let member = json!({ "firstPositionPreference": roles.0, "secondPositionPreference": roles.1 });
                        (200, json!({ "gameConfig": { "queueId": queue }, "localMember": member }).to_string())
                    }
                    None => (404, json!({ "message": "LOBBY_NOT_FOUND" }).to_string()),
                },
                ("POST", SEARCH) => (204, String::new()),
                ("GET", SEARCH_STATE) => (200, self.search_state.to_string()),
                _ => (404, "{}".to_string()),
            }
        }

        /// The queue of the lobby that is open now, if League kept one.
        fn open_lobby(&self) -> Option<u64> {
            let lobbies = self.lobbies.lock().unwrap();
            lobbies.last().copied().filter(|_| lobbies.len() > self.lobbies_closed)
        }

        fn count(&self, call: &str) -> usize {
            self.calls.lock().unwrap().iter().filter(|made| made.starts_with(call)).count()
        }

        /// Calls that change something, in order.
        fn changes(&self) -> Vec<String> {
            self.calls.lock().unwrap().iter().filter(|call| !call.starts_with("GET")).cloned().collect()
        }
    }

    fn serve(league: Arc<FakeLeague>) -> Lcu {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut request = Vec::new();
                let mut chunk = [0u8; 2048];
                let (head, body) = loop {
                    let read = stream.read(&mut chunk).unwrap_or(0);
                    request.extend_from_slice(&chunk[..read]);
                    let text = String::from_utf8_lossy(&request).to_string();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let length = head
                            .lines()
                            .find_map(|line| {
                                line.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|n| n.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if body.len() >= length || read == 0 {
                            break (head.to_string(), body.to_string());
                        }
                    } else if read == 0 {
                        break (text, String::new());
                    }
                };
                let mut first_line = head.lines().next().unwrap_or_default().split(' ');
                let method = first_line.next().unwrap_or_default();
                let path = first_line.next().unwrap_or_default();
                let (status, answer) = league.answer(method, path, &body);
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                    answer.len()
                );
            }
        });
        Lcu {
            client: Client::builder().timeout(Duration::from_secs(5)).build().unwrap(),
            base: format!("http://127.0.0.1:{port}"),
            auth: auth_header("test"),
        }
    }

    const FAST: Timing = Timing {
        open: Duration::from_millis(300),
        load: Duration::from_millis(600),
        lobby: Duration::from_millis(900),
        calm: Duration::from_millis(120),
        settle: Duration::from_millis(400),
        poll: Duration::from_millis(40),
    };

    fn roles(first: &str, second: &str) -> Roles {
        Roles::parse(first, second).unwrap().unwrap()
    }

    #[test]
    fn opens_the_lobby_sets_roles_and_starts_the_search() {
        let league = Arc::new(FakeLeague::new());
        let lcu = serve(league.clone());

        let message =
            queue_up_in(&lcu, Queue::SoloDuo, Some(&roles("MIDDLE", "JUNGLE")), "test player#euw", &FAST).unwrap();

        assert_eq!(message, "In queue for Ranked Solo/Duo as Mid / Jungle.");
        assert_eq!(league.changes(), vec![
            format!(r#"POST {LOBBY} {{"queueId":420}}"#),
            format!(r#"PUT {} {{"firstPreference":"MIDDLE","secondPreference":"JUNGLE"}}"#, ROLE_PATHS[0]),
            format!(r#"PUT {} {{"firstPreference":"MIDDLE","secondPreference":"JUNGLE"}}"#, ROLE_PATHS[1]),
            format!("POST {SEARCH}"),
        ]);
    }

    #[test]
    fn flex_uses_its_own_queue() {
        let league = Arc::new(FakeLeague::new());
        let lcu = serve(league.clone());

        let message = queue_up_in(&lcu, Queue::Flex, Some(&roles("FILL", "")), "", &FAST).unwrap();

        assert_eq!(message, "In queue for Ranked Flex as Fill.");
        assert_eq!(league.changes()[0], format!(r#"POST {LOBBY} {{"queueId":440}}"#));
    }

    #[test]
    fn without_roles_only_the_lobby_is_opened() {
        let league = Arc::new(FakeLeague::new());
        let lcu = serve(league.clone());

        let message = queue_up_in(&lcu, Queue::SoloDuo, None, "Test Player#EUW", &FAST).unwrap();

        assert_eq!(message, "Ranked Solo/Duo lobby is open. Pick your roles and press Find Match.");
        assert_eq!(league.changes(), vec![format!(r#"POST {LOBBY} {{"queueId":420}}"#)]);
    }

    #[test]
    fn another_account_is_never_queued() {
        let league = Arc::new(FakeLeague::new());
        let lcu = serve(league.clone());

        let error =
            queue_up_in(&lcu, Queue::SoloDuo, Some(&roles("TOP", "FILL")), "Someone Else#EUW", &FAST).unwrap_err();

        assert_eq!(error, "League is signed in as Test Player#EUW, not this account, so nothing was queued.");
        assert!(league.changes().is_empty());
    }

    #[test]
    fn a_busy_client_is_left_alone() {
        for (phase, expected) in [
            ("Lobby", "League already has a lobby open, so nothing was changed."),
            ("Matchmaking", "League is already in queue."),
            ("ChampSelect", "League is already in champion select."),
            ("InProgress", "This account has a game in progress, so nothing was queued."),
        ] {
            let league = Arc::new(FakeLeague { phase, ..FakeLeague::new() });
            let lcu = serve(league.clone());

            let error = queue_up_in(&lcu, Queue::SoloDuo, Some(&roles("TOP", "FILL")), "", &FAST).unwrap_err();

            assert_eq!(error, expected);
            assert!(league.changes().is_empty());
        }
    }

    #[test]
    fn the_search_never_starts_when_roles_did_not_stick() {
        let league = Arc::new(FakeLeague { keeps_roles: false, ..FakeLeague::new() });
        let lcu = serve(league.clone());

        let message = queue_up_in(&lcu, Queue::SoloDuo, Some(&roles("BOTTOM", "UTILITY")), "", &FAST).unwrap();

        assert_eq!(
            message,
            "Ranked Solo/Duo lobby is open, but the roles couldn't be set. Pick them and press Find Match."
        );
        assert!(!league.changes().contains(&format!("POST {SEARCH}")));
    }

    #[test]
    fn a_penalty_is_reported() {
        let search_state = json!({
            "errors": [{ "errorType": "QUEUE_DODGER", "message": "QUEUE_DODGER", "penaltyTimeRemaining": 290.0 }],
            "searchState": "Error",
        });
        let league = Arc::new(FakeLeague { search_state, ..FakeLeague::new() });
        let lcu = serve(league);

        let error = queue_up_in(&lcu, Queue::SoloDuo, Some(&roles("TOP", "MIDDLE")), "", &FAST).unwrap_err();

        assert_eq!(error, "League couldn't queue: QUEUE_DODGER (wait 5 min).");
    }

    #[test]
    fn waits_until_league_has_been_ready_for_a_moment() {
        let league = Arc::new(FakeLeague { busy_checks: 3, ..FakeLeague::new() });
        let lcu = serve(league.clone());

        let message = queue_up_in(&lcu, Queue::SoloDuo, Some(&roles("TOP", "MIDDLE")), "", &FAST).unwrap();

        assert_eq!(message, "In queue for Ranked Solo/Duo as Top / Mid.");
        let calls = league.calls.lock().unwrap().clone();
        let opened = calls.iter().position(|call| call.starts_with(&format!("POST {LOBBY}"))).unwrap();
        let checks = calls[..opened].iter().filter(|call| call.starts_with(&format!("GET {AVAILABILITY}"))).count();
        // Three "not yet", then ready at least twice in a row before the lobby is opened.
        assert!(checks >= 5, "opened the lobby after only {checks} checks");
    }

    #[test]
    fn a_lobby_league_closes_is_opened_again() {
        let league = Arc::new(FakeLeague { lobbies_closed: 1, ..FakeLeague::new() });
        let lcu = serve(league.clone());

        let message = queue_up_in(&lcu, Queue::SoloDuo, Some(&roles("MIDDLE", "TOP")), "", &FAST).unwrap();

        assert_eq!(message, "In queue for Ranked Solo/Duo as Mid / Top.");
        assert_eq!(league.count(&format!("POST {LOBBY} ")), 2);
        // No roles and no search on the lobby that was closed.
        assert_eq!(league.changes()[1], format!(r#"POST {LOBBY} {{"queueId":420}}"#));
        assert_eq!(league.count(&format!("POST {SEARCH}")), 1);
    }

    #[test]
    fn gives_up_when_league_keeps_closing_the_lobby() {
        let league = Arc::new(FakeLeague { lobbies_closed: 99, ..FakeLeague::new() });
        let lcu = serve(league.clone());

        let error = queue_up_in(&lcu, Queue::SoloDuo, Some(&roles("MIDDLE", "TOP")), "", &FAST).unwrap_err();

        assert_eq!(error, "Couldn't open a Ranked Solo/Duo lobby: League closed it.");
        assert_eq!(league.count(&format!("POST {LOBBY} ")), 3);
        assert_eq!(league.count("PUT"), 0);
        assert_eq!(league.count(&format!("POST {SEARCH}")), 0);
    }

    #[test]
    fn a_queue_that_is_off_is_not_opened() {
        let league = Arc::new(FakeLeague { queue_availability: "PlatformDisabled", ..FakeLeague::new() });
        let lcu = serve(league.clone());

        let error = queue_up_in(&lcu, Queue::Flex, Some(&roles("FILL", "")), "", &FAST).unwrap_err();

        assert_eq!(error, "Ranked Flex isn't available right now, so nothing was queued.");
        assert!(league.changes().is_empty());
    }

    #[test]
    fn gives_up_when_league_never_opens() {
        let client = Client::new();
        let error = wait_for_league(&client, &FAST, || false).err().unwrap();
        assert_eq!(error, "League didn't open, so nothing was queued.");
    }

    #[test]
    fn roles_are_checked() {
        assert_eq!(Roles::parse("", "").unwrap(), None);
        assert_eq!(
            Roles::parse("fill", "TOP").unwrap(),
            Some(Roles { first: "FILL".into(), second: "UNSELECTED".into() })
        );
        assert_eq!(Roles::parse("TOP", "FILL").unwrap().unwrap().label(), "Top / Fill");
        assert!(Roles::parse("TOP", "TOP").is_err());
        assert!(Roles::parse("TOP", "").is_err());
        assert!(Roles::parse("ADC", "TOP").is_err());
    }

    #[test]
    fn queues_are_parsed() {
        assert_eq!(Queue::parse("solo").map(Queue::id), Some(420));
        assert_eq!(Queue::parse("flex").map(Queue::id), Some(440));
        assert_eq!(Queue::parse("aram"), None);
    }
}
