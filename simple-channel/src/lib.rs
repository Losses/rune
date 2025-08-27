use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;
use tokio::sync::broadcast::error::TryRecvError;

pub struct SimpleChannel<T> {
    _queue: Arc<Mutex<VecDeque<T>>>,
    _capacity: usize,
    _notify: Arc<Notify>,
}

#[derive(Clone)]
pub struct SimpleSender<T> {
    queue: Arc<Mutex<VecDeque<T>>>,
    capacity: usize,
    notify: Arc<Notify>,
}

#[derive(Clone)]
pub struct SimpleReceiver<T> {
    queue: Arc<Mutex<VecDeque<T>>>,
    notify: Arc<Notify>,
}

impl<T> SimpleChannel<T> {
    pub fn channel(capacity: usize) -> (SimpleSender<T>, SimpleReceiver<T>) {
        let queue = Arc::new(Mutex::new(VecDeque::with_capacity(capacity)));
        let notify = Arc::new(Notify::new());
        let sender = SimpleSender {
            queue: queue.clone(),
            capacity,
            notify: notify.clone(),
        };
        let receiver = SimpleReceiver { queue, notify };
        (sender, receiver)
    }
}

impl<T> SimpleSender<T> {
    pub fn send(&self, msg: T) {
        let mut queue = self.queue.lock().unwrap();
        if queue.len() == self.capacity {
            queue.clear(); // Clear the queue if full
        }
        queue.push_back(msg);
        self.notify.notify_one();
    }

    pub fn subscribe(&self) -> SimpleReceiver<T> {
        SimpleReceiver {
            queue: self.queue.clone(),
            notify: self.notify.clone(),
        }
    }
}

impl<T> SimpleReceiver<T> {
    pub async fn recv(&self) -> Result<T, TryRecvError> {
        loop {
            {
                let mut queue = self.queue.lock().unwrap();
                if let Some(msg) = queue.pop_front() {
                    return Ok(msg);
                }
            }
            self.notify.notified().await;
        }
    }
}
