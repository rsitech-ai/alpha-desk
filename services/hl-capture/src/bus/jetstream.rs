use std::{
    fs,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use async_nats::{
    ConnectOptions,
    jetstream::{self, context::ContextBuilder, message::PublishMessage},
};
use async_trait::async_trait;
use tokio::sync::Mutex;

use super::{
    CanonicalPublisher, PublicationAck, PublicationError, PublicationLedger, PublicationMessage,
};

const MAX_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_ACK_INFLIGHT: usize = 100_000;
const MAX_LEDGER_CAPACITY: usize = 10_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JetStreamConfig {
    server_url: String,
    authentication: JetStreamAuthentication,
    connect_timeout: Duration,
    acknowledgement_timeout: Duration,
    max_ack_inflight: usize,
    ledger_capacity: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JetStreamAuthentication {
    Anonymous,
    CredentialsFile(PathBuf),
    UserPasswordFile {
        username: String,
        password_path: PathBuf,
    },
}

impl JetStreamConfig {
    pub fn try_new(
        server_url: impl Into<String>,
        authentication: JetStreamAuthentication,
        connect_timeout: Duration,
        acknowledgement_timeout: Duration,
        max_ack_inflight: usize,
        ledger_capacity: usize,
    ) -> Result<Self, JetStreamConfigError> {
        let server_url = server_url.into();
        validate_server_url(&server_url)?;
        match &authentication {
            JetStreamAuthentication::Anonymous => {}
            JetStreamAuthentication::CredentialsFile(path) => {
                validate_credentials_path(path)?;
            }
            JetStreamAuthentication::UserPasswordFile {
                username,
                password_path,
            } => {
                validate_username(username)?;
                validate_credentials_path(password_path)?;
            }
        }
        if connect_timeout.is_zero() || connect_timeout > MAX_TIMEOUT {
            return Err(JetStreamConfigError::InvalidConnectTimeout);
        }
        if acknowledgement_timeout.is_zero() || acknowledgement_timeout > MAX_TIMEOUT {
            return Err(JetStreamConfigError::InvalidAcknowledgementTimeout);
        }
        if !(1..=MAX_ACK_INFLIGHT).contains(&max_ack_inflight) {
            return Err(JetStreamConfigError::InvalidMaxAckInflight);
        }
        if !(1..=MAX_LEDGER_CAPACITY).contains(&ledger_capacity) {
            return Err(JetStreamConfigError::InvalidLedgerCapacity);
        }
        Ok(Self {
            server_url,
            authentication,
            connect_timeout,
            acknowledgement_timeout,
            max_ack_inflight,
            ledger_capacity,
        })
    }
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum JetStreamConfigError {
    #[error("NATS server URL is invalid or contains inline credentials")]
    UnsafeServerUrl,
    #[error("NATS credentials path must be absolute and normalized")]
    UnsafeCredentialsPath,
    #[error("NATS authentication username is invalid")]
    InvalidUsername,
    #[error("NATS connection timeout is outside the supported bound")]
    InvalidConnectTimeout,
    #[error("JetStream acknowledgement timeout is outside the supported bound")]
    InvalidAcknowledgementTimeout,
    #[error("JetStream maximum in-flight acknowledgement count is outside the supported bound")]
    InvalidMaxAckInflight,
    #[error("publication identity ledger capacity is outside the supported bound")]
    InvalidLedgerCapacity,
}

impl JetStreamConfigError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::UnsafeServerUrl => "jetstream_config.unsafe_server_url",
            Self::UnsafeCredentialsPath => "jetstream_config.unsafe_credentials_path",
            Self::InvalidUsername => "jetstream_config.invalid_username",
            Self::InvalidConnectTimeout => "jetstream_config.invalid_connect_timeout",
            Self::InvalidAcknowledgementTimeout => {
                "jetstream_config.invalid_acknowledgement_timeout"
            }
            Self::InvalidMaxAckInflight => "jetstream_config.invalid_max_ack_inflight",
            Self::InvalidLedgerCapacity => "jetstream_config.invalid_ledger_capacity",
        }
    }
}

#[derive(Debug)]
pub struct JetStreamPublisher {
    context: jetstream::Context,
    ledger: Mutex<PublicationLedger>,
}

impl JetStreamPublisher {
    pub async fn connect(config: JetStreamConfig) -> Result<Self, PublicationError> {
        let options = match &config.authentication {
            JetStreamAuthentication::Anonymous => ConnectOptions::new(),
            JetStreamAuthentication::CredentialsFile(path) => {
                validate_secret_file(path).map_err(|_| PublicationError::TransportConnect)?;
                ConnectOptions::with_credentials_file(path)
                    .await
                    .map_err(|_| PublicationError::TransportConnect)?
            }
            JetStreamAuthentication::UserPasswordFile {
                username,
                password_path,
            } => {
                let password = read_secret_file(password_path)
                    .map_err(|_| PublicationError::TransportConnect)?;
                ConnectOptions::with_user_and_password(username.clone(), password)
            }
        }
        .name("alpha-desk-hl-capture")
        .connection_timeout(config.connect_timeout)
        .request_timeout(Some(config.acknowledgement_timeout));

        let client = options
            .connect(config.server_url.as_str())
            .await
            .map_err(|_| PublicationError::TransportConnect)?;
        let context = ContextBuilder::new()
            .timeout(config.acknowledgement_timeout)
            .ack_timeout(config.acknowledgement_timeout)
            .max_ack_inflight(config.max_ack_inflight)
            .build(client);
        let ledger = PublicationLedger::new(config.ledger_capacity)?;
        Ok(Self {
            context,
            ledger: Mutex::new(ledger),
        })
    }
}

#[async_trait]
impl CanonicalPublisher for JetStreamPublisher {
    async fn publish(
        &self,
        message: &PublicationMessage,
    ) -> Result<PublicationAck, PublicationError> {
        self.ledger.lock().await.record(message)?;

        let publish = PublishMessage::build()
            .payload(message.payload.clone())
            .message_id(message.message_id())
            .expected_stream(message.stream())
            .header("Alpha-Desk-Schema", message.schema_version())
            .header("Alpha-Desk-Chain", message.chain_id().as_str())
            .header(
                "Alpha-Desk-Block-Height",
                message.block_height().get().to_string(),
            )
            .header(
                "Alpha-Desk-Block-Hash",
                hex::encode(message.canonical_block_hash()),
            )
            .header("Alpha-Desk-Archive-Receipt", message.archive_receipt_id())
            .header(
                "Alpha-Desk-Archive-Manifest-SHA256",
                hex::encode(message.archive_manifest_sha256()),
            )
            .header(
                "Alpha-Desk-Publication-SHA256",
                hex::encode(message.publication_sha256()),
            );

        let acknowledgement = self
            .context
            .send_publish(message.subject().as_str(), publish)
            .await
            .map_err(|_| PublicationError::TransportPublish)?
            .await
            .map_err(|_| PublicationError::TransportAck)?;

        PublicationAck::try_new(
            message,
            acknowledgement.stream,
            acknowledgement.sequence,
            acknowledgement.duplicate,
        )
    }
}

fn validate_server_url(value: &str) -> Result<(), JetStreamConfigError> {
    let valid_scheme = value.starts_with("nats://") || value.starts_with("tls://");
    if !valid_scheme
        || value.trim() != value
        || value.contains('@')
        || value.chars().any(char::is_control)
    {
        return Err(JetStreamConfigError::UnsafeServerUrl);
    }
    let authority = value
        .split_once("://")
        .map(|(_, authority)| authority)
        .unwrap_or_default();
    if authority.is_empty()
        || authority.contains('/')
        || authority.contains('?')
        || authority.contains('#')
    {
        return Err(JetStreamConfigError::UnsafeServerUrl);
    }
    Ok(())
}

fn validate_credentials_path(path: &Path) -> Result<(), JetStreamConfigError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        Err(JetStreamConfigError::UnsafeCredentialsPath)
    } else {
        Ok(())
    }
}

fn validate_username(value: &str) -> Result<(), JetStreamConfigError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > 128
        || value.chars().any(char::is_control)
    {
        Err(JetStreamConfigError::InvalidUsername)
    } else {
        Ok(())
    }
}

fn validate_secret_file(path: &Path) -> Result<(), std::io::Error> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 16_384 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "unsafe NATS secret file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "NATS secret file permissions are too broad",
            ));
        }
    }
    Ok(())
}

fn read_secret_file(path: &Path) -> Result<String, std::io::Error> {
    validate_secret_file(path)?;
    let bytes = fs::read(path)?;
    let value = String::from_utf8(bytes)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "NATS secret UTF-8"))?;
    let value = value.strip_suffix('\n').unwrap_or(&value);
    let value = value.strip_suffix('\r').unwrap_or(value);
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid NATS secret",
        ));
    }
    Ok(value.to_owned())
}

#[cfg(all(test, unix))]
mod tests {
    use std::{
        fs,
        os::unix::fs::{PermissionsExt as _, symlink},
    };

    use tempfile::tempdir;

    use super::read_secret_file;

    #[test]
    fn secret_file_must_be_regular_private_and_bounded() {
        let directory = tempdir().expect("temporary directory");
        let secret = directory.path().join("secret");
        fs::write(&secret, b"selected-secret\n").expect("write secret");
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o600)).expect("set private mode");
        assert_eq!(
            read_secret_file(&secret).expect("private secret"),
            "selected-secret"
        );

        fs::set_permissions(&secret, fs::Permissions::from_mode(0o640)).expect("set broad mode");
        assert!(read_secret_file(&secret).is_err());

        fs::set_permissions(&secret, fs::Permissions::from_mode(0o600))
            .expect("restore private mode");
        let link = directory.path().join("secret-link");
        symlink(&secret, &link).expect("create secret symlink");
        assert!(read_secret_file(&link).is_err());
    }
}
