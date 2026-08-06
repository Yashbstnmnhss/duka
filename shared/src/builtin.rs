use std::{
    cmp::Eq,
    collections::HashMap,
    hash::Hash,
    sync::{LazyLock, RwLock},
};

use crate::builtin_meta::MetaInfo;

pub type GlobalBuiltins<V, K = &'static str> = LazyLock<RwLock<Builtins<V, K>>>;

#[derive(Debug, Clone, Default)]
pub struct Builtins<V, K = &'static str>
where
    K: Hash + Eq,
{
    maps: HashMap<K, V>,
    metas: Vec<MetaInfo>,
}

impl<K: Hash + Eq, V> Builtins<V, K> {
    pub fn new() -> Self {
        Self {
            maps: HashMap::new(),
            metas: Vec::new(),
        }
    }
    pub fn new_global() -> GlobalBuiltins<V, K> {
        LazyLock::new(|| RwLock::new(Self::new()))
    }
    pub fn register(mut self, key: K, val: V) -> Self {
        self.maps.insert(key, val);
        self
    }
    pub fn register_meta(mut self, key: K, val: V, meta: MetaInfo) -> Self {
        self.maps.insert(key, val);
        self.metas.push(meta);
        self
    }
    pub fn remove(&mut self, key: &K) -> Option<V> {
        self.maps.remove(key)
    }
    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.maps.get_mut(key)
    }
    pub fn get(&self, key: &K) -> Option<&V> {
        self.maps.get(key)
    }
    pub fn len(&self) -> usize {
        self.maps.len()
    }
    pub fn is_empty(&self) -> bool {
        self.maps.is_empty()
    }
    pub fn into_inner(self) -> HashMap<K, V> {
        self.maps
    }
    pub fn into_metas(self) -> Vec<MetaInfo> {
        self.metas
    }
}
