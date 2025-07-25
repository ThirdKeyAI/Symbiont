//! Priority queue implementation for agent scheduling

use std::collections::BinaryHeap;
use std::collections::HashMap;

use crate::types::AgentId;

/// Priority queue for scheduled tasks
#[derive(Debug)]
pub struct PriorityQueue<T> {
    heap: BinaryHeap<T>,
    index: HashMap<AgentId, usize>,
}

impl<T> PriorityQueue<T>
where
    T: Ord + Clone,
    T: HasAgentId,
{
    /// Create a new priority queue
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
            index: HashMap::new(),
        }
    }

    /// Add an item to the queue
    pub fn push(&mut self, item: T) {
        let agent_id = item.agent_id();
        self.index.insert(agent_id, self.heap.len());
        self.heap.push(item);
    }

    /// Remove and return the highest priority item
    pub fn pop(&mut self) -> Option<T> {
        if let Some(item) = self.heap.pop() {
            let agent_id = item.agent_id();
            self.index.remove(&agent_id);
            // Rebuild index after pop
            self.rebuild_index();
            Some(item)
        } else {
            None
        }
    }

    /// Remove a specific item by agent ID
    pub fn remove(&mut self, agent_id: &AgentId) -> Option<T> {
        if self.index.remove(agent_id).is_some() {
            // Convert heap to vector, remove item, and rebuild heap
            let mut items: Vec<T> = self.heap.drain().collect();
            let mut removed_item = None;

            items.retain(|item| {
                if &item.agent_id() == agent_id {
                    removed_item = Some(item.clone());
                    false
                } else {
                    true
                }
            });

            // Rebuild heap and index
            self.heap = items.into_iter().collect();
            self.rebuild_index();

            removed_item
        } else {
            None
        }
    }

    /// Check if the queue contains an agent
    pub fn contains(&self, agent_id: &AgentId) -> bool {
        self.index.contains_key(agent_id)
    }

    /// Get the number of items in the queue
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Check if the queue is empty
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// Peek at the highest priority item without removing it
    pub fn peek(&self) -> Option<&T> {
        self.heap.peek()
    }

    /// Rebuild the index after heap operations
    fn rebuild_index(&mut self) {
        self.index.clear();
        for (idx, item) in self.heap.iter().enumerate() {
            self.index.insert(item.agent_id(), idx);
        }
    }

    /// Clear all items from the queue
    pub fn clear(&mut self) {
        self.heap.clear();
        self.index.clear();
    }

    /// Get all items as a vector (for debugging/monitoring)
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

/// Trait for items that have an agent ID
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

        queue.push(low_task.clone());
        queue.push(high_task.clone());
        queue.push(normal_task.clone());

        // Should pop in priority order: High, Normal, Low
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
}
