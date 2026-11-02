use axum::{
    Json, Router,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::Response,
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use discovery::server::{PermissionManager, UserStatus};
use rand::{RngCore, rngs::OsRng};
use rsa::{Pss, RsaPublicKey, pkcs8::DecodePublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, RwLock};

const CAP: usize = 1024;
const CHALLENGE_TTL: Duration = Duration::from_secs(60);
const TOKEN_TTL: Duration = Duration::from_secs(300);
const DOMAIN: &str = "rune-sync-auth-1";
#[derive(Default)]
struct Entries {
    nonces: HashMap<String, Instant>,
    tokens: HashMap<String, (String, Instant)>,
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
#[cfg(test)]
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
    pub(super) async fn expire_tokens_for_test(&self) {
        let mut entries = self.entries.lock().await;
        for (_, deadline) in entries.tokens.values_mut() {
            *deadline = Instant::now();
        }
    }
    pub fn router(&self) -> Router {
        Router::new()
            .route("/sync/challenge", get(challenge))
            .route("/sync/authorize", post(authorize))
            .layer(axum::extract::DefaultBodyLimit::max(8192))
            .with_state(self.clone())
    }
    pub async fn allowed(&self, token: &str) -> bool {
        let fingerprint = {
            let mut e = self.entries.lock().await;
            e.tokens
                .retain(|_, (_, deadline)| *deadline > Instant::now());
            e.tokens.get(token).map(|(f, _)| f.clone())
        };
        let approved = match fingerprint {
            Some(f) => self
                .permissions
                .read()
                .await
                .verify_by_fingerprint(&f)
                .await
                .is_some_and(|u| u.status == UserStatus::Approved),
            None => false,
        };
        if !approved {
            self.entries.lock().await.tokens.remove(token);
        }
        approved
    }
}
async fn challenge(State(auth): State<SyncAuth>) -> Result<Json<Challenge>, StatusCode> {
    let mut e = auth.entries.lock().await;
    e.nonces.retain(|_, t| *t > Instant::now());
    if e.nonces.len() >= CAP {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    let nonce = random();
    e.nonces
        .insert(nonce.clone(), Instant::now() + CHALLENGE_TTL);
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
        .is_some_and(|t| t > Instant::now());
    if !valid_nonce {
        return Err(StatusCode::UNAUTHORIZED);
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
    if e.tokens.len() >= CAP {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    let token = random();
    e.tokens.insert(
        token.clone(),
        (request.fingerprint, Instant::now() + TOKEN_TTL),
    );
    Ok(Json(Token { token }))
}
pub async fn require_auth(
    State(auth): State<SyncAuth>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if !auth.allowed(token).await {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use rsa::{RsaPrivateKey, pkcs8::DecodePrivateKey};
    use tower::ServiceExt;

    async fn post(app: &Router, path: &str, value: &impl Serialize) -> (StatusCode, Vec<u8>) {
        let response = app
            .clone()
            .oneshot(
                Request::post(path)
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
            .oneshot(Request::get("/sync/challenge").body(Body::empty()).unwrap())
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
                discovery::utils::DeviceType::Desktop,
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
        assert!(auth.allowed(&token.token).await);
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
        assert!(!auth.allowed(&token.token).await);
        permissions
            .write()
            .await
            .remove_user(&fingerprint)
            .await
            .unwrap();
        assert!(!auth.allowed(&token.token).await);
        permissions
            .write()
            .await
            .add_user(
                cert.public_key.clone(),
                fingerprint.clone(),
                "device".into(),
                "test".into(),
                discovery::utils::DeviceType::Desktop,
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
}
