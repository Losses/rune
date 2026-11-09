//! Node-local encrypted credentials. This protects against accidental leaks of
//! configuration files; it does not protect against someone who can read the
//! configuration directory, which contains both the encrypted file and key.

use std::{
    collections::HashMap,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, ErrorKind, Read, Write},
    path::{Path, PathBuf},
};

use ring::{
    aead::{Aad, CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey},
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::library_registry::registry_write_guard;

const VERSION: u8 = 1;
const KEY_LENGTH: usize = 32;
const NONCE_LENGTH: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CredentialRef(pub Uuid);

impl fmt::Display for CredentialRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

pub struct Secret(Vec<u8>);

impl Secret {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    fn take_bytes(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.0)
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

pub struct Credential {
    pub username: String,
    pub secret: Secret,
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credential")
            .field("username", &self.username)
            .field("secret", &self.secret)
            .finish()
    }
}

#[derive(Error)]
pub enum CredentialStoreError {
    #[error("Credential is missing: {0}")]
    CredentialMissing(CredentialRef),
    #[error("Credential store is unreadable")]
    CredentialStoreUnreadable,
    #[error("Credential store I/O error")]
    CredentialStoreIo(#[source] io::Error),
}

impl fmt::Debug for CredentialStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CredentialMissing(reference) => {
                f.debug_tuple("CredentialMissing").field(reference).finish()
            }
            Self::CredentialStoreUnreadable => f.write_str("CredentialStoreUnreadable"),
            Self::CredentialStoreIo(_) => f.write_str("CredentialStoreIo"),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Entries(HashMap<CredentialRef, Entry>);

#[derive(Serialize, Deserialize)]
struct Entry {
    username: String,
    secret: Vec<u8>,
}

impl Drop for Entry {
    fn drop(&mut self) {
        self.secret.zeroize();
    }
}

pub struct CredentialStore {
    file: PathBuf,
    key_file: PathBuf,
}

impl CredentialStore {
    pub fn open(config_dir: &Path) -> Self {
        Self {
            file: config_dir.join("credentials.enc"),
            key_file: config_dir.join("credentials.key"),
        }
    }

    pub async fn put(
        &self,
        mut credential: Credential,
    ) -> Result<CredentialRef, CredentialStoreError> {
        let _writer = registry_write_guard().await;
        let mut key = Zeroizing::new(self.load_or_create_key()?);
        let mut entries = self.read_entries(&key)?;
        let reference = CredentialRef(Uuid::new_v4());
        entries.0.insert(
            reference,
            Entry {
                username: credential.username,
                secret: credential.secret.take_bytes(),
            },
        );
        let result = self.write_entries(key.as_mut_slice(), &entries);
        result.map(|()| reference)
    }

    pub async fn get(&self, reference: CredentialRef) -> Result<Credential, CredentialStoreError> {
        let _writer = registry_write_guard().await;
        let key = Zeroizing::new(self.load_key_for_reference(reference)?);
        let entries = self.read_entries(&key)?;
        let entry = entries
            .0
            .get(&reference)
            .ok_or(CredentialStoreError::CredentialMissing(reference))?;
        Ok(Credential {
            username: entry.username.clone(),
            secret: Secret::new(entry.secret.clone()),
        })
    }

    pub async fn replace(
        &self,
        reference: CredentialRef,
        mut credential: Credential,
    ) -> Result<(), CredentialStoreError> {
        let _writer = registry_write_guard().await;
        let mut key = Zeroizing::new(self.load_key_for_reference(reference)?);
        let mut entries = self.read_entries(&key)?;
        if !entries.0.contains_key(&reference) {
            return Err(CredentialStoreError::CredentialMissing(reference));
        }
        entries.0.insert(
            reference,
            Entry {
                username: credential.username,
                secret: credential.secret.take_bytes(),
            },
        );
        self.write_entries(key.as_mut_slice(), &entries)
    }

    pub async fn remove(&self, reference: CredentialRef) -> Result<(), CredentialStoreError> {
        let _writer = registry_write_guard().await;
        let mut key = Zeroizing::new(self.load_key_for_reference(reference)?);
        let mut entries = self.read_entries(&key)?;
        if entries.0.remove(&reference).is_none() {
            return Err(CredentialStoreError::CredentialMissing(reference));
        }
        self.write_entries(key.as_mut_slice(), &entries)
    }

    fn load_or_create_key(&self) -> Result<Vec<u8>, CredentialStoreError> {
        match self.load_key() {
            Ok(key) => Ok(key),
            Err(CredentialStoreError::CredentialStoreIo(error))
                if error.kind() == ErrorKind::NotFound && !self.file.exists() =>
            {
                self.create_key()
            }
            Err(CredentialStoreError::CredentialStoreIo(error))
                if error.kind() == ErrorKind::NotFound =>
            {
                Err(CredentialStoreError::CredentialStoreUnreadable)
            }
            Err(error) => Err(error),
        }
    }

    fn load_key(&self) -> Result<Vec<u8>, CredentialStoreError> {
        let mut key = Vec::new();
        File::open(&self.key_file)
            .and_then(|mut file| file.read_to_end(&mut key))
            .map_err(CredentialStoreError::CredentialStoreIo)?;
        if key.len() != KEY_LENGTH {
            key.zeroize();
            return Err(CredentialStoreError::CredentialStoreUnreadable);
        }
        Ok(key)
    }

    fn load_key_for_reference(
        &self,
        reference: CredentialRef,
    ) -> Result<Vec<u8>, CredentialStoreError> {
        match self.load_key() {
            Ok(key) => Ok(key),
            Err(CredentialStoreError::CredentialStoreIo(error))
                if error.kind() == ErrorKind::NotFound && !self.file.exists() =>
            {
                Err(CredentialStoreError::CredentialMissing(reference))
            }
            Err(CredentialStoreError::CredentialStoreIo(error))
                if error.kind() == ErrorKind::NotFound =>
            {
                Err(CredentialStoreError::CredentialStoreUnreadable)
            }
            Err(error) => Err(error),
        }
    }

    fn create_key(&self) -> Result<Vec<u8>, CredentialStoreError> {
        let mut key = Zeroizing::new(vec![0; KEY_LENGTH]);
        SystemRandom::new()
            .fill(&mut key)
            .map_err(|_| CredentialStoreError::CredentialStoreUnreadable)?;
        let temp = temporary_path(&self.key_file);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let result = (|| {
            let mut file = options.open(&temp)?;
            file.write_all(&key)?;
            file.sync_all()?;
            match fs::hard_link(&temp, &self.key_file) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(()),
                Err(error) => Err(error),
            }
        })();
        let _ = fs::remove_file(temp);
        if let Err(error) = result {
            return Err(CredentialStoreError::CredentialStoreIo(error));
        }
        if self.key_file.exists() {
            self.load_key()
        } else {
            Err(CredentialStoreError::CredentialStoreIo(io::Error::new(
                ErrorKind::NotFound,
                "credential key creation failed",
            )))
        }
    }

    fn read_entries(&self, key: &[u8]) -> Result<Entries, CredentialStoreError> {
        let data = match fs::read(&self.file) {
            Ok(data) => data,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Entries::default()),
            Err(error) => return Err(CredentialStoreError::CredentialStoreIo(error)),
        };
        if data.len() <= 1 + NONCE_LENGTH || data[0] != VERSION {
            return Err(CredentialStoreError::CredentialStoreUnreadable);
        }
        let mut plaintext = Zeroizing::new(data[1 + NONCE_LENGTH..].to_vec());
        let nonce = Nonce::try_assume_unique_for_key(&data[1..1 + NONCE_LENGTH])
            .map_err(|_| CredentialStoreError::CredentialStoreUnreadable)?;
        let cipher = cipher(key)?;
        let plaintext = cipher
            .open_in_place(nonce, Aad::empty(), &mut plaintext)
            .map_err(|_| CredentialStoreError::CredentialStoreUnreadable)?;
        let plaintext = Zeroizing::new(plaintext.to_vec());
        serde_json::from_slice(&plaintext)
            .map_err(|_| CredentialStoreError::CredentialStoreUnreadable)
    }

    fn write_entries(&self, key: &mut [u8], entries: &Entries) -> Result<(), CredentialStoreError> {
        let mut plaintext = serde_json::to_vec(entries)
            .map_err(|_| CredentialStoreError::CredentialStoreUnreadable)?;
        let encrypted = (|| {
            let mut nonce_bytes = [0; NONCE_LENGTH];
            SystemRandom::new()
                .fill(&mut nonce_bytes)
                .map_err(|_| CredentialStoreError::CredentialStoreUnreadable)?;
            let nonce = Nonce::assume_unique_for_key(nonce_bytes);
            let cipher = cipher(key)?;
            cipher
                .seal_in_place_append_tag(nonce, Aad::empty(), &mut plaintext)
                .map_err(|_| CredentialStoreError::CredentialStoreUnreadable)?;
            let mut output = Vec::with_capacity(1 + NONCE_LENGTH + plaintext.len());
            output.push(VERSION);
            output.extend_from_slice(&nonce_bytes);
            output.extend_from_slice(&plaintext);
            Ok(Zeroizing::new(output))
        })();
        plaintext.zeroize();
        let output = encrypted?;
        let temp = temporary_path(&self.file);
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            file.write_all(&output)?;
            file.sync_all()?;
            fs::rename(&temp, &self.file)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result.map_err(CredentialStoreError::CredentialStoreIo)
    }
}

fn cipher(key: &[u8]) -> Result<LessSafeKey, CredentialStoreError> {
    let key = UnboundKey::new(&CHACHA20_POLY1305, key)
        .map_err(|_| CredentialStoreError::CredentialStoreUnreadable)?;
    Ok(LessSafeKey::new(key))
}

fn temporary_path(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{name}-{}.tmp", Uuid::new_v4()))
}

#[cfg(test)]
mod tests {
    use super::{Credential, CredentialRef, CredentialStore, CredentialStoreError, Secret};
    use crate::library_registry::{
        LibraryKind, LibraryRecord, LibraryRecordId, LibraryRegistry, SubsonicOpenMode,
    };
    use std::{fs, io, sync::Arc};
    use tempfile::TempDir;
    use uuid::Uuid;

    fn credential(username: &str, secret: &str) -> Credential {
        Credential {
            username: username.into(),
            secret: Secret::new(secret.as_bytes().to_vec()),
        }
    }

    #[tokio::test]
    async fn credentials_round_trip_replace_and_remove() {
        let dir = TempDir::new().unwrap();
        let store = CredentialStore::open(dir.path());
        let reference = store.put(credential("alice", "secret-one")).await.unwrap();
        let found = store.get(reference).await.unwrap();
        assert_eq!(found.username, "alice");
        assert_eq!(found.secret.as_bytes(), b"secret-one");
        store
            .replace(reference, credential("bob", "secret-two"))
            .await
            .unwrap();
        let found = store.get(reference).await.unwrap();
        assert_eq!(found.username, "bob");
        assert_eq!(found.secret.as_bytes(), b"secret-two");
        store.remove(reference).await.unwrap();
        assert!(matches!(
            store.get(reference).await,
            Err(CredentialStoreError::CredentialMissing(_))
        ));
    }

    #[tokio::test]
    async fn credential_file_and_registry_do_not_store_username_or_secret_in_plaintext() {
        let dir = TempDir::new().unwrap();
        let store = CredentialStore::open(dir.path());
        let reference = store
            .put(credential("private-username", "private-secret-bytes"))
            .await
            .unwrap();
        let record = LibraryRecord {
            record_id: LibraryRecordId(Uuid::new_v4()),
            alias: "Subsonic".into(),
            kind: LibraryKind::Subsonic {
                endpoint: "https://music.example".into(),
                credential: reference,
                mode: SubsonicOpenMode::PlayOnThisDevice,
            },
        };
        LibraryRegistry::update(dir.path(), |registry| {
            registry.libraries.push(record);
            Ok(())
        })
        .await
        .unwrap();
        for file in ["credentials.enc", "libraries.json"] {
            let bytes = fs::read(dir.path().join(file)).unwrap();
            assert!(
                !bytes
                    .windows(b"private-username".len())
                    .any(|part| part == b"private-username")
            );
            assert!(
                !bytes
                    .windows(b"private-secret-bytes".len())
                    .any(|part| part == b"private-secret-bytes")
            );
        }
    }

    #[tokio::test]
    async fn wrong_key_is_unreadable_and_does_not_overwrite_credentials_file() {
        let dir = TempDir::new().unwrap();
        let store = CredentialStore::open(dir.path());
        let reference = store.put(credential("alice", "secret")).await.unwrap();
        let before = fs::read(dir.path().join("credentials.enc")).unwrap();
        fs::write(dir.path().join("credentials.key"), [7_u8; 32]).unwrap();
        assert!(matches!(
            store.get(reference).await,
            Err(CredentialStoreError::CredentialStoreUnreadable)
        ));
        assert_eq!(
            fs::read(dir.path().join("credentials.enc")).unwrap(),
            before
        );
        fs::remove_file(dir.path().join("credentials.key")).unwrap();
        assert!(matches!(
            store.get(reference).await,
            Err(CredentialStoreError::CredentialStoreUnreadable)
        ));
        assert_eq!(
            fs::read(dir.path().join("credentials.enc")).unwrap(),
            before
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn key_file_has_private_mode_when_created() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        CredentialStore::open(dir.path())
            .put(credential("alice", "secret"))
            .await
            .unwrap();
        let mode = fs::metadata(dir.path().join("credentials.key"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn debug_and_error_display_redact_secret() {
        let secret = "do-not-print-this";
        let credential = credential("alice", secret);
        assert!(!format!("{credential:?}").contains(secret));
        assert!(!format!("{}", credential.secret).contains(secret));
        let missing = CredentialStoreError::CredentialMissing(CredentialRef(Uuid::new_v4()));
        let unreadable = CredentialStoreError::CredentialStoreUnreadable;
        let io = CredentialStoreError::CredentialStoreIo(io::Error::other(secret));
        for error in [missing, unreadable, io] {
            assert!(!error.to_string().contains(secret));
            assert!(!format!("{error:?}").contains(secret));
        }
    }

    #[tokio::test]
    async fn removing_subsonic_record_removes_its_credential() {
        let dir = TempDir::new().unwrap();
        let store = CredentialStore::open(dir.path());
        let reference = store.put(credential("alice", "secret")).await.unwrap();
        let record_id = LibraryRecordId(Uuid::new_v4());
        LibraryRegistry::update(dir.path(), |registry| {
            registry.libraries.push(LibraryRecord {
                record_id,
                alias: "Subsonic".into(),
                kind: LibraryKind::Subsonic {
                    endpoint: "https://music.example".into(),
                    credential: reference,
                    mode: SubsonicOpenMode::PlayOnThisDevice,
                },
            });
            Ok(())
        })
        .await
        .unwrap();
        assert!(
            LibraryRegistry::remove_record(dir.path(), record_id, &store)
                .await
                .unwrap()
        );
        assert!(matches!(
            store.get(reference).await,
            Err(CredentialStoreError::CredentialMissing(_))
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_puts_both_survive() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(CredentialStore::open(dir.path()));
        let first_store = store.clone();
        let first = tokio::spawn(async move {
            first_store
                .put(credential("first", "secret-a"))
                .await
                .unwrap()
        });
        let second_store = Arc::new(CredentialStore::open(dir.path()));
        let second_reader = second_store.clone();
        let second = tokio::spawn(async move {
            second_reader
                .put(credential("second", "secret-b"))
                .await
                .unwrap()
        });
        let first_ref = first.await.unwrap();
        let second_ref = second.await.unwrap();
        assert_eq!(
            store.get(first_ref).await.unwrap().secret.as_bytes(),
            b"secret-a"
        );
        assert_eq!(
            second_store
                .get(second_ref)
                .await
                .unwrap()
                .secret
                .as_bytes(),
            b"secret-b"
        );
    }
}
