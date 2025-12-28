use std::io;
use std::num::NonZero;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use bytemuck::Zeroable;


struct Buffer<T>(Arc<[T]>);

impl<T: Zeroable> Buffer<T> {
    pub fn new(len: u32) -> io::Result<Self> {
        if let Ok(len) = usize::try_from(len) {
            // Safety: T is zeroable and therfore this is a valid repr of T
            Ok(Self(unsafe { Arc::new_zeroed_slice(len).assume_init() }))
        } else {
            Err(io::Error::from(io::ErrorKind::OutOfMemory))
        }
    }

    pub fn from_slice(slice: &[T]) -> io::Result<Self> where T: Copy {
        if u32::try_from(slice.len()).is_err() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "buffer too long"
            ));
        }
        
        Ok(Self(Arc::<[T]>::from(slice)))
    }

    pub fn len(&self) -> u32 {
        unsafe { u32::try_from(self.0.len()).unwrap_unchecked() }
    }

    #[inline(always)]
    fn get_n<const N: usize>(&self, index: u32) -> Result<&[T; N], ()> {
        let max_index = index.checked_add(const {
            let bias = NonZero::new(N).unwrap().get() - 1;
            assert!(bias <= 256);
            bias as u32
        });

        if let Some(max_index) = max_index
            && max_index < self.len()
        {
            unsafe {
                let index = usize::try_from(index).unwrap_unchecked();
                return Ok(self.0.get_unchecked(index..).first_chunk::<N>().unwrap_unchecked())
            }
        }

        Err(())
    }


    #[inline(always)]
    fn get(&self, index: u32) -> Result<&T, ()> {
        match self.get_n::<1>(index) {
            Ok([x]) => Ok(x),
            Err(()) => Err(())
        }
    }
}

impl<T> Clone for Buffer<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
    
    fn clone_from(&mut self, source: &Self) {
        if !Arc::ptr_eq(&self.0, &source.0) {
            *self = source.clone()
        }
    }
}

#[derive(Clone)]
pub struct OutputBuffer(Buffer<AtomicU8>);

macro_rules! make_store_op {
    ($name: ident, $ty: ty, $to_bytes: ident) => {
        #[inline(always)]
        pub fn $name(&self, index: u32, val: $ty) -> Result<(), ()> {
            let locations = self.0.get_n::<{size_of::<$ty>()}>(index)?;
            let bytes = <$ty>::$to_bytes(val);
            for i in 0..bytes.len() {
                locations[i].store(bytes[i], Ordering::Relaxed)
            }
            Ok(())
        }
    };
}

impl OutputBuffer {
    pub fn new(len: u32) -> io::Result<Self> {
        Buffer::new(len).map(Self)
    }

    pub fn len(&self) -> u32 {
        self.0.len()
    }

    #[inline(always)]
    pub fn store(&self, index: u32, val: u8) -> Result<(), ()> {
        self.0.get(index).map(|byte| byte.store(val, Ordering::Relaxed))
    }

    make_store_op! {  store_x2_le, u16, to_le_bytes }
    make_store_op! {  store_x4_le, u32, to_le_bytes }
    make_store_op! {  store_x8_le, u64, to_le_bytes }

    make_store_op! {  store_x2_be, u16, to_be_bytes }
    make_store_op! {  store_x4_be, u32, to_be_bytes }
    make_store_op! {  store_x8_be, u64, to_be_bytes }

    pub fn into_bytes(mut self) -> Result<Arc<[u8]>, ()> {
        let Some(_mutable) = Arc::get_mut(&mut self.0.0) else {
            return Err(())
        };
        
        Ok(unsafe { Arc::from_raw(Arc::into_raw(self.0.0) as *const [u8]) })
    }
}


#[derive(Clone)]
pub struct InputBuffer(Buffer<u8>);


macro_rules! make_load_op {
    ($name: ident, $ty: ty, $from_bytes: ident) => {
        #[inline(always)]
        pub fn $name(&self, index: u32) -> Result<$ty, ()> {
            self.0.get_n::<{size_of::<$ty>()}>(index).copied().map(<$ty>::$from_bytes)
        }
    };
}

impl InputBuffer {
    pub fn from_slice(slice: &[u8]) -> io::Result<Self> {
        Buffer::from_slice(slice).map(Self)
    }


    pub fn len(&self) -> u32 {
        self.0.len()
    }

    #[inline(always)]
    pub fn load(&self, index: u32) -> Result<u8, ()> {
        self.0.get(index).copied()
    }


    make_load_op! { load_x2_le, u16, from_le_bytes }
    make_load_op! { load_x4_le, u32, from_le_bytes }
    make_load_op! { load_x8_le, u64, from_le_bytes }

    make_load_op! { load_x2_be, u16, from_be_bytes }
    make_load_op! { load_x4_be, u32, from_be_bytes }
    make_load_op! { load_x8_be, u64, from_be_bytes }
}
