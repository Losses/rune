use std::fs::File;
use std::io::BufReader;
use std::sync::{Arc, Mutex};

use rodio::{Decoder, Source};
use tokio::time::Duration;

use crate::buffered::RuneBuffered;

pub struct SharedSource {
    pub inner: Arc<Mutex<RuneBuffered<Decoder<BufReader<File>>>>>,
}

impl SharedSource {
    pub fn new(source: RuneBuffered<Decoder<BufReader<File>>>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(source)),
        }
    }
}

impl Iterator for SharedSource {
    type Item = <RuneBuffered<Decoder<BufReader<File>>> as Iterator>::Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.lock().unwrap().next()
    }
}
