#![cfg(target_arch = "wasm32")]
#![no_std]

use core::panic::PanicInfo;

#[panic_handler]
pub fn handler(_: &PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

#[link(wasm_import_module = "task")]
unsafe extern "C" {
    unsafe fn kernel_call(
        input_len: u32,
        output_len: u32,

        start_range: u64,
        range_length: u32,

        current: u64,
    );
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn run_worker(
    start: u64,
    length: u32,

    range_start: u64,
    range_length: u32,

    input_len: u32,
    output_len: u32,
) {
    let end = unsafe { start.unchecked_add(u64::from(length)) };
    for i in (start..end).rev() {
        unsafe {
            kernel_call(
                input_len,
                output_len,

                range_start,
                range_length,
                i
            )
        }
    }
}