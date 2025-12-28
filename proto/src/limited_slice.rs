use std::cmp;
use std::marker::PhantomData;
use std::ptr::NonNull;

/// # Safety
///
/// * if y := TryInto::<usize>(x) exists TryFrom::<usize>::(y) always succeds
/// * const MAX: usize represents the biggest size than can fit in this usize
/// * if the index is <= MAX TryFrom::<usize>::(index) exists
pub(crate) unsafe trait Length: Sized + Ord + Copy + Send + Sync + 'static + TryFrom<usize> + TryInto<usize> {
    const MAX: usize;
}

unsafe impl Length for u8 {
    const MAX: usize = u8::MAX as usize;
}
unsafe impl Length for u16 {
    const MAX: usize = u16::MAX as usize;
}
unsafe impl Length for u32 {
    const MAX: usize = match usize::BITS <= u32::BITS {
        true => usize::MAX,
        false => u32::MAX as usize,
    };
}

pub(crate) struct LimitedSlice<'a, T, L: Length> {
    ptr: NonNull<T>,
    length: L,
    _marker: PhantomData<&'a [T]>
}

unsafe impl<'a, T, L: Length> Send for LimitedSlice<'a, T, L> where &'a [T]: Send {}
unsafe impl<'a, T, L: Length> Sync for LimitedSlice<'a, T, L> where &'a [T]: Sync {}

impl<'a, T, L: Length> Copy for LimitedSlice<'a, T, L> {}
impl<'a, T, L: Length> Clone for LimitedSlice<'a, T, L> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, T, L: Length> LimitedSlice<'a, T, L> {
    pub fn new(slice: &'a [T]) -> Option<Self> {
        let Ok(length) = L::try_from(slice.len()) else {
            return None
        };
        
        Some(Self {
            ptr: NonNull::from_ref(slice).cast::<T>(),
            length,
            _marker: PhantomData
        })
    }
    
    pub fn truncated(slice: &'a [T]) -> Self {
        let length = unsafe { 
            L::try_from(cmp::min(slice.len(), L::MAX)).unwrap_unchecked()
        };

        Self {
            ptr: NonNull::from_ref(slice).cast::<T>(),
            length,
            _marker: PhantomData
        }
    }
    
    pub fn len(&self) -> L {
        self.length
    }
    
    pub fn as_slice(&self) -> &'a [T] {
        unsafe {
            core::slice::from_raw_parts(
                self.ptr.as_ptr(),
                self.length.try_into().unwrap_unchecked()
            )
        }
    }
}

pub(crate) struct LimitedStr<'a, L: Length>(LimitedSlice<'a, u8, L>);

impl<'a, L: Length> LimitedStr<'a, L> {
    pub fn new(slice: &'a str) -> Option<Self> {
        LimitedSlice::new(slice.as_bytes()).map(Self)
    }
    
    pub fn truncated(slice: &'a str) -> Self {
        Self(LimitedSlice::truncated(slice.as_bytes()))
    }
    
    pub fn len(&self) -> L {
        self.0.len()
    }

    pub fn as_str(&self) -> &'a str {
        unsafe { str::from_utf8_unchecked(self.0.as_slice()) }
    }
}

impl<'a, L: Length> Copy for LimitedStr<'a, L> {}
impl<'a, L: Length> Clone for LimitedStr<'a, L> {
    fn clone(&self) -> Self {
        *self
    }
}
