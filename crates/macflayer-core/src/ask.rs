//! Asks aren't events. A front end implements [`Asks`], and a password travels only through this
//! typed call, so a secret has no path into the event stream or a log.

use zeroize::Zeroizing;

/// The `sudo` password. It zeroes itself when dropped, can't be cloned or serialized, and hides
/// itself from `Debug`.
///
/// ```compile_fail
/// fn needs_serialize<T: serde::Serialize>() {}
/// needs_serialize::<macflayer_core::ask::Password>();
/// ```
///
/// ```compile_fail
/// fn needs_clone<T: Clone>() {}
/// needs_clone::<macflayer_core::ask::Password>();
/// ```
pub struct Password(Zeroizing<String>);

impl Password {
    pub fn new(text: String) -> Self {
        Self(Zeroizing::new(text))
    }

    /// Only the Admin Stage's helper socket should read this.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Password(hidden)")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasswordRequest {
    /// What needs it, in words for the person.
    pub why: String,
    /// The last answer was refused, so ask again without starting the Run.
    pub last_was_wrong: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignInStatus {
    Waiting,
    SignedIn,
    Failed,
}

/// What the engine needs from whoever is sitting at the Machine, implemented by the app and the CLI.
pub trait Asks: Send + Sync {
    /// `None` means the person declined.
    fn password(&self, request: &PasswordRequest) -> Option<Password>;
    /// Shows GitHub's device code and where to enter it.
    fn show_github_code(&self, code: &str, url: &str);
    fn github_status(&self, status: SignInStatus);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_never_shows_up_in_debug_output() {
        let password = Password::new("hunter2".to_string());
        assert!(!format!("{password:?}").contains("hunter2"));
        assert_eq!(password.expose(), "hunter2");
    }
}
