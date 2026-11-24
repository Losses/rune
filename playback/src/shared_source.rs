use std::sync::{Arc, Mutex};

use rodio::{Sample, Source};

pub struct SharedSource<S: Source>
where
    S::Item: Sample,
{
    pub inner: Arc<Mutex<S>>,
}

impl<S: Source> SharedSource<S>
where
    S::Item: Sample,
{
    pub fn new(source: S) -> Self {
        Self {
            inner: Arc::new(Mutex::new(source)),
        }
    }
}

impl<S: Source> Iterator for SharedSource<S>
where
    S::Item: Sample,
{
    type Item = S::Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.lock().unwrap().next()
    }
}
