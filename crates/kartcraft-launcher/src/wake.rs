//! The staging server's wake. Staging deallocates itself when nobody has played for half an hour
//! (its `kartcraft-idle` timer); the testers' launcher wakes it before they log in: when its login
//! port does not answer, the players' site starts it (`/staging/wake`, which runs all the time),
//! and the launcher waits for the port.

use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

/// The site that wakes staging: the players', which never sleeps.
pub const WAKE_URL: &str = "https://worldofkartcraft.com/staging/wake";
/// The login server's port (a 1.12 realmd's).
const LOGIN_PORT: u16 = 3724;
/// How long to wait for a woken server, and how often to look.
pub const PATIENCE: Duration = Duration::from_secs(300);
const LOOK_EVERY: Duration = Duration::from_secs(5);
/// How often to ask for the wake again while waiting (the site starts it at most once a minute).
const ASK_EVERY: Duration = Duration::from_secs(60);

/// `server` with the login port where it names none.
fn address(server: &str) -> String {
    if server.contains(':') {
        server.to_string()
    } else {
        format!("{server}:{LOGIN_PORT}")
    }
}

/// Whether `server`'s login port answers.
pub fn up(server: &str) -> bool {
    let Ok(addrs) = address(server).to_socket_addrs() else {
        return false;
    };
    addrs
        .into_iter()
        .any(|a| TcpStream::connect_timeout(&a, Duration::from_secs(3)).is_ok())
}

/// Asks the site to start staging; its state as the site says (`running`, `starting`...).
pub fn ask() -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout_read(Duration::from_secs(20))
        .build();
    let body = agent
        .post(WAKE_URL)
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    Ok(v.get("state")
        .and_then(|s| s.as_str())
        .unwrap_or("unknown")
        .to_string())
}

/// Waits for `server` to answer, waking it, `say` told what is happening; whether it is up.
pub fn until_up(server: &str, mut say: impl FnMut(String)) -> bool {
    if up(server) {
        return true;
    }
    let start = Instant::now();
    let mut asked: Option<Instant> = None;
    while start.elapsed() < PATIENCE {
        if asked.is_none_or(|a| a.elapsed() >= ASK_EVERY) {
            asked = Some(Instant::now());
            if let Err(e) = ask() {
                say(format!(
                    "The test server is asleep and could not be woken ({e})."
                ));
                return false;
            }
        }
        let secs = start.elapsed().as_secs();
        say(format!(
            "Waking the test server… {}:{:02} (usually about two minutes)",
            secs / 60,
            secs % 60
        ));
        std::thread::sleep(LOOK_EVERY);
        if up(server) {
            return true;
        }
    }
    say("The test server did not wake in time: try again in a minute.".into());
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_without_a_port_is_its_login_port() {
        assert_eq!(address("kartcraft.example"), "kartcraft.example:3724");
        assert_eq!(address("127.0.0.1:4000"), "127.0.0.1:4000");
    }

    #[test]
    fn a_port_nothing_listens_on_is_down() {
        // Bound and dropped: nothing answers there now.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(!up(&format!("127.0.0.1:{port}")));
        let live = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        assert!(up(&live.local_addr().unwrap().to_string()));
    }
}
