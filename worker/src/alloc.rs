use bytemuck::Zeroable;

// Safety: T is zeroable and therfore this is a valid repr of T
pub(crate) fn alloc_zeroed_slice<T: Zeroable>(len: usize) -> Box<[T]> {
    unsafe { Box::new_zeroed_slice(len).assume_init() }
}
