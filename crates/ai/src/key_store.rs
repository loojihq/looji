//! Keeps the API key in the operating system's credential store: Windows Credential Manager on Windows. It never
//! goes in the settings table, and it isn't written to any file the app owns.

use crate::{error::AiError, key::ApiKey};

const SERVICE: &str = "looji";
const ACCOUNT: &str = "anthropic-api-key";

fn entry() -> Result<keyring::Entry, AiError> {
    keyring::Entry::new(SERVICE, ACCOUNT).map_err(store_error)
}

/// The store's own message. Its failures don't carry the key.
fn store_error(err: keyring::Error) -> AiError {
    AiError::KeyStore(err.to_string())
}

/// The stored key, or `None` when none has been saved.
pub fn load() -> Result<Option<ApiKey>, AiError> {
    match entry()?.get_password() {
        Ok(text) => Ok(ApiKey::new(text)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(store_error(err)),
    }
}

pub fn save(key: &ApiKey) -> Result<(), AiError> {
    entry()?.set_password(key.expose()).map_err(store_error)
}

/// Removes the stored key. Removing one that isn't there is not an error.
pub fn delete() -> Result<(), AiError> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(store_error(err)),
    }
}
