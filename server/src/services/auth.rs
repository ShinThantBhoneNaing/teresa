//! Registration, login, sessions.

use teresa_proto::{validate, Timestamp, User, UserId, UserRole, UserStatus};

use crate::error::{Result, ServerError};
use crate::repo::{SessionRow, UserRow};
use crate::security::token;

use super::{after, Core, Store};

/// The hash a login against a nonexistent account verifies into.
///
/// Without this, "wrong password" costs an Argon2 verification and "no such
/// user" costs nothing, and the time difference tells an attacker which
/// usernames exist. Verifying against this throwaway hash makes both paths pay
/// the same bill. (Hash of a random 32-byte value, minted once, secret long
/// discarded.)
const DECOY_HASH: &str =
    "$argon2id$v=19$m=19456,t=2,p=1$L1dTdEhZbUxWc1FrQnNRZg$Xm5rM2ZlYnJOWnhIVFZoUw";

impl<S: Store> Core<S> {
    /// Creates an account and a first session.
    ///
    /// The first account ever registered becomes the server admin — someone
    /// must be, and the person who set the server up is the only sensible
    /// candidate. (Two racing first registrations could in principle both win
    /// admin; they are both the operator racing themself, which is not a
    /// threat model.)
    ///
    /// New accounts are joined to the first seed room, so a fresh identity
    /// lands in `#general` rather than in an empty void.
    pub async fn register(
        &self,
        username: &str,
        password: &str,
        display_name: &str,
    ) -> Result<(String, User)> {
        if !self.config().open_registration {
            return Err(ServerError::Forbidden(
                "registration is closed on this server; ask the operator for an account",
            ));
        }

        let username = validate::username(username)?;
        validate::password(password)?;
        let display_name = validate::display_name(display_name)?;

        let role = if self.store().count_users().await? == 0 {
            UserRole::Admin
        } else {
            UserRole::Member
        };

        let password_hash = self.hash_password(password.to_owned()).await?;

        let now = Timestamp::now();
        let row = UserRow {
            id: UserId::new(),
            username,
            display_name,
            password_hash,
            status: UserStatus::Online,
            status_text: None,
            role,
            created_at: now,
            last_seen_at: now,
        };
        self.store().insert_user(&row).await?;

        if let Some(home) = self.config().seed_rooms.first() {
            if let Some(room) = self.store().room_by_name(home).await? {
                self.store()
                    .add_member(&crate::repo::MembershipRow {
                        room_id: room.id,
                        user_id: row.id,
                        role: teresa_proto::MemberRole::Member,
                        joined_at: now,
                        last_read_id: String::new(),
                    })
                    .await?;
            }
        }

        tracing::info!(user = %row.username, role = %row.role, "account created");
        let secret = self.mint_session(row.id).await?;
        Ok((secret, row.to_user(self.hub().is_live(row.id))))
    }

    /// Verifies credentials and mints a session.
    pub async fn login(&self, username: &str, password: &str) -> Result<(String, User)> {
        // Validation failures here collapse to the same error a wrong password
        // produces: a login form must not explain which part was wrong.
        let username = validate::username(username).map_err(|_| ServerError::InvalidCredentials)?;

        let row = self.store().user_by_username(&username).await?;

        // Verify unconditionally — see DECOY_HASH for why.
        let stored_hash = row
            .as_ref()
            .map_or_else(|| DECOY_HASH.to_owned(), |r| r.password_hash.clone());
        let verified = self.verify_password(password.to_owned(), stored_hash).await;

        let Some(row) = row.filter(|_| verified) else {
            tracing::debug!(user = %username, "failed login");
            return Err(ServerError::InvalidCredentials);
        };

        self.store()
            .touch_last_seen(row.id, Timestamp::now())
            .await?;
        tracing::info!(user = %row.username, "logged in");

        let secret = self.mint_session(row.id).await?;
        Ok((secret, row.to_user(self.hub().is_live(row.id))))
    }

    /// Resolves a bearer token to its account, sliding the session's expiry.
    ///
    /// Every failure is [`ServerError::SessionExpired`]: an invalid, expired
    /// and revoked token must be indistinguishable to the holder.
    pub async fn authenticate(&self, secret: &str) -> Result<UserRow> {
        let now = Timestamp::now();
        let extend_to = after(now, self.config().session_idle());

        let session = self
            .store()
            .take_live(&token::hash(secret), now, extend_to)
            .await?
            .ok_or(ServerError::SessionExpired)?;

        // An account deleted out from under a live session is also just an
        // expired session as far as the client is concerned.
        self.store()
            .user_by_id(session.user_id)
            .await?
            .ok_or(ServerError::SessionExpired)
    }

    /// Revokes one session — logout on one terminal.
    pub async fn logout(&self, secret: &str) -> Result<()> {
        self.store().delete_session(&token::hash(secret)).await
    }

    /// Revokes every session of an account — logout everywhere.
    pub async fn logout_all(&self, user: UserId) -> Result<u64> {
        self.store().delete_for_user(user).await
    }

    /// Mints and stores a session, returning the secret for the client.
    async fn mint_session(&self, user: UserId) -> Result<String> {
        let minted = token::mint();
        let now = Timestamp::now();
        self.store()
            .insert_session(&SessionRow {
                token_hash: minted.hash,
                user_id: user,
                created_at: now,
                last_used_at: now,
                expires_at: after(now, self.config().session_idle()),
            })
            .await?;
        Ok(minted.secret)
    }

    /// Argon2 on the blocking pool: at ~20ms and 19 MiB per call it has no
    /// business on a runtime worker thread.
    async fn hash_password(&self, password: String) -> Result<String> {
        let hasher = self.hasher.clone();
        tokio::task::spawn_blocking(move || hasher.hash(&password))
            .await
            .map_err(|e| ServerError::Internal(e.into()))?
    }

    /// Verification, likewise off the runtime.
    async fn verify_password(&self, password: String, stored_hash: String) -> bool {
        let hasher = self.hasher.clone();
        tokio::task::spawn_blocking(move || hasher.verify(&password, &stored_hash))
            .await
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::super::testkit;
    use super::*;
    use crate::config::ServerConfig;
    use crate::repo::sqlite::SqliteStore;
    use crate::repo::RoomRepo;

    #[tokio::test]
    async fn register_then_login_round_trips() {
        let core = testkit::core().await;

        let (token_a, user) = core
            .register("Neo", "follow the white rabbit", "Neo")
            .await
            .unwrap();
        assert_eq!(user.username, "neo", "usernames fold to lowercase");
        assert_eq!(user.role, UserRole::Admin, "first account is the admin");

        let (token_b, again) = core.login("neo", "follow the white rabbit").await.unwrap();
        assert_eq!(again.id, user.id);
        assert_ne!(token_a, token_b, "every login mints a fresh session");

        // Both sessions resolve to the account.
        assert_eq!(core.authenticate(&token_a).await.unwrap().id, user.id);
        assert_eq!(core.authenticate(&token_b).await.unwrap().id, user.id);
    }

    #[tokio::test]
    async fn later_accounts_are_plain_members() {
        let core = testkit::core().await;
        testkit::register(&core, "first").await;
        let (_, second) = testkit::register(&core, "second").await;
        assert_eq!(second.role, UserRole::Member);
    }

    #[tokio::test]
    async fn new_accounts_land_in_the_first_seed_room() {
        let core = testkit::core().await;
        let (_, user) = testkit::register(&core, "neo").await;

        let joined = core.store().joined_by(user.id).await.unwrap();
        assert_eq!(joined.len(), 1);
        assert_eq!(joined[0].room.name, "general");
    }

    #[tokio::test]
    async fn wrong_password_and_unknown_user_fail_identically() {
        let core = testkit::core().await;
        testkit::register(&core, "neo").await;

        let wrong_pw = core.login("neo", "not the password").await.unwrap_err();
        let no_user = core.login("smith", "not the password").await.unwrap_err();

        assert!(matches!(wrong_pw, ServerError::InvalidCredentials));
        assert!(matches!(no_user, ServerError::InvalidCredentials));
        // And so does a syntactically invalid username.
        let invalid = core.login("not a name!", "whatever pw").await.unwrap_err();
        assert!(matches!(invalid, ServerError::InvalidCredentials));
    }

    #[tokio::test]
    async fn duplicate_usernames_are_refused_case_insensitively() {
        let core = testkit::core().await;
        testkit::register(&core, "neo").await;
        let err = core
            .register("NEO", "another fine password", "Impostor")
            .await
            .unwrap_err();
        assert!(matches!(err, ServerError::Conflict(_)));
    }

    #[tokio::test]
    async fn closed_registration_refuses_new_accounts() {
        let store = SqliteStore::memory().await.unwrap();
        let core = Core::new(
            store,
            ServerConfig {
                open_registration: false,
                ..ServerConfig::default()
            },
        );
        core.bootstrap().await.unwrap();

        let err = core
            .register("neo", "a fine password here", "Neo")
            .await
            .unwrap_err();
        assert!(matches!(err, ServerError::Forbidden(_)));
    }

    #[tokio::test]
    async fn short_passwords_are_rejected_at_registration() {
        let core = testkit::core().await;
        let err = core.register("neo", "short", "Neo").await.unwrap_err();
        assert!(matches!(err, ServerError::Invalid(_)));
    }

    #[tokio::test]
    async fn forged_and_revoked_tokens_read_as_expired() {
        let core = testkit::core().await;
        let (token, _) = testkit::register(&core, "neo").await;

        let forged = core.authenticate("forged-token-value").await.unwrap_err();
        assert!(matches!(forged, ServerError::SessionExpired));

        core.logout(&token).await.unwrap();
        let revoked = core.authenticate(&token).await.unwrap_err();
        assert!(matches!(revoked, ServerError::SessionExpired));
    }

    #[tokio::test]
    async fn logout_all_revokes_every_terminal() {
        let core = testkit::core().await;
        let (first, user) = testkit::register(&core, "neo").await;
        let (second, _) = core
            .login("neo", "a sufficiently long password")
            .await
            .unwrap();

        assert_eq!(core.logout_all(user.id).await.unwrap(), 2);
        assert!(core.authenticate(&first).await.is_err());
        assert!(core.authenticate(&second).await.is_err());
    }

    #[tokio::test]
    async fn the_decoy_hash_is_a_parseable_argon2_string() {
        // If this regresses, the "unknown user" path stops paying the Argon2
        // bill and the timing side channel returns.
        let hasher = crate::security::PasswordHasherService::new();
        // Verifying against it must return false (nothing hashes to a decoy),
        // but parse successfully — a parse failure would short-circuit early.
        assert!(!hasher.verify("any password at all", DECOY_HASH));
        assert!(argon2::password_hash::PasswordHash::new(DECOY_HASH).is_ok());
    }
}
