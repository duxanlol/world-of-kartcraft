//! "Stay logged in", which 1.12 lacks (its login keeps only the account name): after a login with
//! the box checked, the client keeps the account name and its credentials hash,
//! `SHA1(UPPER(name) ":" UPPER(password))`, the one value the SRP6 exchange takes from the
//! password (`benilla_protocol::Secret::Saved`), and logs in by itself the next time it starts.
//! The password is never stored. Changing it on the website ends every kept login; a refusal,
//! unchecking the box and Back from character select forget it here.
//!
//! The file is the account name, then the hash in hex, a line each, in the state folder
//! ([`crate::local_state::saved_login_path`]).

use std::path::Path;

/// The kept login: the account name and its hash.
pub(crate) fn load() -> Option<(String, [u8; 20])> {
    load_from(&crate::local_state::saved_login_path()?)
}

/// Keep `user`'s login.
pub(crate) fn save(user: &str, hash: &[u8; 20]) {
    if let Some(path) = crate::local_state::saved_login_path() {
        save_to(&path, user, hash);
    }
}

/// Forget the kept login, if there is one.
pub(crate) fn forget() {
    if let Some(path) = crate::local_state::saved_login_path() {
        let _ = std::fs::remove_file(path);
    }
}

fn load_from(path: &Path) -> Option<(String, [u8; 20])> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines().map(str::trim);
    let user = lines.next().filter(|u| !u.is_empty())?.to_string();
    let hex = lines.next()?;
    let mut hash = [0u8; 20];
    if hex.len() != 40 {
        return None;
    }
    for (i, byte) in hash.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some((user, hash))
}

fn save_to(path: &Path, user: &str, hash: &[u8; 20]) {
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    if let Err(e) = crate::local_state::write_atomic(path, &format!("{user}\n{hex}\n")) {
        bevy::log::warn!("login: keeping the login failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_login_reads_back_and_a_damaged_one_reads_as_none() {
        let dir = std::env::temp_dir().join(format!("benilla-saved-login-{}", std::process::id()));
        let path = dir.join("login");
        let hash: [u8; 20] = std::array::from_fn(|i| (i * 13) as u8);
        save_to(&path, "Player", &hash);
        assert_eq!(load_from(&path), Some(("Player".to_string(), hash)));
        std::fs::write(&path, "Player\nnot-hex\n").unwrap();
        assert_eq!(load_from(&path), None);
        std::fs::write(&path, "\n").unwrap();
        assert_eq!(load_from(&path), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
