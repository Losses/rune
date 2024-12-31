pub struct Ring<T: Clone + Default> {
    pub buf: Vec<T>,
    pub index: usize,
}

impl<T: Clone + Default> Ring<T> {
    pub fn new(size: usize) -> Self {
        Ring {
            buf: vec![T::default(); size],
            index: 0,
        }
    }

    pub fn mod_index(&self, i: i32) -> usize {
        let len = self.buf.len() as i32;
        let mut idx = i;
        while idx < 0 {
            idx += len;
        }
        (idx as usize) % self.buf.len()
    }

    pub fn at(&self, i: i32) -> &T {
        let idx = self.mod_index(self.index as i32 + i);
        &self.buf[idx]
    }

    pub fn append(&mut self, values: &[T]) {
        let mut remaining = values;
        while !remaining.is_empty() {
            let space_left = self.buf.len() - self.index;
            let n = std::cmp::min(space_left, remaining.len());

            // Copy the slice
            self.buf[self.index..self.index + n].clone_from_slice(&remaining[..n]);

            remaining = &remaining[n..];
            self.index = (self.index + n) % self.buf.len();
        }
    }

    pub fn slice(&self, dest: &mut [T], offset: i32) {
        let mut offset = self.mod_index(offset + self.index as i32);
        let mut remaining = dest;

        while !remaining.is_empty() {
            let available = self.buf.len() - offset;
            let n = std::cmp::min(available, remaining.len());

            remaining[..n].clone_from_slice(&self.buf[offset..offset + n]);

            remaining = &mut remaining[n..];
            offset = (offset + n) % self.buf.len();
        }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}
