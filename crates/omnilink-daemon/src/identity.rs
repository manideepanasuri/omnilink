//! Ed25519 identity keypair keystore, permission enforcement, and persistence.

use directories::ProjectDirs;
use ed25519_dalek::SigningKey;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// File name for the identity key within the data directory.
pub const KEY_FILE_NAME: &str = "identity.key";

/// Length in bytes of an Ed25519 secret seed key.
pub const KEY_BYTE_LENGTH: usize = 32;

/// Length in bytes of an Ed25519 keypair (secret + public).
pub const KEYPAIR_BYTE_LENGTH: usize = 64;

/// Errors that can occur during identity key management.
#[derive(Error, Debug)]
pub enum IdentityError {
    /// Failed to resolve standard XDG data directory.
    #[error("Failed to determine default data directory")]
    DirectoryResolutionFailed,

    /// I/O error occurred during file operations.
    #[error("I/O error at {0}: {1}")]
    Io(PathBuf, #[source] std::io::Error),

    /// File permissions do not meet security requirements.
    #[error("Invalid permissions on {path}: expected mode {expected:04o}, found {found:04o}")]
    InvalidPermissions {
        path: PathBuf,
        expected: u32,
        found: u32,
    },

    /// The key file does not contain the expected number of bytes.
    #[error("Invalid identity key size: expected {expected} bytes, found {found} bytes")]
    InvalidKeyLength { expected: usize, found: usize },

    /// Ed25519 cryptographic operation failure.
    #[error("Ed25519 cryptographic error: {0}")]
    Dalek(#[from] ed25519_dalek::SignatureError),
}

/// Returns the default path for the identity key file:
/// `$XDG_DATA_HOME/omnilink/identity.key` (or `~/.local/share/omnilink/identity.key`).
pub fn default_key_path() -> Result<PathBuf, IdentityError> {
    let proj = ProjectDirs::from("", "", "omnilink")
        .ok_or(IdentityError::DirectoryResolutionFailed)?;
    Ok(proj.data_dir().join(KEY_FILE_NAME))
}

/// Validates file permissions on Unix.
/// Returns Ok(()) if mode & 0o777 == 0o600.
#[cfg(unix)]
pub fn validate_file_permissions(path: &Path) -> Result<(), IdentityError> {
    use std::os::unix::fs::PermissionsExt;
    let meta = fs::metadata(path).map_err(|e| IdentityError::Io(path.to_path_buf(), e))?;
    let mode = meta.permissions().mode() & 0o777;
    if mode != 0o600 {
        return Err(IdentityError::InvalidPermissions {
            path: path.to_path_buf(),
            expected: 0o600,
            found: mode,
        });
    }
    Ok(())
}

/// Validates file permissions on non-Unix platforms (no-op).
#[cfg(not(unix))]
pub fn validate_file_permissions(_path: &Path) -> Result<(), IdentityError> {
    Ok(())
}

/// Validates directory permissions on Unix.
/// Returns Ok(()) if mode & 0o777 == 0o700.
#[cfg(unix)]
pub fn validate_directory_permissions(path: &Path) -> Result<(), IdentityError> {
    use std::os::unix::fs::PermissionsExt;
    let meta = fs::metadata(path).map_err(|e| IdentityError::Io(path.to_path_buf(), e))?;
    let mode = meta.permissions().mode() & 0o777;
    if mode != 0o700 {
        return Err(IdentityError::InvalidPermissions {
            path: path.to_path_buf(),
            expected: 0o700,
            found: mode,
        });
    }
    Ok(())
}

/// Validates directory permissions on non-Unix platforms (no-op).
#[cfg(not(unix))]
pub fn validate_directory_permissions(_path: &Path) -> Result<(), IdentityError> {
    Ok(())
}

/// Saves an Ed25519 SigningKey to the specified path.
/// Enforces directory permissions 0700 and file permissions 0600 on Unix.
pub fn save_identity(path: &Path, signing_key: &SigningKey) -> Result<(), IdentityError> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = fs::DirBuilder::new();
                builder.recursive(true);
                builder.mode(0o700);
                builder
                    .create(parent)
                    .map_err(|e| IdentityError::Io(parent.to_path_buf(), e))?;
            }
            #[cfg(not(unix))]
            {
                fs::create_dir_all(parent)
                    .map_err(|e| IdentityError::Io(parent.to_path_buf(), e))?;
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
                .map_err(|e| IdentityError::Io(parent.to_path_buf(), e))?;
        }
    }

    let key_bytes = signing_key.to_bytes();

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::fs::PermissionsExt;

        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| IdentityError::Io(path.to_path_buf(), e))?;

        file.write_all(&key_bytes)
            .map_err(|e| IdentityError::Io(path.to_path_buf(), e))?;
        file.flush()
            .map_err(|e| IdentityError::Io(path.to_path_buf(), e))?;

        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|e| IdentityError::Io(path.to_path_buf(), e))?;
    }

    #[cfg(not(unix))]
    {
        fs::write(path, key_bytes).map_err(|e| IdentityError::Io(path.to_path_buf(), e))?;
    }

    Ok(())
}

/// Loads an Ed25519 SigningKey from the specified path.
/// Enforces mode 0600 permissions on Unix before reading.
pub fn load_identity(path: &Path) -> Result<SigningKey, IdentityError> {
    #[cfg(unix)]
    validate_file_permissions(path)?;

    let bytes = fs::read(path).map_err(|e| IdentityError::Io(path.to_path_buf(), e))?;
    if bytes.len() == KEY_BYTE_LENGTH {
        let key_arr: [u8; KEY_BYTE_LENGTH] = bytes.as_slice().try_into().map_err(|_| {
            IdentityError::InvalidKeyLength {
                expected: KEY_BYTE_LENGTH,
                found: bytes.len(),
            }
        })?;
        Ok(SigningKey::from_bytes(&key_arr))
    } else if bytes.len() == KEYPAIR_BYTE_LENGTH {
        let key_arr: [u8; KEYPAIR_BYTE_LENGTH] = bytes.as_slice().try_into().map_err(|_| {
            IdentityError::InvalidKeyLength {
                expected: KEYPAIR_BYTE_LENGTH,
                found: bytes.len(),
            }
        })?;
        Ok(SigningKey::from_keypair_bytes(&key_arr)?)
    } else {
        Err(IdentityError::InvalidKeyLength {
            expected: KEY_BYTE_LENGTH,
            found: bytes.len(),
        })
    }
}

/// Loads the identity key from `custom_path` (or `default_key_path()` if None).
/// If the file does not exist, generates a new Ed25519 keypair using OS secure randomness
/// and persists it with mode 0600 permissions.
pub fn load_or_generate_identity(custom_path: Option<&Path>) -> Result<SigningKey, IdentityError> {
    let path = match custom_path {
        Some(p) => p.to_path_buf(),
        None => default_key_path()?,
    };

    if path.exists() {
        load_identity(&path)
    } else {
        let mut csprng = rand::rngs::OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        save_identity(&path, &signing_key)?;
        Ok(signing_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_generation() {
        let mut csprng = rand::rngs::OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let secret_bytes = signing_key.to_bytes();
        assert_eq!(secret_bytes.len(), 32);

        let verifying_key = signing_key.verifying_key();
        assert_eq!(verifying_key.to_bytes().len(), 32);

        let reconstructed = SigningKey::from_bytes(&secret_bytes);
        assert_eq!(signing_key.to_bytes(), reconstructed.to_bytes());
        assert_eq!(
            signing_key.verifying_key().to_bytes(),
            reconstructed.verifying_key().to_bytes()
        );
    }

    #[test]
    fn test_persistence_reload() {
        let temp_dir = std::env::temp_dir().join(format!(
            "omnilink-test-persist-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let key_path = temp_dir.join("identity.key");

        let mut csprng = rand::rngs::OsRng;
        let original_key = SigningKey::generate(&mut csprng);

        save_identity(&key_path, &original_key).expect("Failed to save identity");
        assert!(key_path.exists());

        let loaded_key = load_identity(&key_path).expect("Failed to load identity");
        assert_eq!(original_key.to_bytes(), loaded_key.to_bytes());
        assert_eq!(
            original_key.verifying_key().to_bytes(),
            loaded_key.verifying_key().to_bytes()
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_load_or_generate_creates_and_persists() {
        let temp_dir = std::env::temp_dir().join(format!(
            "omnilink-test-loadgen-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let key_path = temp_dir.join("identity.key");

        assert!(!key_path.exists());
        let generated_key =
            load_or_generate_identity(Some(&key_path)).expect("Failed to load or generate");
        assert!(key_path.exists());

        let reloaded_key =
            load_or_generate_identity(Some(&key_path)).expect("Failed on second call");
        assert_eq!(generated_key.to_bytes(), reloaded_key.to_bytes());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    #[cfg(unix)]
    fn test_0600_file_permission_validation() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = std::env::temp_dir().join(format!(
            "omnilink-test-perms-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let key_path = temp_dir.join("identity.key");

        let mut csprng = rand::rngs::OsRng;
        let original_key = SigningKey::generate(&mut csprng);
        save_identity(&key_path, &original_key).expect("Failed to save identity");

        // Verify initial permissions are mode 0600
        validate_file_permissions(&key_path).expect("Expected valid 0600 permissions");
        validate_directory_permissions(&temp_dir).expect("Expected valid 0700 permissions");

        // Mutate file permissions to 0644 (too open)
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o644))
            .expect("Failed to alter permissions");

        let val_res = validate_file_permissions(&key_path);
        assert!(val_res.is_err());
        match val_res {
            Err(IdentityError::InvalidPermissions {
                expected, found, ..
            }) => {
                assert_eq!(expected, 0o600);
                assert_eq!(found, 0o644);
            }
            other => panic!("Unexpected result: {:?}", other),
        }

        // Loading must also fail due to invalid permissions
        let load_res = load_identity(&key_path);
        assert!(load_res.is_err());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    #[cfg(unix)]
    fn test_0700_directory_permission_validation() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = std::env::temp_dir().join(format!(
            "omnilink-test-dirperms-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).expect("Failed to create dir");
        fs::set_permissions(&temp_dir, fs::Permissions::from_mode(0o755))
            .expect("Failed to set 0755");

        let val_res = validate_directory_permissions(&temp_dir);
        assert!(val_res.is_err());
        match val_res {
            Err(IdentityError::InvalidPermissions {
                expected, found, ..
            }) => {
                assert_eq!(expected, 0o700);
                assert_eq!(found, 0o755);
            }
            other => panic!("Unexpected result: {:?}", other),
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_invalid_key_length() {
        let temp_dir = std::env::temp_dir().join(format!(
            "omnilink-test-badlen-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let key_path = temp_dir.join("identity.key");

        fs::create_dir_all(&temp_dir).expect("Failed to create dir");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&temp_dir, fs::Permissions::from_mode(0o700));
        }

        // Write 16 bytes instead of 32
        fs::write(&key_path, [0u8; 16]).expect("Failed to write short key");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600));
        }

        let load_res = load_identity(&key_path);
        assert!(load_res.is_err());
        match load_res {
            Err(IdentityError::InvalidKeyLength { expected, found }) => {
                assert_eq!(expected, 32);
                assert_eq!(found, 16);
            }
            other => panic!("Unexpected result: {:?}", other),
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
