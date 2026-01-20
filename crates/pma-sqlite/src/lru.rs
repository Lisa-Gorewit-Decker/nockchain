#[derive(Debug)]
struct Slot<V> {
    id: i64,
    value: Option<V>,
    prev: Option<usize>,
    next: Option<usize>,
}

#[derive(Debug)]
pub struct LruCache<V> {
    capacity: usize,
    base_id: i64,
    start: usize,
    slots: Vec<Slot<V>>,
    head: Option<usize>,
    tail: Option<usize>,
    len: usize,
}

impl<V> LruCache<V> {
    pub fn new(capacity: usize) -> Self {
        let mut slots = Vec::with_capacity(capacity);
        slots.resize_with(capacity, || Slot {
            id: 0,
            value: None,
            prev: None,
            next: None,
        });
        Self {
            capacity,
            base_id: 0,
            start: 0,
            slots,
            head: None,
            tail: None,
            len: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get_mut(&mut self, id: i64) -> Option<&mut V> {
        if self.capacity == 0 || !self.in_window(id) {
            return None;
        }
        let idx = self.slot_index(id);
        let slot = self.slots.get_mut(idx)?;
        if slot.value.is_none() || slot.id != id {
            return None;
        }
        self.move_to_head(idx);
        self.slots.get_mut(idx).and_then(|slot| slot.value.as_mut())
    }

    pub fn insert(&mut self, id: i64, value: V) -> Option<V> {
        if self.capacity == 0 {
            return None;
        }
        if !self.in_window(id) {
            self.slide_to_include(id);
        }
        let idx = self.slot_index(id);
        let (occupied, slot_id) = {
            let slot = &self.slots[idx];
            (slot.value.is_some(), slot.id)
        };
        if occupied && slot_id == id {
            let slot = &mut self.slots[idx];
            if let Some(existing) = slot.value.as_mut() {
                let old = std::mem::replace(existing, value);
                self.move_to_head(idx);
                return Some(old);
            }
        }
        if occupied {
            self.clear_slot(idx);
        }
        let slot = &mut self.slots[idx];
        slot.id = id;
        slot.value = Some(value);
        self.len = self.len.saturating_add(1);
        self.attach_head(idx);
        None
    }

    fn in_window(&self, id: i64) -> bool {
        if self.len == 0 {
            return false;
        }
        let max_id = self.base_id.saturating_add(self.capacity as i64);
        id >= self.base_id && id < max_id
    }

    fn slot_index(&self, id: i64) -> usize {
        let offset = id.saturating_sub(self.base_id) as usize;
        if self.capacity == 0 {
            0
        } else {
            (self.start + offset) % self.capacity
        }
    }

    fn slide_to_include(&mut self, id: i64) {
        if self.capacity == 0 {
            return;
        }
        if self.len == 0 {
            self.base_id = id;
            self.start = 0;
            return;
        }
        if id < self.base_id {
            self.clear_all();
            self.base_id = id;
            self.start = 0;
            return;
        }
        let max_id = self.base_id + self.capacity as i64 - 1;
        if id <= max_id {
            return;
        }
        let new_base = id - self.capacity as i64 + 1;
        let delta = new_base - self.base_id;
        if delta >= self.capacity as i64 {
            self.clear_all();
            self.base_id = new_base;
            self.start = 0;
            return;
        }
        let delta_usize = delta as usize;
        for i in 0..delta_usize {
            let idx = (self.start + i) % self.capacity;
            self.clear_slot(idx);
        }
        self.start = (self.start + delta_usize) % self.capacity;
        self.base_id = new_base;
    }

    fn clear_slot(&mut self, idx: usize) -> Option<V> {
        if self.slots.get(idx).is_none() {
            return None;
        }
        if self.slots[idx].value.is_none() {
            return None;
        }
        self.detach(idx);
        self.len = self.len.saturating_sub(1);
        self.slots[idx].prev = None;
        self.slots[idx].next = None;
        let mut value = None;
        std::mem::swap(&mut self.slots[idx].value, &mut value);
        value
    }

    fn clear_all(&mut self) {
        for idx in 0..self.slots.len() {
            self.slots[idx].id = 0;
            self.slots[idx].value = None;
            self.slots[idx].prev = None;
            self.slots[idx].next = None;
        }
        self.head = None;
        self.tail = None;
        self.len = 0;
    }

    fn move_to_head(&mut self, idx: usize) {
        if self.head == Some(idx) {
            return;
        }
        self.detach(idx);
        self.attach_head(idx);
    }

    fn detach(&mut self, idx: usize) {
        let prev = self.slots[idx].prev;
        let next = self.slots[idx].next;
        if let Some(prev_idx) = prev {
            self.slots[prev_idx].next = next;
        }
        if let Some(next_idx) = next {
            self.slots[next_idx].prev = prev;
        }
        if self.head == Some(idx) {
            self.head = next;
        }
        if self.tail == Some(idx) {
            self.tail = prev;
        }
        self.slots[idx].prev = None;
        self.slots[idx].next = None;
    }

    fn attach_head(&mut self, idx: usize) {
        self.slots[idx].prev = None;
        self.slots[idx].next = self.head;
        if let Some(head_idx) = self.head {
            self.slots[head_idx].prev = Some(idx);
        }
        self.head = Some(idx);
        if self.tail.is_none() {
            self.tail = Some(idx);
        }
    }
}
