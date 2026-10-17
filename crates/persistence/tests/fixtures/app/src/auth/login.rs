pub struct Session {
    pub user: String,
}

pub fn login(user: &str, password: &str) -> Option<Session> {
    if verify_password(user, password) {
        Some(Session {
            user: user.to_string(),
        })
    } else {
        None
    }
}

fn verify_password(user: &str, password: &str) -> bool {
    !user.is_empty() && !password.is_empty()
}
