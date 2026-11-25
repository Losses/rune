//! The bindings this device holds for the libraries of the streaming services that sign in on
//! it, and how they are made, signed in and let go.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use apple_music::{AppleMusicClient, RetryPolicy, TokenSource};
use database::library_registry::{LibraryRecordId, LibraryRegistry, ServiceKind};

use crate::access_policy::RuneAccessPolicy;
use crate::apple_music_binding::{AppleMusicBinding, Tokens, verified_tokens};
use crate::messages::{RegisterLibraryFailureKind, ServiceSignIn};
use crate::service::{ServiceBinding, platform_host, reach::Reach};
use crate::utils::Broadcaster;

/// The time what a service library keeps ages by.
pub(crate) trait Clock: Send + Sync {
    /// The current instant.
    fn now(&self) -> Instant;
}

/// The system's monotonic clock.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// How the service libraries of a node reach their services and tell the time. A node is made
/// with the services' own APIs and the system's clock. A test makes one with a server and a
/// clock of its own.
#[derive(Clone)]
pub(crate) struct ServiceSetup {
    /// The Apple Music API host to read from instead of Apple's. It is asked once per call, as
    /// a server of a test answers at once or not at all.
    pub(crate) apple_music_host: Option<String>,
    /// The clock what the libraries keep ages by.
    pub(crate) clock: Arc<dyn Clock>,
}

impl Default for ServiceSetup {
    fn default() -> Self {
        Self {
            apple_music_host: None,
            clock: Arc::new(SystemClock),
        }
    }
}

impl ServiceSetup {
    /// An Apple Music client that reads with `tokens`.
    pub(crate) fn apple_music_client(&self, tokens: Arc<dyn TokenSource>) -> AppleMusicClient {
        let builder = AppleMusicClient::builder(tokens);
        match &self.apple_music_host {
            Some(host) => builder
                .base_url(host.clone())
                .retry_policy(RetryPolicy {
                    max_attempts: 1,
                    ..RetryPolicy::default()
                })
                .build(),
            None => builder.build(),
        }
    }
}

/// Why a platform sign-in to a service could not be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SignInFailure {
    /// The platform sent no developer token, which the hub has none of its own to replace.
    NoDeveloperToken,
    /// The user token is missing, or the service rejects the tokens.
    Rejected,
    /// The service or the network did not answer in time.
    Unreachable,
    /// The service answered, but not as a sign-in it accepts does.
    Protocol,
}

impl SignInFailure {
    /// How a registration or a sign-in reports this failure, with a detail for the logs.
    pub(crate) fn registration_failure(self) -> (RegisterLibraryFailureKind, &'static str) {
        match self {
            Self::NoDeveloperToken => (
                RegisterLibraryFailureKind::InvalidSource,
                "The platform sent no developer token",
            ),
            Self::Rejected => (
                RegisterLibraryFailureKind::RemoteNotAuthorized,
                "The service did not accept the sign-in",
            ),
            Self::Unreachable => (
                RegisterLibraryFailureKind::RemoteUnreachable,
                "Could not reach the service",
            ),
            Self::Protocol => (
                RegisterLibraryFailureKind::RemoteProtocol,
                "The service returned an unexpected response",
            ),
        }
    }
}

/// A platform sign-in the service accepted, for the library of that service to take.
pub(crate) struct VerifiedSignIn(Tokens);

/// Keeps every library registered in `config_dir` whose service stays on this device from other
/// devices, signed in or not, so nothing served by its id reaches them after a restart either.
pub(crate) fn keep_service_records_on_this_device(
    access_policy: &RuneAccessPolicy,
    config_dir: &Path,
) {
    let registry = match LibraryRegistry::load(config_dir) {
        Ok(registry) => registry,
        Err(error) => {
            log::warn!("Could not read the library registry for its service libraries: {error:#}");
            return;
        }
    };
    let invalid = registry.invalid_records().map(|(record, _)| record);
    for record in registry.libraries.iter().chain(invalid) {
        if record
            .protocol
            .service()
            .is_some_and(ServiceKind::signs_in_on_this_device)
        {
            access_policy.keep_on_this_device(record.record_id.0);
        }
    }
}

/// The service libraries this device holds a binding for, by record: those it signed in to, and
/// those it opened before their sign-in, which answer from what they kept on disk until the
/// platform signs them in. Only the libraries of a service whose sign-in lives in its binding
/// (see `ServiceKind::signs_in_on_this_device`) are held here. Their bindings broadcast to this
/// device's own UI only, never through the node's broadcaster that reaches paired devices: the
/// websocket filter relies on it for content changes, which name no library. A binding stays
/// while other libraries open and close, so its sign-in, which lives in memory only, lasts until
/// its record is removed or the app quits.
pub(crate) struct ServiceRegistry {
    access_policy: Arc<RuneAccessPolicy>,
    config_dir: PathBuf,
    broadcaster: Arc<dyn Broadcaster>,
    setup: ServiceSetup,
    apple_music: Mutex<HashMap<LibraryRecordId, Arc<AppleMusicBinding>>>,
    /// What each platform host reported of this device's network, by host. A binding made later
    /// takes it over.
    reach: Mutex<HashMap<&'static str, Reach>>,
}

impl ServiceRegistry {
    /// `broadcaster` carries what the bindings tell the UI of this device, such as a cover
    /// that arrived.
    pub(crate) fn new(
        access_policy: Arc<RuneAccessPolicy>,
        config_dir: &Path,
        broadcaster: Arc<dyn Broadcaster>,
        setup: ServiceSetup,
    ) -> Self {
        Self {
            access_policy,
            config_dir: config_dir.to_owned(),
            broadcaster,
            setup,
            apple_music: Mutex::default(),
            reach: Mutex::default(),
        }
    }

    fn apple_music(&self) -> MutexGuard<'_, HashMap<LibraryRecordId, Arc<AppleMusicBinding>>> {
        self.apple_music
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The Apple Music binding this device holds for `record_id`, whose own state tests read.
    #[cfg(test)]
    pub(crate) fn apple_music_binding(
        &self,
        record_id: LibraryRecordId,
    ) -> Option<Arc<AppleMusicBinding>> {
        self.apple_music().get(&record_id).cloned()
    }

    /// The binding this device holds for the library of `record_id`, whichever service it
    /// belongs to, signed in or not yet. None when it holds none, as for a library neither
    /// opened nor signed in since the app started.
    pub(crate) fn held(&self, record_id: LibraryRecordId) -> Option<Arc<dyn ServiceBinding>> {
        self.apple_music()
            .get(&record_id)
            .map(|binding| Arc::clone(binding) as Arc<dyn ServiceBinding>)
    }

    /// The binding of the library of `record_id`, a library of `service`, made if this device
    /// holds none yet. A binding made here holds no sign-in: it answers from what it kept on
    /// disk until the platform signs it in, and takes over what the host reported of the
    /// network. No other device may reach the library from then on. None for a service whose
    /// binding does not come with its sign-in, which is made from the library's record instead.
    pub(crate) fn hold(
        &self,
        record_id: LibraryRecordId,
        service: ServiceKind,
    ) -> Option<Arc<dyn ServiceBinding>> {
        match service {
            ServiceKind::AppleMusic => {
                Some(self.hold_apple_music(record_id) as Arc<dyn ServiceBinding>)
            }
            ServiceKind::Subsonic => None,
        }
    }

    fn hold_apple_music(&self, record_id: LibraryRecordId) -> Arc<AppleMusicBinding> {
        self.access_policy.keep_on_this_device(record_id.0);
        let mut made = false;
        let binding = Arc::clone(self.apple_music().entry(record_id).or_insert_with(|| {
            made = true;
            AppleMusicBinding::new(
                record_id.0,
                |tokens| self.setup.apple_music_client(tokens),
                &self.config_dir,
                Arc::clone(&self.broadcaster),
                Arc::clone(&self.setup.clock),
            )
        }));
        if made
            && binding.availability_follows_connectivity()
            && let Some(host) = platform_host(binding.service())
        {
            let reach = self
                .reach
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(host)
                .cloned()
                .unwrap_or_default();
            binding.reach_changed(&reach);
        }
        binding
    }

    /// The tokens of a platform sign-in to `service`, once the service accepted them. The check
    /// runs on a client of its own, so no binding's sign-in changes.
    pub(crate) async fn verify_sign_in(
        &self,
        service: ServiceKind,
        sign_in: &ServiceSignIn,
    ) -> Result<VerifiedSignIn, SignInFailure> {
        let ServiceSignIn::Platform {
            user_token,
            developer_token,
        } = sign_in;
        match service {
            ServiceKind::AppleMusic => verified_tokens(
                |tokens| self.setup.apple_music_client(tokens),
                user_token,
                developer_token.as_deref(),
            )
            .await
            .map(VerifiedSignIn),
            // Subsonic signs in with the credential of its record.
            ServiceKind::Subsonic => Err(SignInFailure::Protocol),
        }
    }

    /// Signs the library of `record_id` in with a sign-in its service accepted, making its
    /// binding if it has none yet. A binding held before takes the sign-in itself, so the ids it
    /// gave keep their meaning. No other device may reach the library from then on. Every
    /// sign-in starts the import of what comes in from the service once, in the background.
    pub(crate) fn sign_in(
        &self,
        record_id: LibraryRecordId,
        sign_in: VerifiedSignIn,
    ) -> Arc<dyn ServiceBinding> {
        let binding = self.hold_apple_music(record_id);
        binding.sign_in(sign_in.0);
        binding.import_in_background();
        binding
    }

    /// Forgets the sign-in of the library of `record_id` and everything it read, on disk too.
    /// The binding stays, as one held before a sign-in does.
    pub(crate) fn sign_out(&self, record_id: LibraryRecordId) {
        let binding = self.apple_music().get(&record_id).cloned();
        if let Some(binding) = binding {
            binding.sign_out();
        }
    }

    /// The platform host `host` reported whether this device is online. Every library of a
    /// service on that host that declares its availability follows connectivity is told.
    /// Answers whether any service is on that host, which no other report concerns.
    pub(crate) fn reach_changed(&self, host: &str, reach: Reach) -> bool {
        let Some(host) = ServiceKind::ALL
            .into_iter()
            .filter_map(platform_host)
            .find(|family| *family == host)
        else {
            return false;
        };
        self.reach
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(host, reach.clone());
        let bindings: Vec<_> = self.apple_music().values().cloned().collect();
        for binding in bindings {
            if platform_host(binding.service()) == Some(host)
                && binding.availability_follows_connectivity()
            {
                binding.reach_changed(&reach);
            }
        }
        true
    }

    /// Drops the binding of a removed record and forgets its sign-in and what it kept, also for
    /// anyone still holding the binding.
    pub(crate) async fn retire(&self, record_id: LibraryRecordId) {
        let binding = self.apple_music().remove(&record_id);
        if let Some(binding) = binding {
            binding.close();
            // The binding may outlive its record in other hands. Neither a change to what it
            // keeps on this device nor a cover still downloading may make its folder again
            // after the removal deletes it.
            binding.close_device_records().await;
            binding.shut_down_covers().await;
            binding.sign_out();
        }
    }
}
