pub struct Session {
    pub email: String,
    pub active: bool,
    pub idle_minutes: u32,
}

/// How long a session may sit idle before it stops working.
const IDLE_LIMIT_MINUTES: u32 = 30;

pub fn create_session(email: &str) -> Session {
    Session {
        email: email.to_string(),
        active: true,
        idle_minutes: 0,
    }
}

/// A session that has been idle for too long has timed out and must sign in again.
pub fn has_timed_out(session: &Session) -> bool {
    session.idle_minutes >= IDLE_LIMIT_MINUTES
}

pub fn expire_session(session: &mut Session) {
    session.active = false;
}
