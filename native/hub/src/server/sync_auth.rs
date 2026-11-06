use axum::extract::DefaultBodyLimit;
use axum::{
    Json, Router,
    extract::{ConnectInfo, Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::Response,
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use discovery::access_policy::PeerIdentity;
use discovery::server::{PermissionManager, UserStatus};
use rand::{RngCore, rngs::OsRng};
use rsa::{Pss, RsaPublicKey, pkcs8::DecodePublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, RwLock};
use tower_governor::{
    GovernorLayer, governor::GovernorConfigBuilder, key_extractor::PeerIpKeyExtractor,
};

const CAP: usize = 1024;
const CHALLENGE_TTL: Duration = Duration::from_secs(60);
pub(crate) const TOKEN_TTL: Duration = Duration::from_secs(300);
const DOMAIN: &str = "rune-sync-auth-1";
#[derive(Default)]
struct Entries {
    nonces: HashMap<String, (std::net::IpAddr, Instant)>,
    tokens: HashMap<String, (String, Instant)>,
    #[cfg(test)]
    reject_next_authorization_for: Option<String>,
    #[cfg(test)]
    challenge_count: usize,
    #[cfg(test)]
    authorization_count: usize,
}
#[derive(Clone)]
pub struct SyncAuth {
    entries: Arc<Mutex<Entries>>,
    permissions: Arc<RwLock<PermissionManager>>,
    fingerprint: String,
}
#[derive(Serialize, Deserialize)]
pub struct Challenge {
    pub nonce: String,
    pub server_fingerprint: String,
}
#[derive(Serialize, Deserialize)]
pub struct Authorization {
    pub nonce: String,
    pub fingerprint: String,
    pub signature: String,
}
#[derive(Serialize, Deserialize)]
pub struct Token {
    pub token: String,
}
fn random() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
pub fn message(server: &str, nonce: &str) -> Vec<u8> {
    format!("{DOMAIN}\n{server}\n{nonce}").into_bytes()
}
impl std::fmt::Debug for SyncAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncAuth").finish_non_exhaustive()
    }
}
impl SyncAuth {
    pub fn new(permissions: Arc<RwLock<PermissionManager>>, fingerprint: String) -> Self {
        Self {
            entries: Arc::new(Mutex::new(Entries::default())),
            permissions,
            fingerprint,
        }
    }
    #[cfg(test)]
    pub(super) async fn expire_tokens_for_fingerprint_for_test(&self, fingerprint: &str) -> bool {
        let mut entries = self.entries.lock().await;
        let deadline = Instant::now();
        let mut expired = false;
        for (token_fingerprint, token_deadline) in entries.tokens.values_mut() {
            if token_fingerprint == fingerprint {
                *token_deadline = deadline;
                expired = true;
            }
        }
        expired
    }

    #[cfg(test)]
    pub(super) async fn challenge_count_for_test(&self) -> usize {
        self.entries.lock().await.challenge_count
    }

    #[cfg(test)]
    pub(super) async fn authorization_count_for_test(&self) -> usize {
        self.entries.lock().await.authorization_count
    }
    #[cfg(test)]
    pub(super) async fn reject_next_authorization_for_test(&self, fingerprint: &str) {
        self.entries.lock().await.reject_next_authorization_for = Some(fingerprint.to_owned());
    }
    #[cfg(test)]
    pub(super) async fn authorization_rejection_pending_for_test(&self, fingerprint: &str) -> bool {
        self.entries
            .lock()
            .await
            .reject_next_authorization_for
            .as_deref()
            == Some(fingerprint)
    }
    pub fn router(&self) -> Router {
        let Some(config) = GovernorConfigBuilder::default()
            .per_millisecond(500)
            .burst_size(10)
            .key_extractor(PeerIpKeyExtractor)
            .finish()
        else {
            unreachable!("valid sync rate limit")
        };
        Router::new()
            .route("/sync/challenge", get(challenge))
            .route("/sync/authorize", post(authorize))
            .layer(GovernorLayer {
                config: config.into(),
            })
            .layer(DefaultBodyLimit::max(8192))
            .with_state(self.clone())
    }
    pub async fn resolve_token(&self, token: &str) -> Option<String> {
        let fingerprint = {
            let mut e = self.entries.lock().await;
            e.tokens
                .retain(|_, (_, deadline)| *deadline > Instant::now());
            e.tokens.get(token).map(|(f, _)| f.clone())
        };
        let approved = match &fingerprint {
            Some(f) => self
                .permissions
                .read()
                .await
                .verify_by_fingerprint(f)
                .await
                .is_some_and(|u| u.status == UserStatus::Approved),
            None => false,
        };
        if !approved {
            self.entries.lock().await.tokens.remove(token);
        }
        fingerprint.filter(|_| approved)
    }
}
async fn challenge(
    State(auth): State<SyncAuth>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
) -> Result<Json<Challenge>, StatusCode> {
    let mut e = auth.entries.lock().await;
    #[cfg(test)]
    {
        e.challenge_count += 1;
    }
    e.nonces.retain(|_, (_, t)| *t > Instant::now());
    if e.nonces.len() >= CAP
        || e.nonces
            .values()
            .filter(|(ip, _)| *ip == address.ip())
            .count()
            >= 16
    {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    let nonce = random();
    e.nonces.insert(
        nonce.clone(),
        (address.ip(), Instant::now() + CHALLENGE_TTL),
    );
    Ok(Json(Challenge {
        nonce,
        server_fingerprint: auth.fingerprint,
    }))
}
async fn authorize(
    State(auth): State<SyncAuth>,
    Json(request): Json<Authorization>,
) -> Result<Json<Token>, StatusCode> {
    let valid_nonce = auth
        .entries
        .lock()
        .await
        .nonces
        .remove(&request.nonce)
        .is_some_and(|(_, t)| t > Instant::now());
    if !valid_nonce {
        return Err(StatusCode::UNAUTHORIZED);
    }
    #[cfg(test)]
    {
        let reject_once = {
            let mut entries = auth.entries.lock().await;
            if entries.reject_next_authorization_for.as_deref() == Some(&request.fingerprint) {
                entries.reject_next_authorization_for.take().is_some()
            } else {
                false
            }
        };
        if reject_once {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    let user = auth
        .permissions
        .read()
        .await
        .verify_by_fingerprint(&request.fingerprint)
        .await
        .filter(|u| u.status == UserStatus::Approved)
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let pem = pem::parse(&user.public_key).map_err(|_| StatusCode::UNAUTHORIZED)?;
    let spki = match pem.tag() {
        "PUBLIC KEY" => pem.contents().to_vec(),
        "CERTIFICATE" => {
            let (_, cert) = x509_parser::parse_x509_certificate(pem.contents())
                .map_err(|_| StatusCode::UNAUTHORIZED)?;
            cert.public_key().raw.to_vec()
        }
        _ => return Err(StatusCode::UNAUTHORIZED),
    };
    if discovery::ssl::calculate_base85_fingerprint(&spki).map_err(|_| StatusCode::UNAUTHORIZED)?
        != request.fingerprint
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let key = RsaPublicKey::from_public_key_der(&spki).map_err(|_| StatusCode::UNAUTHORIZED)?;
    let signature = URL_SAFE_NO_PAD
        .decode(&request.signature)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    key.verify(
        Pss::new::<Sha256>(),
        &Sha256::digest(message(&auth.fingerprint, &request.nonce)),
        &signature,
    )
    .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let mut e = auth.entries.lock().await;
    e.tokens.retain(|_, (_, t)| *t > Instant::now());
    let oldest = e
        .tokens
        .iter()
        .filter(|(_, (fingerprint, _))| fingerprint == &request.fingerprint)
        .min_by_key(|(_, (_, deadline))| *deadline)
        .map(|(token, _)| token.clone());
    if e.tokens
        .values()
        .filter(|(fingerprint, _)| fingerprint == &request.fingerprint)
        .count()
        >= 8
        && let Some(oldest) = oldest
    {
        e.tokens.remove(&oldest);
    }
    if e.tokens.len() >= CAP {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    let token = random();
    e.tokens.insert(
        token.clone(),
        (request.fingerprint, Instant::now() + TOKEN_TTL),
    );
    #[cfg(test)]
    {
        e.authorization_count += 1;
    }
    Ok(Json(Token { token }))
}
#[derive(Clone)]
pub struct AuthenticatedFingerprint(pub String);

pub async fn require_auth(
    State(auth): State<SyncAuth>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let fingerprint = auth
        .resolve_token(token)
        .await
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let user = auth
        .permissions
        .read()
        .await
        .verify_by_fingerprint(&fingerprint)
        .await
        .filter(|user| user.status == UserStatus::Approved)
        .ok_or(StatusCode::UNAUTHORIZED)?;
    request.extensions_mut().insert(PeerIdentity {
        fingerprint: fingerprint.clone(),
        public_key: user.public_key,
    });
    request
        .extensions_mut()
        .insert(AuthenticatedFingerprint(fingerprint));
    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use discovery::utils::DeviceType;
    use rsa::{RsaPrivateKey, pkcs8::DecodePrivateKey};
    use tower::ServiceExt;

    #[tokio::test]
    async fn nonce_cap_is_per_ip() {
        let dir = tempfile::tempdir().unwrap();
        let auth = SyncAuth::new(
            Arc::new(RwLock::new(PermissionManager::new(dir.path()).unwrap())),
            "server".into(),
        );
        let a = ConnectInfo("127.0.0.1:1".parse().unwrap());
        for _ in 0..16 {
            assert!(challenge(State(auth.clone()), a).await.is_ok());
        }
        assert!(matches!(
            challenge(State(auth.clone()), a).await,
            Err(StatusCode::TOO_MANY_REQUESTS)
        ));
        assert!(
            challenge(State(auth), ConnectInfo("127.0.0.2:1".parse().unwrap()))
                .await
                .is_ok()
        );
    }

    async fn post(app: &Router, path: &str, value: &impl Serialize) -> (StatusCode, Vec<u8>) {
        let response = app
            .clone()
            .oneshot(
                Request::post(path)
                    .extension(ConnectInfo("127.0.0.1:1234".parse::<SocketAddr>().unwrap()))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(value).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        (
            status,
            to_bytes(response.into_body(), 8192).await.unwrap().to_vec(),
        )
    }
    async fn get_challenge(app: &Router) -> Challenge {
        let response = app
            .clone()
            .oneshot(
                Request::get("/sync/challenge")
                    .extension(ConnectInfo("127.0.0.1:1234".parse::<SocketAddr>().unwrap()))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap()
    }
    #[tokio::test]
    async fn authorization_requires_key_proof_and_tracks_revocation() {
        let dir = std::env::temp_dir().join(format!("rune-auth-{}", random()));
        std::fs::create_dir_all(&dir).unwrap();
        let permissions = Arc::new(RwLock::new(PermissionManager::new(&dir).unwrap()));
        let cert = discovery::ssl::generate_self_signed_cert("test", "rune", "CA", 1).unwrap();
        let fingerprint = cert.public_key_fingerprint.clone();
        let auth = SyncAuth::new(permissions.clone(), "server".into());
        let app = auth.router();
        let sign = |challenge: &Challenge| {
            let private = RsaPrivateKey::from_pkcs8_pem(&cert.private_key).unwrap();
            Authorization {
                nonce: challenge.nonce.clone(),
                fingerprint: fingerprint.clone(),
                signature: URL_SAFE_NO_PAD.encode(
                    private
                        .sign_with_rng(
                            &mut OsRng,
                            Pss::new::<Sha256>(),
                            &Sha256::digest(message("server", &challenge.nonce)),
                        )
                        .unwrap(),
                ),
            }
        };
        assert_eq!(
            post(&app, "/sync/authorize", &sign(&get_challenge(&app).await))
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        permissions
            .write()
            .await
            .add_user(
                cert.certificate.clone(),
                fingerprint.clone(),
                "device".into(),
                "test".into(),
                DeviceType::Desktop,
                "127.0.0.1".into(),
            )
            .await
            .unwrap();
        assert_eq!(
            post(&app, "/sync/authorize", &sign(&get_challenge(&app).await))
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        permissions
            .write()
            .await
            .change_user_status(&fingerprint, UserStatus::Approved)
            .await
            .unwrap();
        let mut issued = Vec::new();
        for _ in 0..20 {
            let challenge = challenge(
                State(auth.clone()),
                ConnectInfo("127.0.0.1:1".parse().unwrap()),
            )
            .await
            .unwrap()
            .0;
            issued.push(
                authorize(State(auth.clone()), Json(sign(&challenge)))
                    .await
                    .unwrap()
                    .0
                    .token,
            );
        }
        let protected = Router::new()
            .route("/protected", get(|| async { StatusCode::OK }))
            .layer(axum::middleware::from_fn_with_state(
                auth.clone(),
                require_auth,
            ));
        for (token, status) in [
            (&issued[0], StatusCode::UNAUTHORIZED),
            (&issued[19], StatusCode::OK),
        ] {
            let response = protected
                .clone()
                .oneshot(
                    Request::get("/protected")
                        .header(header::AUTHORIZATION, format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), status);
        }
        assert_eq!(auth.entries.lock().await.tokens.len(), 8);
        let mut bad = sign(&get_challenge(&app).await);
        bad.signature = URL_SAFE_NO_PAD.encode([0u8; 256]);
        assert_eq!(
            post(&app, "/sync/authorize", &bad).await.0,
            StatusCode::UNAUTHORIZED
        );
        let request = sign(&get_challenge(&app).await);
        let (status, bytes) = post(&app, "/sync/authorize", &request).await;
        assert_eq!(status, StatusCode::OK);
        let token: Token = serde_json::from_slice(&bytes).unwrap();
        assert!(auth.resolve_token(&token.token).await.is_some());
        assert_eq!(
            post(&app, "/sync/authorize", &request).await.0,
            StatusCode::UNAUTHORIZED
        );
        permissions
            .write()
            .await
            .change_user_status(&fingerprint, UserStatus::Blocked)
            .await
            .unwrap();
        assert!(auth.resolve_token(&token.token).await.is_none());
        permissions
            .write()
            .await
            .remove_user(&fingerprint)
            .await
            .unwrap();
        assert!(auth.resolve_token(&token.token).await.is_none());
        permissions
            .write()
            .await
            .add_user(
                cert.public_key.clone(),
                fingerprint.clone(),
                "device".into(),
                "test".into(),
                DeviceType::Desktop,
                "127.0.0.1".into(),
            )
            .await
            .unwrap();
        permissions
            .write()
            .await
            .change_user_status(&fingerprint, UserStatus::Approved)
            .await
            .unwrap();
        assert_eq!(
            post(&app, "/sync/authorize", &sign(&get_challenge(&app).await))
                .await
                .0,
            StatusCode::OK
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_authorization_rejection_is_scoped_to_its_fingerprint() {
        let dir = std::env::temp_dir().join(format!("rune-auth-{}", random()));
        std::fs::create_dir_all(&dir).unwrap();
        let permissions = Arc::new(RwLock::new(PermissionManager::new(&dir).unwrap()));
        let auth = SyncAuth::new(permissions, "server".into());
        let target_fingerprint = "stream-peer";
        auth.reject_next_authorization_for_test(target_fingerprint)
            .await;

        let unrelated_challenge = challenge(
            State(auth.clone()),
            ConnectInfo("127.0.0.1:1".parse().unwrap()),
        )
        .await
        .unwrap()
        .0;
        let unrelated = authorize(
            State(auth.clone()),
            Json(Authorization {
                nonce: unrelated_challenge.nonce,
                fingerprint: "background-worker".into(),
                signature: String::new(),
            }),
        )
        .await;
        assert!(matches!(unrelated, Err(StatusCode::UNAUTHORIZED)));
        assert_eq!(
            auth.entries
                .lock()
                .await
                .reject_next_authorization_for
                .as_deref(),
            Some(target_fingerprint)
        );

        let target_challenge = challenge(
            State(auth.clone()),
            ConnectInfo("127.0.0.1:1".parse().unwrap()),
        )
        .await
        .unwrap()
        .0;
        let target = authorize(
            State(auth.clone()),
            Json(Authorization {
                nonce: target_challenge.nonce,
                fingerprint: target_fingerprint.into(),
                signature: String::new(),
            }),
        )
        .await;
        assert!(matches!(target, Err(StatusCode::UNAUTHORIZED)));
        assert!(
            auth.entries
                .lock()
                .await
                .reject_next_authorization_for
                .is_none()
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
