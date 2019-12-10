use std::marker::PhantomData;

type Index = u16;
type Epoch = u16;

pub struct Id<T>(Index, Epoch, PhantomData<T>);

impl<T> Id<T> {
    pub const ZERO: Self = Id(0, 0, PhantomData);

    pub fn index(&self) -> usize {
        self.0 as usize
    }

    pub fn is_valid(&self) -> bool {
        self.0 != 0 || self.1 != 0
    }
}

pub struct FreeList<T> {
    epochs: Vec<Epoch>,
    free: Vec<Index>,
    marker: PhantomData<T>,
}

impl<T> FreeList<T> {
    pub fn new() -> Self {
        FreeList {
            epochs: Vec::new(),
            free: Vec::new(),
            marker: PhantomData,
        }
    }

    pub fn alloc(&mut self) -> Id<T> {
        match self.free.pop() {
            Some(index) => {
                Id(index, self.epochs[index as usize], PhantomData)
            }
            None => {
                const START_EPOCH: Epoch = 1;
                let index = self.epochs.len() as Index;
                self.epochs.push(START_EPOCH);
                Id(index, START_EPOCH, PhantomData)
            }
        }
    }

    pub fn free(&mut self, id: Id<T>) {
        assert_eq!(self.epochs[id.0 as usize], id.1);
        self.epochs[id.0 as usize] += 1;
        self.free.push(id.0);
    }

    pub fn length(&self) -> usize {
        self.epochs.len()
    }
}
