//! The two REST conversations a client ever has: getting a session, and
//! giving one back.

use serde::Deserialize;
use teresa_proto::{ErrorCode, User};

use crate::config::ClientConfig;

/// What the server said when it refused us, decoded from the error body.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// The server answered with a protocol error.
    #[error("{message}")]
    Refused {
        /// Machine-readable reason.
        code: ErrorCode,
        /// Human text, already safe to display.
        message: String,
    },
    /// The server could not be reached at all.
    #[error("cannot reach {server}: {source}")]
    Unreachable {
        /// Where we tried to go.
        server: String,
        /// The transport-level failure.
        #[source]
        source: reqwest::Error,
    },
    /// The server answered something that is not the protocol.
    #[error("unexpected response from server: {0}")]
    Malformed(String),
}

/// A successful register or login.
#[derive(Debug, Deserialize)]
pub struct AuthOk {
    /// The bearer token to store.
    pub token: String,
    /// The profile the server holds for us.
    pub user: User,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    code: ErrorCode,
    message: String,
}

/// The auth client. Cheap to construct; holds only the base URL.
#[derive(Debug, Clone)]
pub struct AuthClient {
    base: String,
    server_label: String,
    http: reqwest::Client,
}

impl AuthClient {
    /// Builds a client for the configured server.
    ///
    /// # Panics
    /// Never in practice: the reqwest builder only fails over TLS backend
    /// misconfiguration, and the backend is compiled in.
    #[must_use]
    pub fn new(config: &ClientConfig) -> Self {
        Self {
            base: config.http_base(),
            server_label: format!("{}:{}", config.server, config.port),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("reqwest client builds"),
        }
    }

    /// `POST /auth/register`
    ///
    /// # Errors
    /// [`AuthError`] with the server's refusal, or the transport failure.
    pub async fn register(
        &self,
        username: &str,
        password: &str,
        display_name: &str,
    ) -> Result<AuthOk, AuthError> {
        self.post_auth(
            "register",
            &serde_json::json!({
                "username": username,
                "password": password,
                "display_name": display_name,
            }),
        )
        .await
    }

    /// `POST /auth/login`
    ///
    /// # Errors
    /// [`AuthError`] with the server's refusal, or the transport failure.
    pub async fn login(&self, username: &str, password: &str) -> Result<AuthOk, AuthError> {
        self.post_auth(
            "login",
            &serde_json::json!({ "username": username, "password": password }),
        )
        .await
    }

    /// `GET /healthz` — proves the node is alive before asking a human for
    /// credentials. Returns the server's version string.
    ///
    /// # Errors
    /// [`AuthError::Unreachable`] when nothing answers, [`AuthError::Malformed`]
    /// when whatever answers is not a TERESA server.
    pub async fn healthz(&self) -> Result<String, AuthError> {
        let response = self
            .http
            .get(format!("{}/healthz", self.base))
            .send()
            .await
            .map_err(|source| AuthError::Unreachable {
                server: self.server_label.clone(),
                source,
            })?;
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AuthError::Malformed(e.to_string()))?;
        body["version"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| AuthError::Malformed("healthz has no version field".to_owned()))
    }

    /// `POST /auth/logout` — best effort; the local session is deleted either
    /// way, and a server that is down cannot keep us logged in.
    pub async fn logout(&self, token: &str) {
        let _ = self
            .http
            .post(format!("{}/auth/logout", self.base))
            .header("authorization", format!("Bearer {token}"))
            .send()
            .await;
    }

    async fn post_auth(
        &self,
        endpoint: &str,
        body: &serde_json::Value,
    ) -> Result<AuthOk, AuthError> {
        let response = self
            .http
            .post(format!("{}/auth/{endpoint}", self.base))
            .json(body)
            .send()
            .await
            .map_err(|source| AuthError::Unreachable {
                server: self.server_label.clone(),
                source,
            })?;

        if response.status().is_success() {
            response
                .json::<AuthOk>()
                .await
                .map_err(|e| AuthError::Malformed(e.to_string()))
        } else {
            let refusal = response
                .json::<ErrorBody>()
                .await
                .map_err(|e| AuthError::Malformed(e.to_string()))?;
            Err(AuthError::Refused {
                code: refusal.code,
                message: refusal.message,
            })
        }
    }
}
