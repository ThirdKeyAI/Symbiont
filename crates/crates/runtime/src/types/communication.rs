//! Communication system types and data structures

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

use super::{AgentId, MessageId, RequestId};

/// Secure message structure for inter-agent communication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecureMessage {
    pub id: MessageId,
    pub sender: AgentId,
    pub recipient: Option<AgentId>, // None for broadcast
    pub topic: Option<String>,      // For pub/sub
    pub payload: EncryptedPayload,
    pub signature: MessageSignature,
    pub timestamp: SystemTime,
    pub ttl: Duration,
    pub message_type: MessageType,
}

/// Types of messages in the communication system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageType {
    Direct(AgentId),
    Publish(String),
    Subscribe(String),
    Broadcast,
    Request(RequestId),
    Response(RequestId),
}

/// Encrypted message payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedPayload {
    pub data: Bytes,
    pub encryption_algorithm: EncryptionAlgorithm,
    pub nonce: Vec<u8>,
}

/// Supported encryption algorithms
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EncryptionAlgorithm {
    #[default]
    Aes256Gcm,
    ChaCha20Poly1305,
    None, // For testing or non-sensitive data
}

/// Message signature for integrity verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageSignature {
    pub signature: Vec<u8>,
    pub algorithm: SignatureAlgorithm,
    pub public_key: Vec<u8>,
}

/// Supported signature algorithms
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SignatureAlgorithm {
    #[default]
    Ed25519,
    EcdsaP256,
    None, // For testing or non-critical messages
}

/// Communication channel handle
#[derive(Debug, Clone)]
pub struct ChannelHandle {
    pub id: String,
    pub agent_id: AgentId,
    pub channel_type: ChannelType,
    pub created_at: SystemTime,
}

/// Types of communication channels
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelType {
    Direct,
    Broadcast,
    Topic(String),
}

/// Message routing table entry
#[derive(Debug, Clone)]
pub struct RouteEntry {
    pub destination: AgentId,
    pub channel: ChannelHandle,
    pub last_used: SystemTime,
    pub message_count: u64,
}

/// Dead letter queue for undeliverable messages
#[derive(Debug, Clone)]
pub struct DeadLetterQueue {
    pub messages: Vec<DeadLetterMessage>,
    pub max_size: usize,
}

impl DeadLetterQueue {
    pub fn new(max_size: usize) -> Self {
        Self {
            messages: Vec::new(),
            max_size,
        }
    }

    pub fn add_message(&mut self, message: SecureMessage, reason: DeadLetterReason) {
        if self.messages.len() >= self.max_size {
            self.messages.remove(0); // Remove oldest message
        }

        self.messages.push(DeadLetterMessage {
            original_message: message,
            reason,
            timestamp: SystemTime::now(),
        });
    }
}

/// Message that couldn't be delivered
#[derive(Debug, Clone)]
pub struct DeadLetterMessage {
    pub original_message: SecureMessage,
    pub reason: DeadLetterReason,
    pub timestamp: SystemTime,
}

/// Reasons why a message ended up in the dead letter queue
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeadLetterReason {
    RecipientNotFound,
    RecipientUnavailable,
    MessageExpired,
    PolicyViolation(String),
    EncryptionFailure,
    SignatureVerificationFailure,
    MessageTooLarge,
    RateLimitExceeded,
    QueueFull,
    AgentNotFound,
    Expired,
}

/// Message delivery guarantees
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DeliveryGuarantee {
    /// Best effort delivery, no guarantees
    AtMostOnce,
    /// Guaranteed delivery with possible duplicates
    #[default]
    AtLeastOnce,
    /// Guaranteed single delivery (for critical messages)
    ExactlyOnce,
}

/// Rate limiting configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    pub messages_per_second: u32,
    pub burst_size: u32,
    pub window_duration: Duration,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            messages_per_second: 100,
            burst_size: 200,
            window_duration: Duration::from_secs(60),
        }
    }
}

/// Message security configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageSecurity {
    pub encryption_enabled: bool,
    pub signature_required: bool,
    pub key_rotation_interval: Duration,
    pub max_message_size: usize,
    pub rate_limiting: RateLimitConfig,
}

impl Default for MessageSecurity {
    fn default() -> Self {
        Self {
            encryption_enabled: true,
            signature_required: true,
            key_rotation_interval: Duration::from_secs(86400), // 24 hours
            max_message_size: 1024 * 1024,                     // 1MB
            rate_limiting: RateLimitConfig::default(),
        }
    }
}

/// Communication subsystem configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunicationConfig {
    pub max_concurrent_connections: usize,
    pub message_buffer_size: usize,
    pub connection_timeout: Duration,
    pub message_timeout: Duration,
    pub security: MessageSecurity,
    pub dead_letter_queue_size: usize,
}

impl Default for CommunicationConfig {
    fn default() -> Self {
        Self {
            max_concurrent_connections: 10000,
            message_buffer_size: 1000,
            connection_timeout: Duration::from_secs(30),
            message_timeout: Duration::from_secs(60),
            security: MessageSecurity::default(),
            dead_letter_queue_size: 1000,
        }
    }
}

/// Communication channels for an execution context
#[derive(Debug, Clone, Default)]
pub struct CommunicationChannels {
    pub direct_channel: Option<ChannelHandle>,
    pub broadcast_channel: Option<ChannelHandle>,
    pub subscribed_topics: Vec<String>,
}
