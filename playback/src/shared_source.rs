use std::sync::{Arc, Mutex};

use rodio::{Sample, Source};

pub struct SharedSource<S: Source> {
    pub inner: Arc<Mutex<S>>,
}

impl<S: Source> SharedSource<S> {
    pub fn new(source: S) -> Self {
        Self {
            inner: Arc::new(Mutex::new(source)),
        }
    }
}

impl<S: Source> Iterator for SharedSource<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.lock().unwrap().next()
    }
}
