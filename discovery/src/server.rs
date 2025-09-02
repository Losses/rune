use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use log::error;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{RwLock, broadcast};

use crate::persistent::{PersistenceError, PersistentDataManager};
use crate::utils::DeviceType;

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
}

/// Manages user permissions, including adding, removing, and verifying users.
///
/// `PermissionManager` is responsible for handling user permissions. It provides functionalities
/// to manage a list of users, their statuses, and associated device information. It uses
/// `PersistentDataManager` for storing and retrieving permission data from disk. It also maintains
/// an in-memory cache of IP addresses to application fingerprints for rate limiting or tracking purposes.
#[derive(Debug)]
pub struct PermissionManager {
    /// Manages the persistent storage of the permission list.
    storage: PersistentDataManager<PermissionList>,
    /// In-memory cache mapping IP addresses to a queue of application fingerprints.
    /// Used for tracking recent applications from specific IPs, possibly for rate limiting or security.
    ip_applications: Arc<RwLock<HashMap<String, VecDeque<String>>>>,
    request_sender: broadcast::Sender<User>,
}

impl PermissionManager {
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
        })
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
    /// for user existence based on fingerprint before adding. It also manages an IP-based application queue,
    /// potentially for rate limiting or tracking purposes. If the queue for a given IP is full (max 5 entries),
    /// the oldest entry (and its associated user if any) is removed to make space for the new entry.
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
                if permissions.users.contains_key(&fingerprint) {
                    // Check if user already exists
                    return Ok::<(PermissionList, ()), PermissionError>((permissions, ()));
                }

                let mut ip_apps = self.ip_applications.write().await; // Acquire write lock on IP applications map
                let queue = ip_apps.entry(ip).or_default(); // Get or create queue for the given IP

                if queue.len() >= 5 {
                    // Check if queue is full (max 5 entries)
                    if let Some(old_key) = queue.pop_front() {
                        // Remove oldest entry if queue is full
                        permissions.users.remove(&old_key); // Optionally remove user associated with the oldest key
                    }
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
                Ok((permissions, ())) // Return updated permissions and success result
            })
            .await?;

        if let Some(user) = self.verify_by_fingerprint(&fingerprint_clone).await
            && user.status != UserStatus::Blocked && user.status != UserStatus::Approved {
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
        self.storage
            .update(|mut permissions| async move {
                // Update operation on persistent storage
                let user = permissions
                    .users
                    .get_mut(fingerprint) // Get mutable reference to user by fingerprint
                    .ok_or(PermissionError::UserNotFound)?; // Return error if user not found
                user.status = new_status.clone(); // Update user status
                Ok((permissions, ())) // Return updated permissions and success result
            })
            .await
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
        self.storage
            .update(|mut permissions| async move {
                // Update operation on persistent storage
                if permissions.users.remove(fingerprint).is_none() {
                    // Try to remove user by fingerprint
                    Err(PermissionError::UserNotFound) // Return error if user was not found (remove returned None)
                } else {
                    Ok((permissions, ())) // Return updated permissions and success result
                }
            })
            .await
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
