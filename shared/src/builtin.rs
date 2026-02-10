use std::{
    cmp::Eq,
    collections::HashMap,
    hash::Hash,
    sync::{LazyLock, RwLock},
};

pub type GlobalBuiltins<V, K = &'static str> = LazyLock<RwLock<Builtins<V, K>>>;

#[derive(Debug, Clone)]
pub struct Builtins<V, K = &'static str>
where
    K: Hash + Eq,
{
    maps: HashMap<K, V>,
}

impl<K: Hash + Eq, V> Default for Builtins<V, K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Hash + Eq, V> Builtins<V, K> {
    pub fn new() -> Self {
        Self {
            maps: HashMap::new(),
        }
    }
    pub fn new_global() -> GlobalBuiltins<V, K> {
        LazyLock::new(|| RwLock::new(Self::new()))
    }
    pub fn register(mut self, key: K, val: V) -> Self {
        self.maps.insert(key, val);
        self
    }
    pub fn get(&self, key: &K) -> Option<&V> {
        self.maps.get(key)
    }
}
