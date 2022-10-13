use std::alloc::{Global, Allocator, Layout, LayoutError, handle_alloc_error};
use std::collections::TryReserveError;
use std::collections::TryReserveErrorKind::AllocError;
use std::{cmp, intrinsics, ptr};
use std::collections::TryReserveErrorKind::CapacityOverflow;
use std::intrinsics::assume;
use std::mem;
use std::mem::SizedTypeProperties;
use std::ptr::{Unique, NonNull};

pub struct Queue<T> {
    head: Option<NonNull<Node<T>>>,
    tail: Option<NonNull<Node<T>>>,
    size: usize
}

impl <T> Queue<T> {
    pub fn new() -> Self {
        Self {
            head: None,
            tail: None,
            size: 0
        }
    }
}

impl<T> Queue<T> {
    #[inline]
    pub fn push(&mut self, element: T) {
        let mut node =  Box::new(Node::new(element));
        unsafe {
            node.next = self.head;
            node.prev = None;
            let node = Some(Box::leak(node).into());

            match self.head {
                None => self.tail = node,
                Some(head) => (*head.as_ptr()).prev = node
            }

            self.head = node;
            self.size += 1;
        }
    }

    #[inline]
    pub fn take(&mut self) -> Option<T> {
        self.tail.map(|node| unsafe {
            let node = Box::from_raw(node.as_ptr());
            self.tail = node.prev;

            match self.tail {
                None => self.head = None,
                Some(tail) => (*tail.as_ptr()).next = None
            }

            self.size -= 1;
            Node::into_element(node)
        })
    }
}

pub struct Node<T> {
    next: Option<NonNull<Node<T>>>,
    prev: Option<NonNull<Node<T>>>,
    element: T
}

impl<T> Node<T> {
    pub fn new(element: T) -> Self {
        Self {
            next: None,
            prev: None,
            element
        }
    }

    pub fn into_element(self: Box<Self>) -> T {
        self.element
    }
}

enum AllocInit {
    /// The contents of the new memory are uninitialized.
    Uninitialized,
    /// The new memory is guaranteed to be zeroed.
    Zeroed,
}

pub struct Stack<T, A: Allocator = Global> {
    buf: RawStack<T, A>,
    len: usize,
}

impl<T> Stack<T> {
    #[inline]
    pub const fn new() -> Self {
        Self { buf: RawStack::NEW, len: 0 }
    }

    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity_in(capacity, Global)
    }
}

impl<T, A: Allocator> Stack<T, A> {
    #[inline]
    pub const fn new_in(alloc: A) -> Self {
        Stack { buf: RawStack::new_in(alloc), len: 0 }
    }

    #[inline]
    pub fn with_capacity_in(capacity: usize, alloc: A) -> Self {
        Stack { buf: RawStack::with_capacity_in(capacity, alloc), len: 0 }
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.buf.capacity()
    }

    pub fn reserve(&mut self, additional: usize) {
        self.buf.reserve(self.len, additional);
    }

    pub fn reserve_exact(&mut self, additional: usize) {
        self.buf.reserve_exect(self.len, additional);
    }

    pub fn try_reserve(&mut self, additional: usize) -> Result<(), TryReserveError> {
        self.buf.try_reserve(self.len, additional)
    }

    pub fn try_reserve_exact(&mut self, additional: usize) -> Result<(), TryReserveError> {
        self.buf.try_reserve_exact(self.len, additional)
    }

    pub fn shrink_to_fit(&mut self) {
        if self.capacity() > self.len {
            self.buf.shrink_to_fit(self.len);
        }
    }

    pub fn shrink_to(&mut self, min_capacity: usize) {
        if self.capacity() > min_capacity {
            self.buf.shrink_to_fit(cmp::max(self.len, min_capacity));
        }
    }

    pub fn as_ptr(&self) -> *const T {
        let ptr = self.buf.ptr();
        unsafe {
            assume(!ptr.is_null());
        }
        ptr
    }

    pub fn as_mut_ptr(&mut self) -> *mut T {
        let ptr = self.buf.ptr();
        unsafe {
            assume(!ptr.is_null());
        }
        ptr
    }

    #[inline]
    pub fn allocator(&self) -> &A {
        self.buf.allocator()
    }
}

impl<T> Stack<T> {
    pub fn push(&mut self, element: T) {
        if self.len == self.buf.capacity() {
            self.buf.reserve_for_push(self.len);
        }
        unsafe {
            let end = self.as_mut_ptr().add(self.len);
            ptr::write(end, element);
            self.len += 1;
        }
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            None
        } else {
            unsafe {
                self.len -= 1;
                Some(ptr::read(self.as_mut_ptr().add(self.len)))
            }
        }
    }
}

struct RawStack<T, A: Allocator = Global> {
    ptr: Unique<T>,
    capacity: usize,
    alloc: A,
}

impl<T> RawStack<T, Global> {

    pub const NEW: Self = Self::new();

    pub const fn new() -> Self {
        Self::new_in(Global)
    }

    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity_in(capacity, Global)
    }

    #[inline]
    pub fn with_capacity_zeroed(capacity: usize) -> Self {
        Self::with_capacity_zeroed_in(capacity, Global)
    }
}

impl<T, A: Allocator> RawStack<T, A> {

    pub(crate) const MIN_NON_ZERO_CAP: usize = if mem::size_of::<T>() == 1 {
        8
    } else if mem::size_of::<T>() <= 1024 {
        4
    } else {
        1
    };

    #[inline]
    pub const fn new_in(alloc: A) -> Self {
        Self { ptr: Unique::dangling(), capacity: 0, alloc }
    }

    #[inline]
    pub fn with_capacity_in(capacity: usize, alloc: A) -> Self {
        Self::allocate_in(capacity, AllocInit::Uninitialized, alloc)
    }

    #[inline]
    pub fn with_capacity_zeroed_in(capacity: usize, alloc: A) -> Self {
        Self::allocate_in(capacity, AllocInit::Zeroed, alloc)
    }

    fn allocate_in(capacity: usize, init: AllocInit, alloc: A) -> Self {
        if T::IS_ZST || capacity == 0 {
            Self::new_in(alloc)
        } else {
            let layout = match Layout::array::<T>(capacity) {
                Ok(layout) => layout,
                Err(_) => capacity_overflow(),
            };
            match alloc_guard(layout.size()) {
                Ok(_) => {},
                Err(_) => capacity_overflow()
            }
            let result = match init {
                AllocInit::Uninitialized => alloc.allocate(layout),
                AllocInit::Zeroed => alloc.allocate_zeroed(layout),
            };
            let ptr = match result {
                Ok(ptr) => ptr,
                Err(_) => handle_alloc_error(layout),
            };

            Self {
                ptr: unsafe {
                    Unique::new_unchecked(ptr.cast().as_ptr())
                },
                capacity,
                alloc,
            }
        }
    }

    #[inline]
    pub fn ptr(&self) -> *mut T {
        self.ptr.as_ptr()
    }

    #[inline(always)]
    pub fn capacity(&self) -> usize {
        if T::IS_ZST { usize::MAX } else { self.capacity }
    }

    pub fn allocator(&self) -> &A {
        &self.alloc
    }

    fn current_memory(&self) -> Option<(NonNull<u8>, Layout)> {
        if T::IS_ZST || self.capacity == 0 {
            None
        } else {
            unsafe {
                let layout = Layout::array::<T>(self.capacity).unwrap_unchecked();
                Some((self.ptr.cast().into(), layout))
            }
        }
    }

    #[inline]
    pub fn reserve(&mut self, len: usize, additional: usize) {
        #[cold]
        fn do_reserve_and_handle<T, A: Allocator>(
                slf: &mut RawStack<T, A>,
                len: usize,
                additional: usize
        ) {
            handle_reserve(slf.grow_amortized(len, additional));
        }

        if self.needs_to_grow(len, additional) {
            do_reserve_and_handle(self, len, additional);
        }
    }

    #[inline(never)]
    pub fn reserve_for_push(&mut self, len: usize) {
        handle_reserve(self.grow_amortized(len, 1));
    }

    pub fn try_reserve(&mut self, len: usize, additional: usize) -> Result<(), TryReserveError> {
        if self.needs_to_grow(len, additional) {
            self.grow_amortized(len, additional)
        } else {
            Ok(())
        }
    }

    pub fn reserve_exect(&mut self, len: usize, additional: usize) {
        handle_reserve(self.try_reserve_exact(len, additional));
    }

    pub fn try_reserve_exact(
            &mut self,
            len: usize,
            additional: usize,
    ) -> Result<(), TryReserveError> {
        if self.needs_to_grow(len, additional) {
            self.grow_exact(len, additional)
        } else {
            Ok(())
        }
    }

    pub fn shrink_to_fit(&mut self, capacity: usize) {
        handle_reserve(self.shrink(capacity));
    }
}

impl<T, A: Allocator> RawStack<T, A> {

    fn needs_to_grow(&self, len: usize, additional: usize) -> bool {
        additional > self.capacity().wrapping_sub(len)
    }

    fn set_ptr_and_capacity(&mut self, ptr: NonNull<[u8]>, capacity: usize) {
        self.ptr = unsafe { Unique::new_unchecked(ptr.cast().as_ptr()) };
        self.capacity = capacity;
    }

    fn grow_amortized(&mut self, len: usize, additional: usize) -> Result<(), TryReserveError> {
        debug_assert!(additional > 0);

        if T::IS_ZST {
            return Err(CapacityOverflow.into());
        }

        let required_capacity = len.checked_add(additional).ok_or(CapacityOverflow)?;

        let capacity = cmp::max(self.capacity * 2, required_capacity);
        let capacity = cmp::max(Self::MIN_NON_ZERO_CAP, capacity);

        let new_layout = Layout::array::<T>(capacity);

        let ptr = finish_grow(new_layout, self.current_memory(), &mut self.alloc)?;
        self.set_ptr_and_capacity(ptr, capacity);
        Ok(())
    }

    fn grow_exact(&mut self, len: usize, additional: usize) -> Result<(), TryReserveError> {
        if T::IS_ZST {
            return Err(CapacityOverflow.into());
        }

        let capacity = len.checked_add(additional).ok_or(CapacityOverflow)?;
        let new_layout = Layout::array::<T>(capacity);

        let ptr = finish_grow(new_layout, self.current_memory(), &mut self.alloc)?;
        self.set_ptr_and_capacity(ptr, capacity);
        Ok(())
    }

    fn shrink(&mut self, capacity: usize) -> Result<(), TryReserveError> {
        assert!(capacity <= self.capacity(), "Tried to shrink to a larger capacity");

        let (ptr, layout) = if let Some(mem) = self.current_memory() { mem } else { return Ok(()) };

        let ptr = unsafe {
            let new_layout = Layout::array::<T>(capacity).unwrap_unchecked();
            self.alloc
            .shrink(ptr, layout, new_layout)
            .map_err(|_| AllocError { layout: new_layout, non_exhaustive: () })?
        };
        self.set_ptr_and_capacity(ptr, capacity);
        Ok(())
    }
}

fn finish_grow<A>(
        new_layout: Result<Layout, LayoutError>,
        current_memory: Option<(NonNull<u8>, Layout)>,
        alloc: &mut A,
        ) -> Result<NonNull<[u8]>, TryReserveError>
where A: Allocator {
    let new_layout = new_layout.map_err(|_| CapacityOverflow)?;

    alloc_guard(new_layout.size())?;

    let memory = if let Some((ptr, old_layout)) = current_memory {
        debug_assert_eq!(old_layout.align(), new_layout.align());
        unsafe {
            intrinsics::assume(old_layout.align() == new_layout.align());
            alloc.grow(ptr, old_layout, new_layout)
        }
    } else {
        alloc.allocate(new_layout)
    };

    memory.map_err(|_| AllocError { layout: new_layout, non_exhaustive: () }.into())
}

/*impl<T, A: Allocator> Drop for RawStack<T, A> {
    fn drop(&mut self) {
        if let Some((ptr, layout)) = self.current_memory() {
            unsafe { self.alloc.deallocate(ptr, layout) }
        }
    }
}*/

#[inline]
fn handle_reserve(result: Result<(), TryReserveError>) {
    match result.map_err(|e| e.kind()) {
        Err(CapacityOverflow) => capacity_overflow(),
        Err(AllocError { layout, .. }) => handle_alloc_error(layout),
        Ok(()) => {}
    }
}

fn alloc_guard(alloc_size: usize) -> Result<(), TryReserveError> {
    if usize::BITS < 64 && alloc_size > isize::MAX as usize {
        Err(CapacityOverflow.into())
    } else {
        Ok(())
    }
}

fn capacity_overflow() -> ! {
    panic!("capacity oveefow");
}