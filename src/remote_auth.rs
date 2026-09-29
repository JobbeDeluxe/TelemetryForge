use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rand_core::OsRng;

pub fn hash_password(password: &str) -> Result<String, String> {
    if password.len() < 8 {
        return Err("Password must contain at least 8 characters".into());
    }
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| error.to_string())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).ok().is_some_and(|parsed| {
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    })
}

/// Cache successful credentials briefly so preview polling does not run
/// Argon2 for every frame. Hash changes/revocation invalidate old entries.
pub async fn verify_request(password: String, hash: String) -> bool {
    use std::{
        sync::{Arc, LazyLock, Mutex},
        time::{Duration, Instant},
    };
    static VERIFIED: LazyLock<Mutex<Vec<(String, String, Instant)>>> =
        LazyLock::new(|| Mutex::new(Vec::new()));
    static SLOTS: LazyLock<Arc<tokio::sync::Semaphore>> =
        LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(2)));
    if password.len() > 1024 || hash.is_empty() {
        return false;
    }
    {
        let mut entries = VERIFIED.lock().expect("auth cache poisoned");
        entries.retain(|entry| entry.2.elapsed() < Duration::from_secs(30));
        if entries
            .iter()
            .any(|entry| entry.0 == password && entry.1 == hash)
        {
            return true;
        }
    }
    let Ok(permit) = SLOTS.clone().try_acquire_owned() else {
        return false;
    };
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        if !verify_password(&password, &hash) {
            return false;
        }
        let mut entries = VERIFIED.lock().expect("auth cache poisoned");
        if entries.len() >= 32 {
            entries.remove(0);
        }
        entries.push((password, hash, Instant::now()));
        true
    })
    .await
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hashes_are_not_plaintext_and_verify() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(!hash.contains("correct horse battery staple"));
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("wrong password", &hash));
    }
}
