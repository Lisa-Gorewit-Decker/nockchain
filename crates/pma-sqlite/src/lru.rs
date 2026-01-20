use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

#[derive(Debug)]
struct Entry<V> {
    value: V,
    gen: u64,
}

#[derive(Debug)]
pub struct LruCache<K, V> {
    capacity: usize,
    map: HashMap<K, Entry<V>>,
    order: VecDeque<(K, u64)>,
    tick: u64,
}

impl<K, V> LruCache<K, V>
where
    K: Clone + Eq + Hash,
{
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            map: HashMap::new(),
            order: VecDeque::new(),
            tick: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn get(&mut self, key: &K) -> Option<V>
    where
        V: Clone,
    {
        if !self.map.contains_key(key) {
            return None;
        }
        let gen = self.bump_gen();
        let entry = self.map.get_mut(key)?;
        entry.gen = gen;
        self.order.push_back((key.clone(), gen));
        Some(entry.value.clone())
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        if !self.map.contains_key(key) {
            return None;
        }
        let gen = self.bump_gen();
        let entry = self.map.get_mut(key)?;
        entry.gen = gen;
        self.order.push_back((key.clone(), gen));
        Some(&mut entry.value)
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        let gen = self.bump_gen();
        self.order.push_back((key.clone(), gen));
        let prev = self
            .map
            .insert(key, Entry { value, gen })
            .map(|entry| entry.value);
        self.evict_if_needed();
        prev
    }

    fn bump_gen(&mut self) -> u64 {
        self.tick = self.tick.wrapping_add(1);
        self.tick
    }

    fn evict_if_needed(&mut self) {
        while self.map.len() > self.capacity {
            if self.evict_one().is_none() {
                break;
            }
        }
    }

    fn evict_one(&mut self) -> Option<V> {
        while let Some((key, gen)) = self.order.pop_front() {
            let should_evict = self.map.get(&key).is_some_and(|entry| entry.gen == gen);
            if should_evict {
                return self.map.remove(&key).map(|entry| entry.value);
            }
        }
        None
    }
}
