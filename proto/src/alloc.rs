use std::alloc::Layout;
use std::ptr::NonNull;
use bytemuck::Zeroable;



// Safety: T is zeroable and therfore this is a valid repr of T
pub fn try_alloc_zeroed_slice<T: Zeroable>(len: usize) -> Result<Box<[T]>, ()> {
    let Ok(layout) = Layout::array::<T>(len) else { 
        return Err(())
    };
    
    if layout.size() == 0 { 
        return Ok(unsafe { Box::<[T; 0]>::from_raw(core::ptr::dangling_mut()) })
    }
    
    match NonNull::new(unsafe { std::alloc::alloc_zeroed(layout) }) {
        Some(ptr) => Ok(unsafe {
            Box::from_raw(core::ptr::slice_from_raw_parts_mut(
                ptr.as_ptr().cast::<T>(),
                len
            ))
        }),
        None => Err(()),
    }
}

// Safety: T is zeroable and therfore this is a valid repr of T
pub fn alloc_zeroed_slice<T: Zeroable>(len: usize) -> Box<[T]> {
    unsafe { Box::new_zeroed_slice(len).assume_init() }
}
