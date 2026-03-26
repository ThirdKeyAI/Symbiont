//! Priority queue implementation for agent scheduling.
//!
//! Uses a BinaryHeap for O(log n) push/pop with a HashMap for O(1)
//! membership checks. The index tracks presence only — not heap positions,
//! which are unstable across operations.

use std::collections::BinaryHeap;
use std::collections::HashMap;

use crate::types::AgentId;

/// Priority queue for scheduled tasks.
///
/// Provides O(log n) push/pop, O(1) contains, and O(n) remove-by-id.
/// The remove-by-id cost is acceptable because it's infrequent relative
/// to push/pop in the scheduler hot path.
#[derive(Debug)]
pub struct PriorityQueue<T> {
    heap: BinaryHeap<T>,
    /// Tracks which agent IDs are in the queue (presence only, not position).
    members: HashMap<AgentId, ()>,
}

impl<T> PriorityQueue<T>
where
    T: Ord + Clone + HasAgentId,
{
    /// Create a new priority queue.
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
            members: HashMap::new(),
        }
    }

    /// Create a new priority queue with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            heap: BinaryHeap::with_capacity(capacity),
            members: HashMap::with_capacity(capacity),
        }
    }

    /// Add an item to the queue. O(log n).
    pub fn push(&mut self, item: T) {
        let agent_id = item.agent_id();
        self.members.insert(agent_id, ());
        self.heap.push(item);
    }

    /// Remove and return the highest priority item. O(log n).
    pub fn pop(&mut self) -> Option<T> {
        let item = self.heap.pop()?;
        self.members.remove(&item.agent_id());
        Some(item)
    }

    /// Remove a specific item by agent ID. O(n) — acceptable for
    /// infrequent cancellations.
    pub fn remove(&mut self, agent_id: &AgentId) -> Option<T> {
        self.members.remove(agent_id)?;

        // Drain heap, extract target, rebuild.
        let items: Vec<T> = self.heap.drain().collect();
        let mut removed = None;

        // Rebuild heap excluding the target. Pre-allocate to avoid
        // repeated growth.
        let mut remaining = Vec::with_capacity(items.len() - 1);
        for item in items {
            if removed.is_none() && &item.agent_id() == agent_id {
                removed = Some(item);
            } else {
                remaining.push(item);
            }
        }
        self.heap = remaining.into_iter().collect();

        removed
    }

    /// Check if the queue contains an agent. O(1).
    pub fn contains(&self, agent_id: &AgentId) -> bool {
        self.members.contains_key(agent_id)
    }

    /// Find an item by agent ID. O(n) — use sparingly.
    pub fn find(&self, agent_id: &AgentId) -> Option<&T> {
        if !self.members.contains_key(agent_id) {
            return None;
        }
        self.heap.iter().find(|item| &item.agent_id() == agent_id)
    }

    /// Get the number of items in the queue.
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Check if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// Peek at the highest priority item without removing it. O(1).
    pub fn peek(&self) -> Option<&T> {
        self.heap.peek()
    }

    /// Clear all items from the queue.
    pub fn clear(&mut self) {
        self.heap.clear();
        self.members.clear();
    }

    /// Get all items as a vector (for debugging/monitoring).
    pub fn to_vec(&self) -> Vec<T> {
        self.heap.iter().cloned().collect()
    }
}

impl<T> Default for PriorityQueue<T>
where
    T: Ord + Clone + HasAgentId,
{
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for items that have an agent ID.
pub trait HasAgentId {
    fn agent_id(&self) -> AgentId;
}

impl HasAgentId for super::ScheduledTask {
    fn agent_id(&self) -> AgentId {
        self.agent_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::ScheduledTask;
    use crate::types::{
        AgentConfig, AgentId, ExecutionMode, Priority, ResourceLimits, SecurityTier,
    };
    use std::collections::HashMap;

    fn create_test_task(priority: Priority) -> ScheduledTask {
        let agent_id = AgentId::new();
        let config = AgentConfig {
            id: agent_id,
            name: "test".to_string(),
            dsl_source: "test".to_string(),
            execution_mode: ExecutionMode::Ephemeral,
            security_tier: SecurityTier::Tier1,
            resource_limits: ResourceLimits::default(),
            capabilities: vec![],
            policies: vec![],
            metadata: HashMap::new(),
            priority,
        };
        ScheduledTask::new(config)
    }

    #[test]
    fn test_priority_queue_ordering() {
        let mut queue = PriorityQueue::new();

        let low_task = create_test_task(Priority::Low);
        let high_task = create_test_task(Priority::High);
        let normal_task = create_test_task(Priority::Normal);

        queue.push(low_task);
        queue.push(high_task);
        queue.push(normal_task);

        assert_eq!(queue.pop().unwrap().priority, Priority::High);
        assert_eq!(queue.pop().unwrap().priority, Priority::Normal);
        assert_eq!(queue.pop().unwrap().priority, Priority::Low);
    }

    #[test]
    fn test_priority_queue_remove() {
        let mut queue = PriorityQueue::new();

        let task1 = create_test_task(Priority::High);
        let task2 = create_test_task(Priority::Normal);
        let task3 = create_test_task(Priority::Low);

        let agent_id2 = task2.agent_id;

        queue.push(task1);
        queue.push(task2);
        queue.push(task3);

        assert_eq!(queue.len(), 3);
        assert!(queue.contains(&agent_id2));

        let removed = queue.remove(&agent_id2);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().agent_id, agent_id2);
        assert_eq!(queue.len(), 2);
        assert!(!queue.contains(&agent_id2));
    }

    #[test]
    fn test_pop_maintains_membership() {
        let mut queue = PriorityQueue::new();
        let task = create_test_task(Priority::High);
        let id = task.agent_id;

        queue.push(task);
        assert!(queue.contains(&id));

        queue.pop();
        assert!(!queue.contains(&id));
    }

    #[test]
    fn test_remove_nonexistent() {
        let mut queue: PriorityQueue<ScheduledTask> = PriorityQueue::new();
        let fake_id = AgentId::new();
        assert!(queue.remove(&fake_id).is_none());
    }
}
