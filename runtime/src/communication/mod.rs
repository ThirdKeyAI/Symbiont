//! Agent Communication Bus
//! 
//! Secure messaging system for inter-agent communication

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use async_trait::async_trait;
use parking_lot::RwLock;
use tokio::sync::{mpsc, Notify};
use tokio::time::interval;

use crate::types::*;

/// Communication bus trait
#[async_trait]
pub trait CommunicationBus {
    /// Send a message to an agent
    async fn send_message(&self, message: SecureMessage) -> Result<MessageId, CommunicationError>;
    
    /// Receive messages for an agent
    async fn receive_messages(&self, agent_id: AgentId) -> Result<Vec<SecureMessage>, CommunicationError>;
    
    /// Subscribe to a topic
    async fn subscribe(&self, agent_id: AgentId, topic: String) -> Result<(), CommunicationError>;
    
    /// Unsubscribe from a topic
    async fn unsubscribe(&self, agent_id: AgentId, topic: String) -> Result<(), CommunicationError>;
    
    /// Publish a message to a topic
    async fn publish(&self, topic: String, message: SecureMessage) -> Result<(), CommunicationError>;
    
    /// Get message delivery status
    async fn get_delivery_status(&self, message_id: MessageId) -> Result<DeliveryStatus, CommunicationError>;
    
    /// Register an agent for communication
    async fn register_agent(&self, agent_id: AgentId) -> Result<(), CommunicationError>;
    
    /// Unregister an agent
    async fn unregister_agent(&self, agent_id: AgentId) -> Result<(), CommunicationError>;
    
    /// Shutdown the communication bus
    async fn shutdown(&self) -> Result<(), CommunicationError>;
}

/// Communication bus configuration
#[derive(Debug, Clone)]
pub struct CommunicationConfig {
    pub max_message_size: usize,
    pub message_ttl: Duration,
    pub max_queue_size: usize,
    pub delivery_timeout: Duration,
    pub retry_attempts: u32,
    pub enable_encryption: bool,
    pub enable_compression: bool,
    pub dead_letter_queue_size: usize,
}

impl Default for CommunicationConfig {
    fn default() -> Self {
        Self {
            max_message_size: 1024 * 1024, // 1MB
            message_ttl: Duration::from_secs(3600), // 1 hour
            max_queue_size: 10000,
            delivery_timeout: Duration::from_secs(30),
            retry_attempts: 3,
            enable_encryption: true,
            enable_compression: true,
            dead_letter_queue_size: 1000,
        }
    }
}

/// Default implementation of the communication bus
pub struct DefaultCommunicationBus {
    config: CommunicationConfig,
    message_queues: Arc<RwLock<HashMap<AgentId, MessageQueue>>>,
    subscriptions: Arc<RwLock<HashMap<String, Vec<AgentId>>>>,
    message_tracker: Arc<RwLock<HashMap<MessageId, MessageTracker>>>,
    dead_letter_queue: Arc<RwLock<DeadLetterQueue>>,
    event_sender: mpsc::UnboundedSender<CommunicationEvent>,
    shutdown_notify: Arc<Notify>,
    is_running: Arc<RwLock<bool>>,
}

impl DefaultCommunicationBus {
    /// Create a new communication bus
    pub async fn new(config: CommunicationConfig) -> Result<Self, CommunicationError> {
        let message_queues = Arc::new(RwLock::new(HashMap::new()));
        let subscriptions = Arc::new(RwLock::new(HashMap::new()));
        let message_tracker = Arc::new(RwLock::new(HashMap::new()));
        let dead_letter_queue = Arc::new(RwLock::new(DeadLetterQueue::new(config.dead_letter_queue_size)));
        let (event_sender, event_receiver) = mpsc::unbounded_channel();
        let shutdown_notify = Arc::new(Notify::new());
        let is_running = Arc::new(RwLock::new(true));

        let bus = Self {
            config,
            message_queues,
            subscriptions,
            message_tracker,
            dead_letter_queue,
            event_sender,
            shutdown_notify,
            is_running,
        };

        // Start background tasks
        bus.start_event_loop(event_receiver).await;
        bus.start_cleanup_loop().await;

        Ok(bus)
    }

    /// Start the event processing loop
    async fn start_event_loop(&self, mut event_receiver: mpsc::UnboundedReceiver<CommunicationEvent>) {
        let message_queues = self.message_queues.clone();
        let subscriptions = self.subscriptions.clone();
        let message_tracker = self.message_tracker.clone();
        let dead_letter_queue = self.dead_letter_queue.clone();
        let shutdown_notify = self.shutdown_notify.clone();
        let config = self.config.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    event = event_receiver.recv() => {
                        if let Some(event) = event {
                            Self::process_communication_event(
                                event,
                                &message_queues,
                                &subscriptions,
                                &message_tracker,
                                &dead_letter_queue,
                                &config,
                            ).await;
                        } else {
                            break;
                        }
                    }
                    _ = shutdown_notify.notified() => {
                        break;
                    }
                }
            }
        });
    }

    /// Start the cleanup loop for expired messages
    async fn start_cleanup_loop(&self) {
        let message_queues = self.message_queues.clone();
        let message_tracker = self.message_tracker.clone();
        let dead_letter_queue = self.dead_letter_queue.clone();
        let shutdown_notify = self.shutdown_notify.clone();
        let is_running = self.is_running.clone();
        let message_ttl = self.config.message_ttl;

        tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(60)); // Cleanup every minute
            
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if !*is_running.read() {
                            break;
                        }

                        Self::cleanup_expired_messages(&message_queues, &message_tracker, &dead_letter_queue, message_ttl).await;
                    }
                    _ = shutdown_notify.notified() => {
                        break;
                    }
                }
            }
        });
    }

    /// Process a communication event
    async fn process_communication_event(
        event: CommunicationEvent,
        message_queues: &Arc<RwLock<HashMap<AgentId, MessageQueue>>>,
        subscriptions: &Arc<RwLock<HashMap<String, Vec<AgentId>>>>,
        message_tracker: &Arc<RwLock<HashMap<MessageId, MessageTracker>>>,
        dead_letter_queue: &Arc<RwLock<DeadLetterQueue>>,
        config: &CommunicationConfig,
    ) {
        match event {
            CommunicationEvent::MessageSent { message } => {
                let recipient = message.recipient;
                let message_id = message.id;
                
                // Add to message tracker
                message_tracker.write().insert(message_id, MessageTracker::new(message.clone()));
                
                // Try to deliver the message
                let mut queues = message_queues.write();
                if let Some(recipient_id) = recipient {
                    if let Some(queue) = queues.get_mut(&recipient_id) {
                        if queue.can_accept_message(config) {
                            queue.add_message(message);
                            
                            // Update delivery status
                            if let Some(tracker) = message_tracker.write().get_mut(&message_id) {
                                tracker.status = DeliveryStatus::Delivered;
                                tracker.delivered_at = Some(SystemTime::now());
                            }
                            
                            tracing::debug!("Message {} delivered to agent {}", message_id, recipient_id);
                        } else {
                            // Queue is full, send to dead letter queue
                            dead_letter_queue.write().add_message(message, DeadLetterReason::QueueFull);
                            
                            if let Some(tracker) = message_tracker.write().get_mut(&message_id) {
                                tracker.status = DeliveryStatus::Failed;
                                tracker.failure_reason = Some("Queue full".to_string());
                            }
                            
                            tracing::warn!("Message {} failed to deliver: queue full for agent {}", message_id, recipient_id);
                        }
                    } else {
                        // Agent not registered
                        dead_letter_queue.write().add_message(message, DeadLetterReason::AgentNotFound);
                        
                        if let Some(tracker) = message_tracker.write().get_mut(&message_id) {
                            tracker.status = DeliveryStatus::Failed;
                            tracker.failure_reason = Some("Agent not registered".to_string());
                        }
                        
                        tracing::warn!("Message {} failed to deliver: agent {:?} not registered", message_id, recipient);
                    }
                } else {
                    // Agent not registered
                    dead_letter_queue.write().add_message(message, DeadLetterReason::AgentNotFound);
                    
                    if let Some(tracker) = message_tracker.write().get_mut(&message_id) {
                        tracker.status = DeliveryStatus::Failed;
                        tracker.failure_reason = Some("Agent not registered".to_string());
                    }
                    
                    tracing::warn!("Message {} failed to deliver: agent {:?} not registered", message_id, recipient);
                }
            }
            CommunicationEvent::TopicPublished { topic, message } => {
                let subscribers = subscriptions.read().get(&topic).cloned().unwrap_or_default();
                let subscriber_count = subscribers.len();
                
                for subscriber in &subscribers {
                    let mut subscriber_message = message.clone();
                    subscriber_message.recipient = Some(*subscriber);
                    subscriber_message.id = MessageId::new();
                    
                    // Send to each subscriber
                    Box::pin(Self::process_communication_event(
                        CommunicationEvent::MessageSent { message: subscriber_message },
                        message_queues,
                        subscriptions,
                        message_tracker,
                        dead_letter_queue,
                        config,
                    )).await;
                }
                
                tracing::debug!("Published message to topic {} for {} subscribers", topic, subscriber_count);
            }
            CommunicationEvent::AgentRegistered { agent_id } => {
                message_queues.write().insert(agent_id, MessageQueue::new());
                tracing::info!("Registered agent {} for communication", agent_id);
            }
            CommunicationEvent::AgentUnregistered { agent_id } => {
                message_queues.write().remove(&agent_id);
                
                // Remove from all subscriptions
                let mut subs = subscriptions.write();
                for subscribers in subs.values_mut() {
                    subscribers.retain(|&id| id != agent_id);
                }
                
                tracing::info!("Unregistered agent {} from communication", agent_id);
            }
        }
    }

    /// Cleanup expired messages
    async fn cleanup_expired_messages(
        message_queues: &Arc<RwLock<HashMap<AgentId, MessageQueue>>>,
        message_tracker: &Arc<RwLock<HashMap<MessageId, MessageTracker>>>,
        dead_letter_queue: &Arc<RwLock<DeadLetterQueue>>,
        message_ttl: Duration,
    ) {
        let now = SystemTime::now();
        let mut expired_messages = Vec::new();
        
        // Find expired messages in queues and check for stale queues
        {
            let mut queues = message_queues.write();
            let mut stale_queues = 0;
            for queue in queues.values_mut() {
                let expired = queue.remove_expired_messages(now, message_ttl);
                expired_messages.extend(expired);
                
                // Check if queue itself is stale (no activity for extended period)
                if queue.is_stale(message_ttl * 3) {
                    stale_queues += 1;
                }
            }
            
            if stale_queues > 0 {
                tracing::debug!("Found {} stale message queues", stale_queues);
            }
        }
        
        // Move expired messages to dead letter queue
        {
            let mut dlq = dead_letter_queue.write();
            for message in expired_messages {
                dlq.add_message(message.clone(), DeadLetterReason::Expired);
                
                // Update tracker
                if let Some(tracker) = message_tracker.write().get_mut(&message.id) {
                    tracker.status = DeliveryStatus::Failed;
                    tracker.failure_reason = Some("Message expired".to_string());
                }
            }
        }
        
        // Cleanup old message trackers and check for retry candidates
        {
            let mut tracker = message_tracker.write();
            let mut retry_candidates = Vec::new();
            
            tracker.retain(|message_id, t| {
                let age = t.get_age();
                if age < message_ttl * 2 {
                    // Check if message should be retried
                    if t.should_retry(message_ttl) {
                        retry_candidates.push(*message_id);
                        
                        // Log details about the retry candidate
                        let msg = t.get_message();
                        tracing::debug!("Message {} eligible for retry: size={} bytes, age={:?}s, sender={}",
                                      message_id, t.get_message_size(), t.get_age().as_secs(), msg.sender);
                    }
                    true
                } else {
                    false
                }
            });
            
            // Log retry candidates for monitoring
            if !retry_candidates.is_empty() {
                tracing::debug!("Found {} messages eligible for retry", retry_candidates.len());
            }
        }
    }

    /// Send a communication event
    fn send_event(&self, event: CommunicationEvent) -> Result<(), CommunicationError> {
        self.event_sender.send(event)
            .map_err(|_| CommunicationError::EventProcessingFailed {
                reason: "Failed to send communication event".to_string(),
            })
    }
}

#[async_trait]
impl CommunicationBus for DefaultCommunicationBus {
    async fn send_message(&self, message: SecureMessage) -> Result<MessageId, CommunicationError> {
        if !*self.is_running.read() {
            return Err(CommunicationError::ShuttingDown);
        }

        // Validate message size
        if message.payload.data.len() > self.config.max_message_size {
            return Err(CommunicationError::MessageTooLarge {
                size: message.payload.data.len(),
                max_size: self.config.max_message_size,
            });
        }

        let message_id = message.id;
        
        self.send_event(CommunicationEvent::MessageSent { message })?;
        
        Ok(message_id)
    }

    async fn receive_messages(&self, agent_id: AgentId) -> Result<Vec<SecureMessage>, CommunicationError> {
        let mut queues = self.message_queues.write();
        if let Some(queue) = queues.get_mut(&agent_id) {
            Ok(queue.drain_messages())
        } else {
            Err(CommunicationError::AgentNotRegistered { agent_id })
        }
    }

    async fn subscribe(&self, agent_id: AgentId, topic: String) -> Result<(), CommunicationError> {
        let mut subscriptions = self.subscriptions.write();
        subscriptions.entry(topic.clone()).or_default().push(agent_id);
        
        tracing::info!("Agent {} subscribed to topic {}", agent_id, topic);
        Ok(())
    }

    async fn unsubscribe(&self, agent_id: AgentId, topic: String) -> Result<(), CommunicationError> {
        let mut subscriptions = self.subscriptions.write();
        if let Some(subscribers) = subscriptions.get_mut(&topic) {
            subscribers.retain(|&id| id != agent_id);
            if subscribers.is_empty() {
                subscriptions.remove(&topic);
            }
        }
        
        tracing::info!("Agent {} unsubscribed from topic {}", agent_id, topic);
        Ok(())
    }

    async fn publish(&self, topic: String, message: SecureMessage) -> Result<(), CommunicationError> {
        if !*self.is_running.read() {
            return Err(CommunicationError::ShuttingDown);
        }

        self.send_event(CommunicationEvent::TopicPublished { topic, message })?;
        Ok(())
    }

    async fn get_delivery_status(&self, message_id: MessageId) -> Result<DeliveryStatus, CommunicationError> {
        self.message_tracker.read().get(&message_id)
            .map(|tracker| tracker.status.clone())
            .ok_or(CommunicationError::MessageNotFound { message_id })
    }

    async fn register_agent(&self, agent_id: AgentId) -> Result<(), CommunicationError> {
        self.send_event(CommunicationEvent::AgentRegistered { agent_id })?;
        Ok(())
    }

    async fn unregister_agent(&self, agent_id: AgentId) -> Result<(), CommunicationError> {
        self.send_event(CommunicationEvent::AgentUnregistered { agent_id })?;
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), CommunicationError> {
        tracing::info!("Shutting down communication bus");
        
        *self.is_running.write() = false;
        self.shutdown_notify.notify_waiters();

        // Unregister all agents
        let agent_ids: Vec<AgentId> = self.message_queues.read().keys().copied().collect();
        
        for agent_id in agent_ids {
            if let Err(e) = self.unregister_agent(agent_id).await {
                tracing::error!("Failed to unregister agent {} during shutdown: {}", agent_id, e);
            }
        }

        Ok(())
    }
}

/// Message queue for an agent
#[derive(Debug, Clone)]
struct MessageQueue {
    messages: Vec<SecureMessage>,
    created_at: SystemTime,
}

impl MessageQueue {
    fn new() -> Self {
        Self {
            messages: Vec::new(),
            created_at: SystemTime::now(),
        }
    }

    fn can_accept_message(&self, config: &CommunicationConfig) -> bool {
        self.messages.len() < config.max_queue_size
    }

    fn add_message(&mut self, message: SecureMessage) {
        self.messages.push(message);
    }

    fn drain_messages(&mut self) -> Vec<SecureMessage> {
        std::mem::take(&mut self.messages)
    }

    fn remove_expired_messages(&mut self, now: SystemTime, ttl: Duration) -> Vec<SecureMessage> {
        let mut expired = Vec::new();
        
        self.messages.retain(|message| {
            let age = now.duration_since(message.timestamp).unwrap_or_default();
            if age > ttl {
                expired.push(message.clone());
                false
            } else {
                true
            }
        });
        
        expired
    }

    fn get_queue_age(&self) -> Duration {
        SystemTime::now().duration_since(self.created_at).unwrap_or_default()
    }

    fn is_stale(&self, max_age: Duration) -> bool {
        self.get_queue_age() > max_age
    }
}

/// Message tracker for delivery status
#[derive(Debug, Clone)]
struct MessageTracker {
    message: SecureMessage,
    status: DeliveryStatus,
    created_at: SystemTime,
    delivered_at: Option<SystemTime>,
    failure_reason: Option<String>,
}

impl MessageTracker {
    fn new(message: SecureMessage) -> Self {
        Self {
            message,
            status: DeliveryStatus::Pending,
            created_at: SystemTime::now(),
            delivered_at: None,
            failure_reason: None,
        }
    }

    /// Get the tracked message
    fn get_message(&self) -> &SecureMessage {
        &self.message
    }

    /// Get message size in bytes
    fn get_message_size(&self) -> usize {
        self.message.payload.data.len()
    }

    /// Get age of the tracking record
    fn get_age(&self) -> Duration {
        SystemTime::now().duration_since(self.created_at).unwrap_or_default()
    }

    /// Check if message should be retried
    fn should_retry(&self, max_age: Duration) -> bool {
        matches!(self.status, DeliveryStatus::Failed) && self.get_age() < max_age
    }
}

/// Delivery status for messages
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryStatus {
    Pending,
    Delivered,
    Failed,
    Expired,
}

/// Communication events for internal processing
#[derive(Debug, Clone)]
enum CommunicationEvent {
    MessageSent {
        message: SecureMessage,
    },
    TopicPublished {
        topic: String,
        message: SecureMessage,
    },
    AgentRegistered {
        agent_id: AgentId,
    },
    AgentUnregistered {
        agent_id: AgentId,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MessageType, EncryptedPayload};

    fn create_test_message(sender: AgentId, recipient: AgentId) -> SecureMessage {
        use crate::types::RequestId;
        SecureMessage {
            id: MessageId::new(),
            sender,
            recipient: Some(recipient),
            message_type: MessageType::Request(RequestId::new()),
            topic: Some("test".to_string()),
            payload: EncryptedPayload {
                data: b"test message".to_vec().into(),
                nonce: [0u8; 12].to_vec(),
                encryption_algorithm: EncryptionAlgorithm::Aes256Gcm,
            },
            signature: MessageSignature {
                signature: vec![0u8; 64],
                algorithm: SignatureAlgorithm::Ed25519,
                public_key: vec![0u8; 32],
            },
            ttl: Duration::from_secs(3600),
            timestamp: SystemTime::now(),
        }
    }

    #[tokio::test]
    async fn test_agent_registration() {
        let bus = DefaultCommunicationBus::new(CommunicationConfig::default()).await.unwrap();
        let agent_id = AgentId::new();

        let result = bus.register_agent(agent_id).await;
        assert!(result.is_ok());

        // Give the event loop time to process
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Should be able to receive messages now
        let messages = bus.receive_messages(agent_id).await;
        assert!(messages.is_ok());
    }

    #[tokio::test]
    async fn test_message_sending() {
        let bus = DefaultCommunicationBus::new(CommunicationConfig::default()).await.unwrap();
        let sender = AgentId::new();
        let recipient = AgentId::new();

        // Register both agents
        bus.register_agent(sender).await.unwrap();
        bus.register_agent(recipient).await.unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        // Send a message
        let message = create_test_message(sender, recipient);
        let message_id = bus.send_message(message).await.unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        // Check delivery status
        let status = bus.get_delivery_status(message_id).await.unwrap();
        assert_eq!(status, DeliveryStatus::Delivered);

        // Receive messages
        let messages = bus.receive_messages(recipient).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].sender, sender);
    }

    #[tokio::test]
    async fn test_topic_subscription() {
        let bus = DefaultCommunicationBus::new(CommunicationConfig::default()).await.unwrap();
        let publisher = AgentId::new();
        let subscriber1 = AgentId::new();
        let subscriber2 = AgentId::new();

        // Register agents
        bus.register_agent(publisher).await.unwrap();
        bus.register_agent(subscriber1).await.unwrap();
        bus.register_agent(subscriber2).await.unwrap();

        // Subscribe to topic
        let topic = "test_topic".to_string();
        bus.subscribe(subscriber1, topic.clone()).await.unwrap();
        bus.subscribe(subscriber2, topic.clone()).await.unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        // Publish a message
        let message = create_test_message(publisher, AgentId::new()); // Recipient will be overridden
        bus.publish(topic, message).await.unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        // Both subscribers should receive the message
        let messages1 = bus.receive_messages(subscriber1).await.unwrap();
        let messages2 = bus.receive_messages(subscriber2).await.unwrap();

        assert_eq!(messages1.len(), 1);
        assert_eq!(messages2.len(), 1);
        assert_eq!(messages1[0].sender, publisher);
        assert_eq!(messages2[0].sender, publisher);
    }

    #[tokio::test]
    async fn test_message_size_limit() {
        let config = CommunicationConfig {
            max_message_size: 100, // Very small limit
            ..Default::default()
        };

        let bus = DefaultCommunicationBus::new(config).await.unwrap();
        let sender = AgentId::new();
        let recipient = AgentId::new();

        bus.register_agent(sender).await.unwrap();
        bus.register_agent(recipient).await.unwrap();

        // Create a message that's too large
        let mut message = create_test_message(sender, recipient);
        message.payload.data = vec![0u8; 200].into(); // Larger than limit

        let result = bus.send_message(message).await;
        assert!(result.is_err());
        
        if let Err(CommunicationError::MessageTooLarge { size, max_size }) = result {
            assert_eq!(size, 200);
            assert_eq!(max_size, 100);
        } else {
            panic!("Expected MessageTooLarge error");
        }
    }

    #[tokio::test]
    async fn test_agent_unregistration() {
        let bus = DefaultCommunicationBus::new(CommunicationConfig::default()).await.unwrap();
        let agent_id = AgentId::new();

        // Register and then unregister
        bus.register_agent(agent_id).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        bus.unregister_agent(agent_id).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Should not be able to receive messages
        let result = bus.receive_messages(agent_id).await;
        assert!(result.is_err());
    }
}