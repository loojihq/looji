use crate::auth::session::{create_session, Session};

/// Checks a password against the stored hash. Empty passwords are never accepted.
pub fn verify_password(input: &str, stored_hash: &str) -> bool {
    if input.is_empty() {
        return false;
    }
    hash_password(input) == stored_hash
}

fn hash_password(input: &str) -> String {
    format!("hashed:{input}")
}

/// Signs a customer in when their password is right, and starts a session for them.
pub fn sign_in(email: &str, password: &str, stored_hash: &str) -> Option<Session> {
    if verify_password(password, stored_hash) {
        Some(create_session(email))
    } else {
        None
    }
}

/// Ends the customer's session straight away.
pub fn sign_out(session: &mut Session) {
    session.active = false;
}
