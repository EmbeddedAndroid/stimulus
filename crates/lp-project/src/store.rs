use crate::Capture;
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{Arc, RwLock},
};

#[derive(Clone)]
pub struct CaptureStore {
    inner: Arc<RwLock<Inner>>,
    capacity: usize,
}

#[derive(Default)]
struct Inner {
    captures: VecDeque<Capture>,
    pinned: BTreeSet<u32>,
    next_id: u32,
    next_seq: u64,
}

impl CaptureStore {
    pub fn new(capacity: usize) -> Result<Self, StoreError> {
        if capacity == 0 {
            return Err(StoreError::ZeroCapacity);
        }
        Ok(Self {
            inner: Arc::new(RwLock::new(Inner {
                next_id: 1,
                next_seq: 1,
                ..Inner::default()
            })),
            capacity,
        })
    }

    pub fn insert(&self, mut capture: Capture) -> Result<Capture, StoreError> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Poisoned)?;
        capture.id = inner.next_id;
        capture.seq = inner.next_seq;
        inner.next_id = inner
            .next_id
            .checked_add(1)
            .ok_or(StoreError::IdExhausted)?;
        inner.next_seq = inner
            .next_seq
            .checked_add(1)
            .ok_or(StoreError::IdExhausted)?;
        inner.captures.push_back(capture.clone());
        while inner.captures.len() > self.capacity {
            let removable = inner
                .captures
                .iter()
                .position(|item| !inner.pinned.contains(&item.id));
            match removable {
                Some(index) => {
                    inner.captures.remove(index);
                }
                None => break,
            }
        }
        Ok(capture)
    }

    pub fn get(&self, id: u32) -> Result<Option<Capture>, StoreError> {
        let inner = self.inner.read().map_err(|_| StoreError::Poisoned)?;
        Ok(inner
            .captures
            .iter()
            .find(|capture| capture.id == id)
            .cloned())
    }

    pub fn latest(&self) -> Result<Option<Capture>, StoreError> {
        let inner = self.inner.read().map_err(|_| StoreError::Poisoned)?;
        Ok(inner.captures.back().cloned())
    }

    pub fn list(&self, limit: usize) -> Result<Vec<Capture>, StoreError> {
        let inner = self.inner.read().map_err(|_| StoreError::Poisoned)?;
        Ok(inner.captures.iter().rev().take(limit).cloned().collect())
    }

    pub fn pin(&self, id: u32, pinned: bool) -> Result<(), StoreError> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Poisoned)?;
        if !inner.captures.iter().any(|capture| capture.id == id) {
            return Err(StoreError::UnknownCapture(id));
        }
        if pinned {
            inner.pinned.insert(id);
        } else {
            inner.pinned.remove(&id);
        }
        Ok(())
    }

    pub fn is_pinned(&self, id: u32) -> Result<bool, StoreError> {
        let inner = self.inner.read().map_err(|_| StoreError::Poisoned)?;
        if !inner.captures.iter().any(|capture| capture.id == id) {
            return Err(StoreError::UnknownCapture(id));
        }
        Ok(inner.pinned.contains(&id))
    }

    pub fn remove(&self, id: u32) -> Result<Capture, StoreError> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Poisoned)?;
        let index = inner
            .captures
            .iter()
            .position(|capture| capture.id == id)
            .ok_or(StoreError::UnknownCapture(id))?;
        inner.pinned.remove(&id);
        inner
            .captures
            .remove(index)
            .ok_or(StoreError::UnknownCapture(id))
    }

    pub fn clear(&self) -> Result<(), StoreError> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Poisoned)?;
        inner.captures.clear();
        inner.pinned.clear();
        Ok(())
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StoreError {
    #[error("capture store capacity must be positive")]
    ZeroCapacity,
    #[error("capture store lock is poisoned")]
    Poisoned,
    #[error("capture ID/sequence space exhausted")]
    IdExhausted,
    #[error("unknown capture {0}")]
    UnknownCapture(u32),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Run;
    fn capture(data: u64) -> Capture {
        Capture::new(0, 1e-6, 0, vec![Run { data, count: 1 }]).unwrap_or_else(|e| panic!("{e}"))
    }
    #[test]
    fn retains_latest_sixteen_and_respects_pin() {
        let store = CaptureStore::new(16).unwrap_or_else(|e| panic!("{e}"));
        let first = store.insert(capture(0)).unwrap_or_else(|e| panic!("{e}"));
        store.pin(first.id, true).unwrap_or_else(|e| panic!("{e}"));
        for value in 1..20 {
            store
                .insert(capture(value))
                .unwrap_or_else(|e| panic!("{e}"));
        }
        assert!(
            store
                .get(first.id)
                .unwrap_or_else(|e| panic!("{e}"))
                .is_some()
        );
        assert_eq!(store.list(100).unwrap_or_else(|e| panic!("{e}")).len(), 16);
        assert_eq!(
            store
                .latest()
                .unwrap_or_else(|e| panic!("{e}"))
                .map(|c| c.id),
            Some(20)
        );
        assert_eq!(store.is_pinned(first.id), Ok(true));
        assert_eq!(
            store.remove(first.id).map(|capture| capture.id),
            Ok(first.id)
        );
        assert_eq!(store.get(first.id), Ok(None));
        assert_eq!(
            store.is_pinned(first.id),
            Err(StoreError::UnknownCapture(first.id))
        );
    }
}
