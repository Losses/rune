use std::collections::{HashMap, VecDeque};
use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::path::Path;
use std::sync::{Arc, PoisonError, RwLock as StdRwLock};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{RwLock, broadcast, broadcast::error::RecvError, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::persistent::{PersistenceError, PersistentDataManager};
use crate::utils::DeviceType;
use std::collections;

/// Defines the status of a user within the permission system.
///
/// `UserStatus` is an enum that represents the current state of a user's permission.
/// It can be `Approved`, `Pending` for initial approval, or `Blocked` from accessing resources.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum UserStatus {
    /// User has been approved and has access.
    Approved,
    /// User is pending approval and may not have full access.
    Pending,
    /// User has been blocked and does not have access.
    Blocked,
}

// Helper function to get current time, used as default for add_time during deserialization
fn get_current_time() -> SystemTime {
    SystemTime::now()
}

/// Represents a user with detailed information for permission management.
///
/// `User` struct holds all necessary information about a user, including their public key,
/// unique fingerprint, alias, device model, device type, current permission status and adding time.
/// It is used for internal representation and persistent storage of user data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// Public key of the user, used for identification and potentially authentication.
    pub public_key: String,
    /// Unique fingerprint identifying the user, often derived from the public key.
    pub fingerprint: String,
    /// Human-readable alias or name for the user.
    pub alias: String,
    /// Device model associated with the user, providing context about their device.
    pub device_model: String,
    /// Type of device associated with the user, categorizing the device (e.g., Light, Sensor).
    device_type: DeviceType,
    /// Current status of the user in the permission system (`Approved`, `Pending`, `Blocked`).
    pub status: UserStatus,
    /// The timestamp of the time when user added.
    #[serde(default = "get_current_time")]
    pub add_time: SystemTime,
}

/// Provides a summary view of a user, omitting sensitive details.
///
/// `UserSummary` contains a subset of `User` information, intended for listing and display purposes
/// where detailed information like the public key is not necessary or should be omitted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSummary {
    /// Human-readable alias or name for the user.
    pub alias: String,
    /// Unique fingerprint identifying the user.
    pub fingerprint: String,
    /// Device model associated with the user.
    pub device_model: String,
    /// Type of device associated with the user.
    pub device_type: DeviceType,
    /// Current status of the user.
    pub status: UserStatus,
    /// User adding time
    pub add_time: SystemTime,
}

/// Contains a list of users and their permissions, managed as a HashMap.
///
/// `PermissionList` is the struct that holds the collection of users and their associated
/// permission data. It uses a `HashMap` for efficient lookup by user fingerprint. This struct
/// is serialized and persisted by the `PersistentDataManager`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionList {
    /// HashMap of users, keyed by their unique fingerprint for quick access.
    pub users: HashMap<String, User>, // Fingerprint -> User
}

/// Represents errors that can occur within the permission management system.
///
/// `PermissionError` enum categorizes errors specific to permission management operations,
/// including persistence errors, user-related errors (not found, already exists), and path validation errors.
#[derive(Error, Debug)]
pub enum PermissionError {
    /// Error originating from the persistent data storage layer.
    #[error("Persistence error: {0}")]
    Persistence(#[from] PersistenceError),
    /// Error indicating a requested user was not found in the permission list.
    #[error("User not found")]
    UserNotFound,
    /// Error indicating that a provided path is not a directory when a directory is expected.
    #[error("Path is not a directory")]
    NotADirectory,
    /// Error indicating that a provided path is invalid or cannot be used.
    #[error("Path is invalid")]
    InvalidPath,
    /// Application grant persistence failed.
    #[error("Device grants: {0}")]
    Grants(String),
    /// A fingerprint is already bound to different public key material.
    #[error("Fingerprint is already registered with a different key")]
    FingerprintConflict,
}

type TrustChangeHook = Arc<dyn Fn(&str, Option<&UserStatus>) -> Result<(), String> + Send + Sync>;

/// Maximum number of pending pairing requests kept per source address.
const MAX_PENDING_REQUESTS: usize = 5;

/// Manages user permissions, including adding, removing, and verifying users.
///
/// `PermissionManager` is responsible for handling user permissions. It provides functionalities
/// to manage a list of users, their statuses, and associated device information. It uses
/// `PersistentDataManager` for storing and retrieving permission data from disk. It also maintains
/// an in-memory queue of pending pairing requests per source address.
pub struct PermissionManager {
    /// Manages the persistent storage of the permission list.
    storage: PersistentDataManager<PermissionList>,
    /// In-memory cache mapping IP addresses to a queue of pending application fingerprints.
    /// Used for bounding pending pairing requests from specific IPs.
    ip_applications: Arc<RwLock<HashMap<String, VecDeque<String>>>>,
    request_sender: broadcast::Sender<User>,
    change_sender: watch::Sender<u64>,
    trust_change_hook: Option<TrustChangeHook>,
    additional_trust_change_hooks: Arc<StdRwLock<Vec<TrustChangeHook>>>,
}

impl Debug for PermissionManager {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("PermissionManager").finish_non_exhaustive()
    }
}

impl PermissionManager {
    /// Attach the application's durable grant lifecycle to trust edits.
    pub fn set_trust_change_hook(&mut self, hook: TrustChangeHook) {
        self.trust_change_hook = Some(hook);
    }

    pub fn add_trust_change_hook(&mut self, hook: TrustChangeHook) {
        self.additional_trust_change_hooks
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(hook);
    }

    fn notify_trust_change(
        &self,
        fingerprint: &str,
        status: Option<&UserStatus>,
    ) -> Result<(), PermissionError> {
        if let Some(hook) = &self.trust_change_hook {
            hook(fingerprint, status).map_err(PermissionError::Grants)?;
        }
        let hooks = self
            .additional_trust_change_hooks
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for hook in &hooks {
            hook(fingerprint, status).map_err(PermissionError::Grants)?;
        }
        Ok(())
    }

    /// Apply externally written permission changes to the same trust lifecycle
    /// used by in-process edits.
    pub async fn watch_external_changes(&self, cancel: CancellationToken) -> JoinHandle<()> {
        let mut changes = self.storage.subscribe_external();
        let change_sender = self.change_sender.clone();
        let trust_change_hook = self.trust_change_hook.clone();
        let additional_trust_change_hooks = self.additional_trust_change_hooks.clone();
        let mut known = self.storage.read().await.users.clone();
        tokio::spawn(async move {
            loop {
                let received = tokio::select! {
                    _ = cancel.cancelled() => return,
                    received = changes.recv() => received,
                };
                let next = match received {
                    Ok(next) => next,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return,
                };
                let fingerprints = known
                    .keys()
                    .chain(next.users.keys())
                    .cloned()
                    .collect::<collections::HashSet<_>>();
                let mut status_changed = false;
                for fingerprint in fingerprints {
                    let old_status = known.get(&fingerprint).map(|user| &user.status);
                    let new_status = next.users.get(&fingerprint).map(|user| &user.status);
                    if old_status == new_status {
                        continue;
                    }
                    status_changed = true;
                    if let Some(hook) = &trust_change_hook
                        && let Err(error) = hook(&fingerprint, new_status)
                    {
                        log::warn!(
                            "Could not apply external trust change for {fingerprint}: {error}"
                        );
                    }
                    let hooks = additional_trust_change_hooks
                        .read()
                        .unwrap_or_else(PoisonError::into_inner)
                        .clone();
                    for hook in &hooks {
                        if let Err(error) = hook(&fingerprint, new_status) {
                            log::warn!(
                                "Could not apply external trust change for {fingerprint}: {error}"
                            );
                        }
                    }
                }
                known = next.users;
                if status_changed {
                    change_sender
                        .send_modify(|generation| *generation = generation.wrapping_add(1));
                }
            }
        })
    }

    /// Creates a new `PermissionManager` instance, initializing persistent storage.
    ///
    /// This constructor sets up a `PermissionManager` by initializing its `PersistentDataManager`.
    /// It determines the storage path based on the given path argument and loads existing permissions
    /// from disk, or creates a new storage file if one does not exist.
    ///
    /// # Arguments
    /// * `path` - The base path under which the permission data will be stored.
    ///   A subdirectory `.known-clients` will be created under this path for storing permission data.
    ///
    /// # Returns
    /// `Result<Self, PermissionError>` - A `Result` containing the new `PermissionManager` instance,
    ///                                     or a `PermissionError` if initialization fails, typically due to persistence issues.
    ///
    /// # Errors
    /// Returns `PermissionError::Persistence` if the underlying `PersistentDataManager` fails to initialize.
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, PermissionError> {
        let storage_path = path.as_ref().join(".known-clients");
        let storage =
            PersistentDataManager::new(storage_path).map_err(PermissionError::Persistence)?;

        let (request_sender, _) = broadcast::channel(256);

        Ok(Self {
            storage,
            ip_applications: Arc::new(RwLock::new(HashMap::new())),
            request_sender,
            change_sender: watch::channel(0).0,
            trust_change_hook: None,
            additional_trust_change_hooks: Arc::new(StdRwLock::new(Vec::new())),
        })
    }

    /// Subscribe to permission changes.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.change_sender.subscribe()
    }

    /// Lists all users with summary information.
    ///
    /// This method retrieves all users from the permission list and converts each `User` entry
    /// into a `UserSummary`, providing a simplified view of each user.
    ///
    /// # Returns
    /// `Vec<UserSummary>` - A vector of `UserSummary` structs, each representing a user in the system.
    pub async fn list_users(&self) -> Vec<UserSummary> {
        self.storage
            .read()
            .await // Acquire read lock on the permission storage
            .users // Access the users HashMap
            .values() // Iterate over user values
            .map(|user| UserSummary {
                // Map each User to UserSummary
                alias: user.alias.clone(),
                fingerprint: user.fingerprint.clone(),
                device_model: user.device_model.clone(),
                device_type: user.device_type,
                status: user.status.clone(),
                add_time: user.add_time,
            })
            .collect() // Collect UserSummary into a Vec
    }

    /// Adds a new user to the permission system.
    ///
    /// This method adds a new user with the provided details to the permission list. It checks
    /// for user existence based on fingerprint before adding. It also manages an IP-based queue of
    /// pending requests. If the queue for a given IP is full, the oldest request that is still
    /// pending is removed to make space for the new entry; approved and blocked identities are
    /// never evicted.
    ///
    /// # Arguments
    /// * `public_key` - Public key of the user.
    /// * `fingerprint` - Unique fingerprint for the user.
    /// * `alias` - User-friendly alias for the user.
    /// * `device_model` - Model of the user's device.
    /// * `device_type` - Type of the user's device.
    /// * `ip` - IP address from which the user is applying or connecting.
    ///
    /// # Returns
    /// `Result<(), PermissionError>` - A `Result` indicating success or failure.
    ///
    /// # Errors
    /// Returns `PermissionError::UserAlreadyExists` if a user with the given fingerprint already exists.
    /// Returns `PermissionError::Persistence` if there is an issue updating the persistent storage.
    pub async fn add_user(
        &self,
        public_key: String,
        fingerprint: String,
        alias: String,
        device_model: String,
        device_type: DeviceType,
        ip: String,
    ) -> Result<(), PermissionError> {
        let fingerprint_clone = fingerprint.clone();

        self.storage
            .update(|mut permissions| async move {
                // Update operation on persistent storage
                if let Some(existing) = permissions.users.get(&fingerprint) {
                    if existing.public_key != public_key {
                        return Err(PermissionError::FingerprintConflict);
                    }
                    return Ok::<(PermissionList, ()), PermissionError>((permissions, ()));
                }

                let mut ip_apps = self.ip_applications.write().await; // Acquire write lock on IP applications map
                let queue = ip_apps.entry(ip).or_default(); // Get or create queue for the given IP

                // Only pending requests count against the limit. Entries that were
                // decided while queued are dropped without touching their trust, so
                // an approved or blocked identity cannot be evicted by later requests.
                while queue.len() >= MAX_PENDING_REQUESTS {
                    let Some(old_key) = queue.front().cloned() else {
                        break;
                    };
                    let is_pending = permissions
                        .users
                        .get(&old_key)
                        .is_some_and(|user| user.status == UserStatus::Pending);
                    if !is_pending {
                        queue.pop_front();
                        continue;
                    }
                    self.notify_trust_change(&old_key, None)?;
                    queue.pop_front();
                    permissions.users.remove(&old_key);
                }
                queue.push_back(fingerprint.clone()); // Add new fingerprint to the IP queue

                permissions.users.insert(
                    // Insert new user into permissions map
                    fingerprint.clone(),
                    User {
                        public_key,
                        fingerprint,
                        alias,
                        device_model,
                        device_type,
                        status: UserStatus::Pending, // Default status is Pending for new users
                        add_time: SystemTime::now(), // Set current time when adding user
                    },
                );
                Ok::<_, PermissionError>((permissions, ())) // Return updated permissions and success result
            })
            .await?;

        self.change_sender
            .send_modify(|generation| *generation = generation.wrapping_add(1));
        if let Some(user) = self.verify_by_fingerprint(&fingerprint_clone).await
            && user.status != UserStatus::Blocked
            && user.status != UserStatus::Approved
        {
            let _ = self.request_sender.send(user.clone());
        }

        Ok(())
    }

    /// Verifies a user by their fingerprint.
    ///
    /// This method attempts to retrieve a user from the permission list based on their fingerprint.
    ///
    /// # Arguments
    /// * `fingerprint` - The fingerprint of the user to verify.
    ///
    /// # Returns
    /// `Option<User>` - An `Option` containing the `User` struct if found, or `None` if no user with the given fingerprint exists.
    pub async fn verify_by_fingerprint(&self, fingerprint: &str) -> Option<User> {
        self.storage.read().await.users.get(fingerprint).cloned() // Read user from storage by fingerprint, clone if found
    }

    /// Verifies a user by their public key.
    ///
    /// This method searches for a user in the permission list that matches the given public key.
    ///
    /// # Arguments
    /// * `public_key` - The public key of the user to verify.
    ///
    /// # Returns
    /// `Option<User>` - An `Option` containing the `User` struct if a user with the given public key is found, or `None` otherwise.
    pub async fn verify_by_public_key(&self, public_key: &str) -> Option<User> {
        let permissions = self.storage.read().await; // Acquire read lock on permission storage
        permissions
            .users
            .values()
            .find(|user| user.public_key == public_key) // Find user by matching public key
            .cloned() // Clone the found user, if any
    }

    /// Changes the status of a user in the permission system.
    ///
    /// This method updates the `UserStatus` of a user identified by their fingerprint.
    ///
    /// # Arguments
    /// * `fingerprint` - The fingerprint of the user whose status is to be changed.
    /// * `new_status` - The new `UserStatus` to set for the user.
    ///
    /// # Returns
    /// `Result<(), PermissionError>` - A `Result` indicating success or failure.
    ///
    /// # Errors
    /// Returns `PermissionError::UserNotFound` if no user with the given fingerprint is found.
    /// Returns `PermissionError::Persistence` if there is an issue updating the persistent storage.
    pub async fn change_user_status(
        &self,
        fingerprint: &str,
        new_status: UserStatus,
    ) -> Result<(), PermissionError> {
        let status = new_status.clone();
        // The trust hooks run only once the new status is saved; a failed save leaves
        // grants untouched, and a failed hook restores the previous trust file.
        self.storage
            .update_with_commit(
                |mut permissions| async move {
                    // Update operation on persistent storage
                    let user = permissions
                        .users
                        .get_mut(fingerprint) // Get mutable reference to user by fingerprint
                        .ok_or(PermissionError::UserNotFound)?; // Return error if user not found
                    user.status = status.clone(); // Update user status
                    if status != UserStatus::Pending {
                        // A decided request must not keep a pending slot or be
                        // evicted by later requests from the same address.
                        let mut ip_apps = self.ip_applications.write().await;
                        for queue in ip_apps.values_mut() {
                            queue.retain(|queued| queued != fingerprint);
                        }
                    }
                    Ok::<_, PermissionError>((permissions, ())) // Return updated permissions and success result
                },
                || self.notify_trust_change(fingerprint, Some(&new_status)),
            )
            .await?;
        self.change_sender
            .send_modify(|generation| *generation = generation.wrapping_add(1));
        Ok(())
    }

    /// Removes a user from the permission system.
    ///
    /// This method deletes a user from the permission list based on their fingerprint.
    ///
    /// # Arguments
    /// * `fingerprint` - The fingerprint of the user to remove.
    ///
    /// # Returns
    /// `Result<(), PermissionError>` - A `Result` indicating success or failure.
    ///
    /// # Errors
    /// Returns `PermissionError::UserNotFound` if no user with the given fingerprint is found.
    /// Returns `PermissionError::Persistence` if there is an issue updating the persistent storage.
    pub async fn remove_user(&self, fingerprint: &str) -> Result<(), PermissionError> {
        // The trust hooks run only once the removal is saved; a failed save leaves
        // grants untouched, and a failed hook restores the previous trust file.
        self.storage
            .update_with_commit(
                |mut permissions| async move {
                    // Update operation on persistent storage
                    if permissions.users.remove(fingerprint).is_none() {
                        // Try to remove user by fingerprint
                        Err(PermissionError::UserNotFound) // Return error if user was not found (remove returned None)
                    } else {
                        Ok::<_, PermissionError>((permissions, ())) // Return updated permissions and success result
                    }
                },
                || self.notify_trust_change(fingerprint, None),
            )
            .await?;
        self.change_sender
            .send_modify(|generation| *generation = generation.wrapping_add(1));
        Ok(())
    }

    /// Finds users by their device type.
    ///
    /// This method retrieves a list of users whose device type matches the specified `device_type`.
    ///
    /// # Arguments
    /// * `device_type` - The `DeviceType` to filter users by.
    ///
    /// # Returns
    /// `Vec<UserSummary>` - A vector of `UserSummary` structs for users matching the given device type.
    pub async fn find_by_device_type(&self, device_type: DeviceType) -> Vec<UserSummary> {
        self.storage
            .read()
            .await // Acquire read lock on permission storage
            .users
            .values()
            .filter(|u| u.device_type == device_type) // Filter users by device_type
            .map(|user| UserSummary {
                // Map filtered users to UserSummary
                alias: user.alias.clone(),
                fingerprint: user.fingerprint.clone(),
                device_model: user.device_model.clone(),
                device_type: user.device_type,
                status: user.status.clone(),
                add_time: user.add_time,
            })
            .collect() // Collect UserSummary into a Vec
    }

    /// Gets the count of users with 'Pending' status.
    ///
    /// This method counts the number of users in the permission list who have a `UserStatus` of `Pending`.
    ///
    /// # Returns
    /// `usize` - The number of users with 'Pending' status.
    pub async fn get_pending_count(&self) -> usize {
        self.storage
            .read()
            .await // Acquire read lock on permission storage
            .users
            .values()
            .filter(|u| u.status == UserStatus::Pending) // Filter users by Pending status
            .count() // Count the number of filtered users
    }

    /// Subscribes to changes in the permission list.
    ///
    /// This method returns a broadcast receiver that will receive updates whenever the `PermissionList` is modified.
    /// Subscribers can use this to react to changes in user permissions, such as additions, removals, or status updates.
    ///
    /// # Returns
    /// `broadcast::Receiver<PermissionList>` - A broadcast receiver for `PermissionList` updates.
    pub fn subscribe_changes(&self) -> broadcast::Receiver<PermissionList> {
        self.storage.subscribe() // Subscribe to storage changes and return the receiver
    }

    pub fn subscribe_new_user(&self) -> broadcast::Receiver<User> {
        self.request_sender.subscribe()
    }
}

#[cfg(test)]
mod change_tests {
    use super::{PermissionError, PermissionManager, UserStatus};
    use crate::utils::DeviceType;
    use std::env;
    use std::error;
    use std::fs;
    use std::process as std_process;
    use std::sync::{Arc, Mutex};
    use tokio::time::{Duration, timeout};
    use uuid::Uuid;
    const TEST_DEADLINE: Duration = Duration::from_secs(60);
    use tokio_util::sync::CancellationToken;

    async fn add_permission_user(
        manager: &PermissionManager,
        fingerprint: &str,
    ) -> Result<(), PermissionError> {
        manager
            .add_user(
                format!("{fingerprint}-key"),
                fingerprint.into(),
                fingerprint.into(),
                "model".into(),
                DeviceType::Desktop,
                format!("{fingerprint}-ip"),
            )
            .await
    }

    async fn add_user_from(
        manager: &PermissionManager,
        fingerprint: &str,
        ip: &str,
    ) -> Result<(), PermissionError> {
        manager
            .add_user(
                format!("{fingerprint}-key"),
                fingerprint.into(),
                fingerprint.into(),
                "model".into(),
                DeviceType::Desktop,
                ip.into(),
            )
            .await
    }

    /// Trust changes observed through the hook, recorded as (fingerprint, new status).
    type ObservedTrustChanges = Arc<Mutex<Vec<(String, Option<UserStatus>)>>>;

    fn record_trust_changes(manager: &mut PermissionManager) -> ObservedTrustChanges {
        let changes = Arc::new(Mutex::new(Vec::new()));
        let observed = changes.clone();
        manager.set_trust_change_hook(Arc::new(move |fingerprint, status| {
            observed
                .lock()
                .unwrap()
                .push((fingerprint.to_owned(), status.cloned()));
            Ok(())
        }));
        changes
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_trust_save_changes_nothing_and_can_be_retried()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::PermissionsExt;

        let dir =
            std::env::temp_dir().join(format!("permission-failed-save-{}", uuid::Uuid::new_v4()));
        let mut manager = PermissionManager::new(&dir)?;
        let changes = record_trust_changes(&mut manager);
        add_permission_user(&manager, "device").await?;
        manager
            .change_user_status("device", UserStatus::Approved)
            .await?;
        let notified_before = changes.lock().unwrap().len();

        // A process that may write anyway (such as root) would let this pass without a failed write.
        let trust_file = dir.join(".known-clients");
        std::fs::set_permissions(&trust_file, std::fs::Permissions::from_mode(0o444))?;
        assert!(
            std::fs::OpenOptions::new()
                .write(true)
                .open(&trust_file)
                .is_err(),
            "trust file is still writable; run this test as an unprivileged user"
        );
        assert!(
            manager
                .change_user_status("device", UserStatus::Blocked)
                .await
                .is_err()
        );
        assert_eq!(
            manager
                .verify_by_fingerprint("device")
                .await
                .map(|user| user.status),
            Some(UserStatus::Approved)
        );
        assert_eq!(changes.lock().unwrap().len(), notified_before);

        std::fs::set_permissions(&trust_file, std::fs::Permissions::from_mode(0o644))?;
        manager
            .change_user_status("device", UserStatus::Blocked)
            .await?;
        assert_eq!(
            manager
                .verify_by_fingerprint("device")
                .await
                .map(|user| user.status),
            Some(UserStatus::Blocked)
        );
        assert_eq!(
            changes.lock().unwrap().last(),
            Some(&("device".to_owned(), Some(UserStatus::Blocked)))
        );
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[tokio::test]
    async fn separate_managers_merge_updates_to_the_same_file() -> Result<(), Box<dyn error::Error>>
    {
        let dir = env::temp_dir().join(format!("permission-manager-merge-{}", Uuid::new_v4()));
        let first = PermissionManager::new(&dir)?;
        let second = PermissionManager::new(&dir)?;

        add_permission_user(&first, "first-fingerprint").await?;
        add_permission_user(&second, "second-fingerprint").await?;

        let latest = PermissionManager::new(&dir)?;
        assert!(
            latest
                .verify_by_fingerprint("first-fingerprint")
                .await
                .is_some()
        );
        assert!(
            latest
                .verify_by_fingerprint("second-fingerprint")
                .await
                .is_some()
        );
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_updates_and_watcher_reloads_do_not_deadlock()
    -> Result<(), Box<dyn error::Error>> {
        let dir = env::temp_dir().join(format!("permission-manager-concurrent-{}", Uuid::new_v4()));
        let first = Arc::new(PermissionManager::new(&dir)?);
        let second = Arc::new(PermissionManager::new(&dir)?);
        let writers = [first.clone(), second.clone()].map(|manager| {
            tokio::spawn(async move {
                for index in 0..40 {
                    let fingerprint = format!("{:p}-{index}", Arc::as_ptr(&manager));
                    add_permission_user(&manager, &fingerprint).await?;
                }
                Ok::<_, PermissionError>(())
            })
        });
        timeout(Duration::from_secs(60), async {
            for writer in writers {
                writer.await??;
            }
            Ok::<_, Box<dyn error::Error>>(())
        })
        .await??;
        let latest = PermissionManager::new(&dir)?;
        assert_eq!(latest.list_users().await.len(), 80);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[tokio::test]
    async fn generation_tracks_mutations_not_reads() -> Result<(), Box<dyn error::Error>> {
        let dir = env::temp_dir().join(format!("permission-generation-{}", std_process::id()));
        let _ = fs::remove_dir_all(&dir);
        let manager = PermissionManager::new(&dir)?;
        manager
            .add_user(
                "key".into(),
                "fingerprint".into(),
                "alias".into(),
                "model".into(),
                DeviceType::Desktop,
                "ip".into(),
            )
            .await?;
        let mut changes = manager.changes();
        let initial = *changes.borrow_and_update();
        manager.verify_by_fingerprint("fingerprint").await;
        assert!(!changes.has_changed()?);
        manager
            .change_user_status("fingerprint", UserStatus::Approved)
            .await?;
        assert_eq!(*changes.borrow_and_update(), initial + 1);
        manager.remove_user("fingerprint").await?;
        assert_eq!(*changes.borrow_and_update(), initial + 2);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[tokio::test]
    async fn existing_fingerprint_with_another_key_is_a_conflict()
    -> Result<(), Box<dyn error::Error>> {
        let dir = env::temp_dir().join(format!("permission-conflict-{}", std_process::id()));
        let _ = fs::remove_dir_all(&dir);
        let manager = PermissionManager::new(&dir)?;
        manager
            .add_user(
                "original-key".into(),
                "fingerprint".into(),
                "one".into(),
                "model".into(),
                DeviceType::Desktop,
                "ip".into(),
            )
            .await?;
        let result = manager
            .add_user(
                "different-key".into(),
                "fingerprint".into(),
                "two".into(),
                "model".into(),
                DeviceType::Desktop,
                "ip".into(),
            )
            .await;
        assert!(matches!(result, Err(PermissionError::FingerprintConflict)));
        let duplicate = manager
            .add_user(
                "original-key".into(),
                "fingerprint".into(),
                "one".into(),
                "model".into(),
                DeviceType::Desktop,
                "ip".into(),
            )
            .await;
        assert!(duplicate.is_ok());
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[tokio::test]
    async fn file_reload_notifies_once_and_runs_trust_hook() -> Result<(), Box<dyn error::Error>> {
        let dir = env::temp_dir().join(format!("permission-file-reload-{}", std_process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut manager = PermissionManager::new(&dir)?;
        manager
            .add_user(
                "key".into(),
                "fingerprint".into(),
                "alias".into(),
                "model".into(),
                DeviceType::Desktop,
                "ip".into(),
            )
            .await?;
        manager
            .change_user_status("fingerprint", UserStatus::Approved)
            .await?;
        let hook_calls = Arc::new(Mutex::new(Vec::new()));
        let observed = hook_calls.clone();
        manager.set_trust_change_hook(Arc::new(move |fingerprint, status| {
            observed
                .lock()
                .unwrap()
                .push((fingerprint.to_owned(), status.cloned()));
            Ok(())
        }));
        manager
            .watch_external_changes(CancellationToken::new())
            .await;
        let mut changes = manager.changes();
        let initial_generation = *changes.borrow_and_update();

        let cli_manager = PermissionManager::new(&dir)?;
        cli_manager
            .change_user_status("fingerprint", UserStatus::Blocked)
            .await?;

        timeout(TEST_DEADLINE, changes.changed()).await??;
        assert_eq!(*changes.borrow(), initial_generation + 1);
        assert_eq!(
            *hook_calls.lock().unwrap(),
            vec![("fingerprint".to_owned(), Some(UserStatus::Blocked))]
        );
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[tokio::test]
    async fn approved_and_blocked_requests_survive_later_registrations()
    -> Result<(), Box<dyn error::Error>> {
        for (label, status) in [
            ("approved", UserStatus::Approved),
            ("blocked", UserStatus::Blocked),
        ] {
            let dir =
                env::temp_dir().join(format!("permission-decided-{label}-{}", Uuid::new_v4()));
            let mut manager = PermissionManager::new(&dir)?;
            let changes = record_trust_changes(&mut manager);

            add_user_from(&manager, "victim", "127.0.0.1").await?;
            manager.change_user_status("victim", status.clone()).await?;
            // A duplicate registration must not refill the pending queue.
            add_user_from(&manager, "victim", "127.0.0.1").await?;
            // Registrations from another address use their own queue.
            for index in 0..5 {
                add_user_from(&manager, &format!("other-{index}"), "127.0.0.2").await?;
            }

            for index in 0..4 {
                add_user_from(&manager, &format!("new-{index}"), "127.0.0.1").await?;
            }
            assert_eq!(
                manager
                    .verify_by_fingerprint("victim")
                    .await
                    .map(|user| user.status),
                Some(status.clone()),
                "{label} request disappeared after four later registrations"
            );

            add_user_from(&manager, "new-4", "127.0.0.1").await?;
            assert_eq!(
                manager
                    .verify_by_fingerprint("victim")
                    .await
                    .map(|user| user.status),
                Some(status.clone()),
                "{label} request was evicted by five later registrations"
            );

            let reopened = PermissionManager::new(&dir)?;
            assert_eq!(
                reopened
                    .verify_by_fingerprint("victim")
                    .await
                    .map(|user| user.status),
                Some(status.clone()),
                "{label} request was lost after reopening storage"
            );
            add_user_from(&reopened, "victim", "127.0.0.1").await?;
            assert_eq!(
                reopened
                    .verify_by_fingerprint("victim")
                    .await
                    .map(|user| user.status),
                Some(status.clone()),
                "re-registration reset a {label} request"
            );

            assert_eq!(
                *changes.lock().unwrap(),
                vec![("victim".to_owned(), Some(status.clone()))],
                "{label} request produced an unexpected trust change"
            );
            fs::remove_dir_all(dir)?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn pending_requests_stay_bounded_and_release_decided_entries()
    -> Result<(), Box<dyn error::Error>> {
        let dir = env::temp_dir().join(format!("permission-pending-bounded-{}", Uuid::new_v4()));
        let mut manager = PermissionManager::new(&dir)?;
        let changes = record_trust_changes(&mut manager);

        for name in ["one", "two", "three", "four", "five"] {
            add_user_from(&manager, name, "127.0.0.1").await?;
        }
        manager
            .change_user_status("one", UserStatus::Approved)
            .await?;
        manager
            .change_user_status("two", UserStatus::Blocked)
            .await?;

        // Decided entries no longer occupy pending slots, so these two do not
        // push anyone out yet.
        add_user_from(&manager, "six", "127.0.0.1").await?;
        add_user_from(&manager, "seven", "127.0.0.1").await?;
        assert!(manager.verify_by_fingerprint("three").await.is_some());

        // The queue is full again; the oldest remaining pending request goes.
        add_user_from(&manager, "eight", "127.0.0.1").await?;

        assert_eq!(
            manager
                .verify_by_fingerprint("one")
                .await
                .map(|user| user.status),
            Some(UserStatus::Approved)
        );
        assert_eq!(
            manager
                .verify_by_fingerprint("two")
                .await
                .map(|user| user.status),
            Some(UserStatus::Blocked)
        );
        assert!(
            manager.verify_by_fingerprint("three").await.is_none(),
            "the oldest pending request should have been evicted"
        );
        assert_eq!(manager.get_pending_count().await, 5);

        let reopened = PermissionManager::new(&dir)?;
        assert_eq!(
            reopened
                .verify_by_fingerprint("one")
                .await
                .map(|user| user.status),
            Some(UserStatus::Approved)
        );
        assert_eq!(
            reopened
                .verify_by_fingerprint("two")
                .await
                .map(|user| user.status),
            Some(UserStatus::Blocked)
        );
        assert!(reopened.verify_by_fingerprint("three").await.is_none());

        // A re-registration after eviction becomes a fresh pending request.
        add_user_from(&reopened, "three", "127.0.0.1").await?;
        assert_eq!(
            reopened
                .verify_by_fingerprint("three")
                .await
                .map(|user| user.status),
            Some(UserStatus::Pending)
        );

        assert_eq!(
            *changes.lock().unwrap(),
            vec![
                ("one".to_owned(), Some(UserStatus::Approved)),
                ("two".to_owned(), Some(UserStatus::Blocked)),
                ("three".to_owned(), None),
            ]
        );
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[tokio::test]
    async fn externally_approved_request_is_not_evicted_from_the_queue()
    -> Result<(), Box<dyn error::Error>> {
        let dir = env::temp_dir().join(format!("permission-external-decision-{}", Uuid::new_v4()));
        let mut manager = PermissionManager::new(&dir)?;
        let changes = record_trust_changes(&mut manager);
        add_user_from(&manager, "victim", "127.0.0.1").await?;
        let mut generation = manager.changes();
        manager
            .watch_external_changes(CancellationToken::new())
            .await;

        let other = PermissionManager::new(&dir)?;
        other
            .change_user_status("victim", UserStatus::Approved)
            .await?;
        timeout(TEST_DEADLINE, generation.changed()).await??;
        assert_eq!(
            manager
                .verify_by_fingerprint("victim")
                .await
                .map(|user| user.status),
            Some(UserStatus::Approved)
        );

        for index in 0..5 {
            add_user_from(&manager, &format!("new-{index}"), "127.0.0.1").await?;
        }
        assert_eq!(
            manager
                .verify_by_fingerprint("victim")
                .await
                .map(|user| user.status),
            Some(UserStatus::Approved),
            "an externally approved request was evicted from the stale queue"
        );
        assert!(
            changes
                .lock()
                .unwrap()
                .iter()
                .all(|(fingerprint, status)| fingerprint != "victim" || status.is_some()),
            "evicting a decided request reported a trust removal"
        );
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
