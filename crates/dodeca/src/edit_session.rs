//! In-browser editor sessions.
//!
//! The editor's websocket (vox RPC over `/_/ws`) is a *different* connection
//! than the HTTP request that carries the oauth2-proxy identity headers. To
//! bridge them without trusting browser-asserted identity, the host **mints an
//! opaque random token** at the identity-bearing `GET /_dodeca/edit/<page>`
//! load — but only for verified editors. The token is kept server-side, mapped
//! to the verified [`Identity`]; the browser presents it as an ordinary RPC
//! argument on `edit_*` calls. Possessing a live token is itself the proof of
//! edit rights: it's 256 bits of randomness, unguessable and unforgeable.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use cell_http_proto::Identity;

/// How long an editing session token stays valid after minting.
const TOKEN_TTL: Duration = Duration::from_secs(60 * 60);

struct Session {
    identity: Identity,
    expires: Instant,
}

/// Server-side store of live editing sessions, keyed by opaque token.
#[derive(Default)]
pub struct EditSessionStore {
    sessions: RwLock<HashMap<String, Session>>,
}

impl EditSessionStore {
    /// Mint a token for an already-verified editor. Caller MUST have checked
    /// [`crate::authz::is_editor`] first — this store does not re-check.
    pub fn mint(&self, identity: Identity) -> String {
        self.mint_at(identity, Instant::now())
    }

    fn mint_at(&self, identity: Identity, now: Instant) -> String {
        let token = random_token();
        let session = Session {
            identity,
            expires: now + TOKEN_TTL,
        };
        self.sessions
            .write()
            .expect("edit sessions poisoned")
            .insert(token.clone(), session);
        token
    }

    /// Resolve a token to its verified identity, or `None` if unknown/expired.
    pub fn resolve(&self, token: &str) -> Option<Identity> {
        self.resolve_at(token, Instant::now())
    }

    fn resolve_at(&self, token: &str, now: Instant) -> Option<Identity> {
        let sessions = self.sessions.read().expect("edit sessions poisoned");
        let session = sessions.get(token)?;
        if session.expires <= now {
            return None;
        }
        Some(session.identity.clone())
    }
}

/// 32 bytes of OS randomness, hex-encoded (64 chars).
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("getrandom failed");
    let mut out = String::with_capacity(64);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(user: &str) -> Identity {
        Identity {
            user: user.to_string(),
            email: format!("{user}@example.com"),
            name: user.to_string(),
            groups: vec![],
        }
    }

    #[test]
    fn mint_then_resolve_roundtrips_identity() {
        let store = EditSessionStore::default();
        let token = store.mint(identity("amos"));
        assert_eq!(store.resolve(&token).unwrap().user, "amos");
    }

    #[test]
    fn unknown_token_does_not_resolve() {
        let store = EditSessionStore::default();
        store.mint(identity("amos"));
        assert!(store.resolve("deadbeef").is_none());
    }

    #[test]
    fn expired_token_does_not_resolve() {
        let store = EditSessionStore::default();
        let t0 = Instant::now();
        let token = store.mint_at(identity("amos"), t0);
        // Just inside the window: still valid.
        assert!(store.resolve_at(&token, t0 + TOKEN_TTL / 2).is_some());
        // Past the window: gone.
        assert!(
            store
                .resolve_at(&token, t0 + TOKEN_TTL + Duration::from_secs(1))
                .is_none()
        );
    }

    #[test]
    fn tokens_are_distinct_and_hex() {
        let store = EditSessionStore::default();
        let a = store.mint(identity("amos"));
        let b = store.mint(identity("amos"));
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
