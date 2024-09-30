use nih_plug::params::persist::PersistentField;
use serde::{Deserialize, Serialize};
use std::ops::Deref;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::RUNTIME;

#[derive(Debug)]
pub struct TokioMutexParam<T: Send + Sync> {
    inner: Arc<Mutex<T>>,
}

impl<'a, T: Send + Sync + Serialize + Deserialize<'a> + Default> Default for TokioMutexParam<T> {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(T::default())),
        }
    }
}

impl<'a, T: Send + Sync + Serialize + Deserialize<'a>> PersistentField<'a, T>
    for TokioMutexParam<T>
{
    fn set(&self, value: T) {
        let mut inner = RUNTIME.block_on(self.inner.lock());
        *inner = value;
    }
    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&T) -> R,
    {
        let inner = RUNTIME.block_on(self.inner.lock());
        f(&*inner)
    }
}

impl<'a, T: Send + Sync + Serialize + Deserialize<'a>> Deref for TokioMutexParam<T> {
    type Target = Arc<Mutex<T>>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
