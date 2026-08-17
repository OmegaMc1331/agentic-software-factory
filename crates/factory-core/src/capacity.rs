use std::collections::HashMap;
use std::sync::Mutex;

/// Tracks in-flight work per agent across the factory so selection can prefer
/// the least-loaded member of a pool.
///
/// The factory executes workflows sequentially within a run, but may run
/// several workflows concurrently in separate threads. Capacity is therefore
/// shared and synchronized: every invocation that has been handed to an agent
/// increments the agent's load until it finishes.
#[derive(Debug, Default)]
pub struct AgentCapacity {
    load: Mutex<HashMap<String, u64>>,
}

impl AgentCapacity {
    pub fn new() -> AgentCapacity {
        AgentCapacity::default()
    }

    /// Records that an agent began an in-flight invocation and returns a guard
    /// that releases the slot when dropped.
    pub fn acquire(&self, agent: &str) -> LoadGuard<'_> {
        self.adjust(agent, 1);
        LoadGuard {
            capacity: self,
            agent: agent.to_string(),
            active: true,
        }
    }

    pub fn release(&self, agent: &str) {
        self.adjust(agent, 0);
    }

    /// Current in-flight count for an agent; `0` for agents never observed.
    pub fn inflight(&self, agent: &str) -> u64 {
        self.load
            .lock()
            .map(|load| load.get(agent).copied().unwrap_or(0))
            .unwrap_or(0)
    }

    fn adjust(&self, agent: &str, delta: u64) {
        if let Ok(mut load) = self.load.lock() {
            let count = load.entry(agent.to_string()).or_insert(0);
            if delta == 0 {
                *count = count.saturating_sub(1);
            } else {
                *count = count.saturating_add(delta);
            }
        }
    }
}

/// RAII guard that returns an agent's slot once the invocation finishes.
#[derive(Debug)]
pub struct LoadGuard<'a> {
    capacity: &'a AgentCapacity,
    agent: String,
    active: bool,
}

impl Drop for LoadGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            self.capacity.release(&self.agent);
        }
    }
}

/// Selects the least-loaded agent from a pool.
///
/// Among the pool members the agent with the fewest in-flight invocations is
/// chosen; ties fall back to round-robin using the supplied index so the choice
/// stays stable for otherwise identical pools.
pub fn select_agent_with_capacity<'a>(
    pool: &'a [String],
    index: usize,
    capacity: &AgentCapacity,
) -> Option<&'a String> {
    if pool.is_empty() {
        return None;
    }
    let min_load = pool
        .iter()
        .map(|agent| capacity.inflight(agent))
        .min()
        .unwrap_or(0);
    let candidates: Vec<usize> = pool
        .iter()
        .enumerate()
        .filter(|(_, agent)| capacity.inflight(agent) == min_load)
        .map(|(position, _)| position)
        .collect();
    Some(&pool[candidates[index % candidates.len()]])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn empty_pool_never_selects() {
        let capacity = AgentCapacity::new();
        let empty: Vec<String> = Vec::new();
        assert!(select_agent_with_capacity(&empty, 0, &capacity).is_none());
    }

    #[test]
    fn idle_pool_falls_back_to_round_robin() {
        let capacity = AgentCapacity::new();
        let members = pool(&["a", "b", "c"]);
        let picks: Vec<&str> = (0..7)
            .map(|index| {
                select_agent_with_capacity(&members, index, &capacity)
                    .unwrap()
                    .as_str()
            })
            .collect();
        assert_eq!(picks, ["a", "b", "c", "a", "b", "c", "a"]);
    }

    #[test]
    fn least_loaded_agent_wins() {
        let capacity = AgentCapacity::new();
        let members = pool(&["busy", "idle"]);
        let _guard = capacity.acquire("busy");
        assert_eq!(
            select_agent_with_capacity(&members, 0, &capacity).unwrap().as_str(),
            "idle"
        );
    }

    #[test]
    fn ties_round_robin_among_min_load() {
        let capacity = AgentCapacity::new();
        let members = pool(&["a", "b", "c"]);
        let _guard = capacity.acquire("a");
        let picks: Vec<&str> = (0..6)
            .map(|index| {
                select_agent_with_capacity(&members, index, &capacity)
                    .unwrap()
                    .as_str()
            })
            .collect();
        assert_eq!(picks, ["b", "c", "b", "c", "b", "c"]);
    }

    #[test]
    fn guard_releases_the_slot_on_drop() {
        let capacity = AgentCapacity::new();
        assert_eq!(capacity.inflight("alone"), 0);
        {
            let _guard = capacity.acquire("alone");
            assert_eq!(capacity.inflight("alone"), 1);
        }
        assert_eq!(capacity.inflight("alone"), 0);
    }

    #[test]
    fn release_never_underflows() {
        let capacity = AgentCapacity::new();
        capacity.release("ghost");
        assert_eq!(capacity.inflight("ghost"), 0);
        capacity.acquire("ghost");
        capacity.release("ghost");
        capacity.release("ghost");
        assert_eq!(capacity.inflight("ghost"), 0);
    }
}