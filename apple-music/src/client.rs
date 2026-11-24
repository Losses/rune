//! The HTTP client: authentication, retries and concurrency limits.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use reqwest::header::{
    AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, HeaderValue, RETRY_AFTER,
};
use reqwest::{Method, StatusCode, redirect};
use serde::de::DeserializeOwned;
use tokio::sync::Semaphore;
use tokio::time;

use crate::error::{ApiError, Error, Result, is_client_not_registered, parse_errors};
use crate::tokens::TokenSource;

/// The production API host.
pub const DEFAULT_BASE_URL: &str = "https://api.music.apple.com";

/// A request that took longer than this in total is logged.
const SLOW_REQUEST: Duration = Duration::from_secs(3);

/// What a request went through, for the log of a slow one.
#[derive(Default)]
struct Attempts {
    /// Requests sent, the first one included.
    sent: u32,
    /// Time spent waiting before a retry.
    waited: Duration,
}

const USER_TOKEN_HEADER: &str = "Music-User-Token";

/// How failed requests are retried (429, 5xx and transport errors).
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Total number of attempts, including the first one.
    pub max_attempts: u32,
    /// Delay before the first retry; doubles each time.
    pub base_delay: Duration,
    /// Upper bound of the exponential delay.
    pub max_delay: Duration,
    /// Upper bound applied to a server supplied `Retry-After`.
    pub max_retry_after: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            max_retry_after: Duration::from_secs(120),
        }
    }
}

impl RetryPolicy {
    /// Exponential delay with jitter before retry number `retry` (1-based).
    fn backoff(&self, retry: u32) -> Duration {
        let exp = self.base_delay.saturating_mul(
            1u32.checked_shl(retry.saturating_sub(1))
                .unwrap_or(u32::MAX),
        );
        let capped = exp.min(self.max_delay);
        // Jitter in [0.5, 1.0] of the capped delay.
        capped.mul_f64(0.5 + rand::random::<f64>() * 0.5)
    }
}

/// Which tokens the next request asks the token source to refresh, and which refreshes
/// were tried already.
#[derive(Default)]
struct Refresh {
    user: bool,
    developer: bool,
    user_spent: bool,
    developer_spent: bool,
    /// Whether the token a refresh handed back unchanged was sent once more already.
    same_token_retried: bool,
}

impl Refresh {
    /// Picks the next token to refresh after a 401 or 403, or gives up with that error.
    fn escalate(&mut self, status: StatusCode, auth: Auth) -> Result<()> {
        if auth == Auth::User && !self.user_spent {
            // Apple answers 403 for an expired or invalid user token.
            self.user_spent = true;
            self.user = true;
        } else if status == StatusCode::UNAUTHORIZED && !self.developer_spent {
            self.developer_spent = true;
            self.developer = true;
        } else if status == StatusCode::UNAUTHORIZED {
            return Err(Error::Unauthorized);
        } else {
            return Err(Error::Forbidden);
        }
        Ok(())
    }
}

/// Which tokens a request carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Auth {
    /// Developer token only (catalog).
    Developer,
    /// Developer token and Music User Token (`/v1/me/*`).
    User,
}

struct Inner {
    base_url: String,
    http: reqwest::Client,
    tokens: Arc<dyn TokenSource>,
    configured_storefront: Option<String>,
    storefront: Mutex<StorefrontCache>,
    permits: Semaphore,
    retry: RetryPolicy,
}

/// The storefront in use. `epoch` counts [`AppleMusicClient::forget_storefront`] calls, so
/// a lookup that started before one is not kept.
struct StorefrontCache {
    epoch: u64,
    id: Option<String>,
}

/// A client for the Apple Music API. Cloning is cheap and shares all state.
#[derive(Clone)]
pub struct AppleMusicClient {
    inner: Arc<Inner>,
}

/// Builds an [`AppleMusicClient`].
pub struct ClientBuilder {
    tokens: Arc<dyn TokenSource>,
    base_url: String,
    storefront: Option<String>,
    max_concurrent: usize,
    retry: RetryPolicy,
    http: Option<reqwest::Client>,
}

impl ClientBuilder {
    /// Overrides the API host, for example to point at a test server.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Sets the storefront (lowercase country code) used by catalog calls.
    /// When unset it is looked up with `/v1/me/storefront` on first use.
    pub fn storefront(mut self, storefront: impl Into<String>) -> Self {
        self.storefront = Some(storefront.into());
        self
    }

    /// Caps the number of requests in flight. The default is 4.
    pub fn max_concurrent_requests(mut self, n: usize) -> Self {
        self.max_concurrent = n.max(1);
        self
    }

    /// Replaces the retry policy.
    pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry = policy;
        self
    }

    /// Uses an existing `reqwest` client (proxy settings, timeouts, ...).
    pub fn http_client(mut self, client: reqwest::Client) -> Self {
        self.http = Some(client);
        self
    }

    /// Builds the client.
    pub fn build(self) -> AppleMusicClient {
        let http = self.http.unwrap_or_else(|| {
            // Redirects are not followed: reqwest would send the Music User Token on to any
            // host a redirect names.
            reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(redirect::Policy::none())
                .build()
                .expect("an HTTP client with a timeout and no redirects always builds")
        });
        AppleMusicClient {
            inner: Arc::new(Inner {
                base_url: self.base_url.trim_end_matches('/').to_owned(),
                http,
                tokens: self.tokens,
                configured_storefront: self.storefront.clone(),
                storefront: Mutex::new(StorefrontCache {
                    epoch: 0,
                    id: self.storefront,
                }),
                permits: Semaphore::new(self.max_concurrent),
                retry: self.retry,
            }),
        }
    }
}

impl AppleMusicClient {
    /// Creates a client for the production API with default settings.
    pub fn new(tokens: Arc<dyn TokenSource>) -> Self {
        Self::builder(tokens).build()
    }

    /// Starts building a client.
    pub fn builder(tokens: Arc<dyn TokenSource>) -> ClientBuilder {
        ClientBuilder {
            tokens,
            base_url: DEFAULT_BASE_URL.to_owned(),
            storefront: None,
            max_concurrent: 4,
            retry: RetryPolicy::default(),
            http: None,
        }
    }

    /// The storefront in use, if known, and the epoch to hand back to [`Self::set_storefront`].
    pub(crate) fn cached_storefront(&self) -> (u64, Option<String>) {
        let cache = self
            .inner
            .storefront
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        (cache.epoch, cache.id.clone())
    }

    /// Keeps a looked-up storefront, unless the storefront was forgotten since `epoch`.
    pub(crate) fn set_storefront(&self, epoch: u64, storefront: String) {
        let mut cache = self
            .inner
            .storefront
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if cache.epoch == epoch {
            cache.id = Some(storefront);
        }
    }

    /// Drops the storefront looked up for the last user, for example after another
    /// user signed in, including a lookup still in flight. A storefront set on the
    /// builder stays.
    pub fn forget_storefront(&self) {
        let mut cache = self
            .inner
            .storefront
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        cache.epoch += 1;
        cache.id.clone_from(&self.inner.configured_storefront);
    }

    /// Sends a request and returns the success body.
    ///
    /// `target` is the path with its query string, appended to the base URL.
    pub(crate) async fn execute(
        &self,
        method: Method,
        target: &str,
        body: Option<&serde_json::Value>,
        auth: Auth,
    ) -> Result<Vec<u8>> {
        // A `next` link comes from the server: only a path of the API is followed, so it
        // cannot turn the base URL into another host.
        if !target.starts_with("/v1/") {
            return Err(Error::Api {
                status: 0,
                errors: vec![ApiError {
                    title: Some("a link that does not lead to the Apple Music API".into()),
                    ..ApiError::default()
                }],
            });
        }
        let started = Instant::now();
        let mut attempts = Attempts::default();
        let result = self
            .execute_counted(method.clone(), target, body, auth, &mut attempts)
            .await;
        let total = started.elapsed();
        if total > SLOW_REQUEST {
            // Only the path is logged: the query may carry ids or tokens.
            let path = target.split('?').next().unwrap_or_default();
            log::warn!(
                "Slow Apple Music request: {method} {path} took {total:.1?} over {} attempt(s), \
                 {:.1?} of it waiting to retry",
                attempts.sent,
                attempts.waited,
            );
        }
        result
    }

    async fn execute_counted(
        &self,
        method: Method,
        target: &str,
        body: Option<&serde_json::Value>,
        auth: Auth,
        attempts: &mut Attempts,
    ) -> Result<Vec<u8>> {
        let inner = &*self.inner;
        let url = format!("{}{}", inner.base_url, target);
        let policy = &inner.retry;

        let mut failures = 0u32;
        let mut refresh = Refresh::default();
        let mut rejection = StatusCode::UNAUTHORIZED;
        // The tokens of the last request sent, to tell a refresh that brought new ones.
        let mut sent_developer: Option<String> = None;
        let mut sent_user: Option<String> = None;

        loop {
            let developer = inner.tokens.developer_token(refresh.developer).await?;
            let user = if auth == Auth::User {
                Some(
                    inner
                        .tokens
                        .user_token(refresh.user)
                        .await?
                        .ok_or(Error::MissingUserToken)?,
                )
            } else {
                None
            };
            // A source that cannot refresh hands back the token just rejected. It is sent
            // once more after a short wait, which absorbs a one-off rejection; a second
            // rejection goes on to the next refresh, or ends the call.
            let unchanged = (refresh.developer && sent_developer.as_deref() == Some(&developer))
                || (refresh.user && user.is_some() && user == sent_user);
            refresh.developer = false;
            refresh.user = false;
            if unchanged {
                if refresh.same_token_retried {
                    refresh.escalate(rejection, auth)?;
                    continue;
                }
                refresh.same_token_retried = true;
                attempts.waited += policy.base_delay;
                time::sleep(policy.base_delay).await;
            }
            sent_developer = Some(developer.clone());
            sent_user.clone_from(&user);

            let mut request = inner.http.request(method.clone(), &url).header(
                AUTHORIZATION,
                secret_header(&format!("Bearer {developer}"))?,
            );
            if let Some(user) = &user {
                request = request.header(USER_TOKEN_HEADER, secret_header(user)?);
            }
            if let Some(body) = body {
                request = request
                    .header(CONTENT_TYPE, "application/json")
                    .body(serde_json::to_vec(body)?);
            } else if method == Method::POST {
                // Apple answers 411 when a bodyless POST doesn't state its length.
                request = request.header(CONTENT_LENGTH, 0);
            }

            let outcome = {
                let _permit = inner.permits.acquire().await.ok();
                match request.send().await {
                    Ok(response) => {
                        let status = response.status();
                        let headers = response.headers().clone();
                        response
                            .bytes()
                            .await
                            .map(|bytes| (status, headers, bytes.to_vec()))
                    }
                    Err(e) => Err(e),
                }
            };

            attempts.sent += 1;
            let (status, headers, bytes) = match outcome {
                Ok(parts) => parts,
                Err(e) => {
                    failures += 1;
                    // A POST may have been applied even though the answer was lost.
                    if method == Method::POST || failures >= policy.max_attempts {
                        return Err(Error::Http(e.without_url()));
                    }
                    let delay = policy.backoff(failures);
                    attempts.waited += delay;
                    time::sleep(delay).await;
                    continue;
                }
            };

            if status.is_success() {
                return Ok(bytes);
            }

            match status {
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                    rejection = status;
                    refresh.escalate(status, auth)?;
                }
                StatusCode::NOT_FOUND => {
                    let errors = parse_errors(&bytes);
                    if is_client_not_registered(&errors) {
                        return Err(Error::ClientNotRegistered);
                    }
                    let message = errors
                        .first()
                        .and_then(|e| e.detail.clone().or_else(|| e.title.clone()))
                        .unwrap_or_else(|| target.to_owned());
                    return Err(Error::NotFound(message));
                }
                s if s == StatusCode::TOO_MANY_REQUESTS || s.is_server_error() => {
                    let retry_after = parse_retry_after(&headers);
                    failures += 1;
                    // Only a 429 means a POST was not processed; a 5xx may have been applied.
                    let post_not_retryable =
                        method == Method::POST && s != StatusCode::TOO_MANY_REQUESTS;
                    // A wait longer than the policy allows is left to the caller.
                    let too_long = retry_after.is_some_and(|d| d > policy.max_retry_after);
                    if post_not_retryable || too_long || failures >= policy.max_attempts {
                        return Err(if s == StatusCode::TOO_MANY_REQUESTS {
                            Error::RateLimited { retry_after }
                        } else {
                            Error::Server {
                                status: s.as_u16(),
                                errors: parse_errors(&bytes),
                            }
                        });
                    }
                    let delay = match retry_after {
                        // Never below the base delay, so `Retry-After: 0` cannot spin.
                        Some(d) => d.max(policy.base_delay),
                        None => policy.backoff(failures),
                    };
                    attempts.waited += delay;
                    time::sleep(delay).await;
                }
                s => {
                    return Err(Error::Api {
                        status: s.as_u16(),
                        errors: parse_errors(&bytes),
                    });
                }
            }
        }
    }

    /// Sends a request and decodes the JSON body.
    pub(crate) async fn execute_json<T: DeserializeOwned>(
        &self,
        method: Method,
        target: &str,
        body: Option<&serde_json::Value>,
        auth: Auth,
    ) -> Result<T> {
        let bytes = self.execute(method, target, body, auth).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

/// A header value that HTTP tooling must not print.
fn secret_header(value: &str) -> Result<HeaderValue> {
    let mut header = HeaderValue::from_str(value)
        .map_err(|_| Error::Token("a token is not a valid header value".into()))?;
    header.set_sensitive(true);
    Ok(header)
}

fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    let seconds: u64 = headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_headers_are_marked_sensitive() {
        assert!(secret_header("Bearer token").unwrap().is_sensitive());
        assert!(matches!(secret_header("bad\nvalue"), Err(Error::Token(_))));
    }
}
