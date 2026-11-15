//! Admin password (PBKDF2-HMAC-SHA256), browser sessions and API tokens. Randomness and time are passed
//! in (the ESP has a hardware RNG, tests use fixed bytes). Only hashes are stored.

use crate::crypto::{ct_eq, hex, pbkdf2_sha256, sha256, unhex};

pub const ITERATIONS: u32 = 10_000;
pub const MAX_FAILURES: u32 = 5;
pub const LOCKOUT_MS: u64 = 60_000;
pub const SESSION_IDLE_MS: u64 = 30 * 60_000;
pub const MAX_TOKENS: usize = 8;
pub const MAX_SESSIONS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    Wrong,
    Locked { until_ms: u64 },
    NoPassword,
}

#[derive(Debug, Clone, Default)]
pub struct Auth {
    hash: String,
    failures: u32,
    last_failure: u64,
    sessions: Vec<(String, u64)>,
    tokens: Vec<(String, String)>,
}

impl Auth {
    /// From the stored password hash (`pbkdf2$<iters>$<salt hex>$<hash hex>` or empty) and token list.
    pub fn from_hash(hash: &str) -> Auth {
        Auth { hash: hash.to_string(), ..Default::default() }
    }

    pub fn has_password(&self) -> bool {
        !self.hash.is_empty()
    }

    pub fn hash(&self) -> &str {
        &self.hash
    }

    pub fn set_password(&mut self, pw: &str, salt: [u8; 16]) {
        let h = pbkdf2_sha256(pw.as_bytes(), &salt, ITERATIONS, 32);
        self.hash = format!("pbkdf2${ITERATIONS}${}${}", hex(&salt), hex(&h));
        self.sessions.clear();
        self.failures = 0;
    }

    fn verify(&self, pw: &str) -> bool {
        let parts: Vec<&str> = self.hash.split('$').collect();
        let [kind, iters, salt, want] = parts.as_slice() else { return false };
        let (Ok(iters), Some(salt), Some(want)) = (iters.parse::<u32>(), unhex(salt), unhex(want)) else { return false };
        *kind == "pbkdf2" && iters > 0 && iters <= 1_000_000 && ct_eq(&pbkdf2_sha256(pw.as_bytes(), &salt, iters, want.len()), &want)
    }

    pub fn check_password(&mut self, pw: &str, now_ms: u64) -> Result<(), AuthError> {
        if !self.has_password() {
            return Err(AuthError::NoPassword);
        }
        if self.failures >= MAX_FAILURES {
            let until = self.last_failure + LOCKOUT_MS;
            if now_ms < until {
                return Err(AuthError::Locked { until_ms: until });
            }
            self.failures = 0;
        }
        if self.verify(pw) {
            self.failures = 0;
            Ok(())
        } else {
            self.failures += 1;
            self.last_failure = now_ms;
            Err(AuthError::Wrong)
        }
    }

    /// Check the password and open a session; returns the session token (hex of `random`).
    pub fn login(&mut self, pw: &str, now_ms: u64, random: [u8; 32]) -> Result<String, AuthError> {
        self.check_password(pw, now_ms)?;
        let t = hex(&random);
        self.sessions.retain(|(_, last)| now_ms.saturating_sub(*last) < SESSION_IDLE_MS);
        if self.sessions.len() >= MAX_SESSIONS {
            self.sessions.remove(0);
        }
        self.sessions.push((t.clone(), now_ms));
        Ok(t)
    }

    pub fn logout(&mut self, token: &str) {
        self.sessions.retain(|(t, _)| !ct_eq(t.as_bytes(), token.as_bytes()));
    }

    /// A valid session (refreshed on use) or API token.
    pub fn bearer_ok(&mut self, token: &str, now_ms: u64) -> bool {
        self.sessions.retain(|(_, last)| now_ms.saturating_sub(*last) < SESSION_IDLE_MS);
        if let Some(s) = self.sessions.iter_mut().find(|(t, _)| ct_eq(t.as_bytes(), token.as_bytes())) {
            s.1 = now_ms;
            return true;
        }
        let h = hex(&sha256(token.as_bytes()));
        self.tokens.iter().any(|(_, th)| ct_eq(th.as_bytes(), h.as_bytes()))
    }

    /// New API token (shown once); `None` when the name is taken or the limit is reached.
    pub fn create_token(&mut self, name: &str, random: [u8; 32]) -> Option<String> {
        if self.tokens.len() >= MAX_TOKENS || self.tokens.iter().any(|(n, _)| n == name) || name.is_empty() || name.contains([':', ';']) {
            return None;
        }
        let t = hex(&random);
        self.tokens.push((name.to_string(), hex(&sha256(t.as_bytes()))));
        Some(t)
    }

    pub fn revoke(&mut self, name: &str) -> bool {
        let n = self.tokens.len();
        self.tokens.retain(|(x, _)| x != name);
        n != self.tokens.len()
    }

    pub fn token_names(&self) -> Vec<String> {
        self.tokens.iter().map(|(n, _)| n.clone()).collect()
    }

    /// (name, SHA-256 hex) pairs, for storage.
    pub fn stored_tokens(&self) -> &[(String, String)] {
        &self.tokens
    }

    /// Storage form of the tokens: `name:hash;name:hash`.
    pub fn tokens_blob(&self) -> String {
        self.tokens.iter().map(|(n, h)| format!("{n}:{h}")).collect::<Vec<_>>().join(";")
    }

    pub fn load_tokens(&mut self, blob: &str) {
        self.tokens = blob
            .split(';')
            .filter_map(|e| e.split_once(':'))
            .filter(|(n, h)| !n.is_empty() && h.len() == 64)
            .take(MAX_TOKENS)
            .map(|(n, h)| (n.to_string(), h.to_string()))
            .collect();
    }
}
