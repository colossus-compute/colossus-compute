use std::io::Read;
use std::sync::OnceLock;
use wasmtime::{Config, Engine, Module, OptLevel};
use proto::alloc::alloc_zeroed_slice;

fn decompress_bridge() -> Box<[u8]> {
    const COMPRESSED_WORKER_OBJ: &[u8] =
        include_bytes!(concat!(env!("OUT_DIR"), "/wasm-worker-bridge.wasm.obj"));

    const PARSED_OBJ: (usize, &[u8]) = {
        let (len, worker_gz) = COMPRESSED_WORKER_OBJ.split_first_chunk().unwrap();
        let len = u64::from_le_bytes(*len);
        let len_usize = match usize::BITS < u64::BITS {
            // usize::BITS >= u64::BITS so casting a u64 to a usize is always fine
            false => len as usize,
            true => {
                // usize::BITS < u64::BITS so casting usize::MAX to a u64 is fine
                let max = usize::MAX as u64;
                assert!(len <= max);
                len as usize
            }
        };

        assert!(
            len_usize <= isize::MAX.cast_unsigned(),
            "object too large to fit in memory"
        );

        (len_usize, worker_gz)
    };

    let (obj_len, worker_gz) = PARSED_OBJ;

    let mut object = alloc_zeroed_slice::<u8>(obj_len);
    let mut worker_file = flate2::bufread::DeflateDecoder::new(worker_gz);
    worker_file.read_exact(&mut object).unwrap();
    let hit_eof = worker_file.read(&mut [0]).unwrap() == 0;
    assert!(hit_eof);

    object
}

pub(super) fn get_cached() -> &'static (Engine, Module) {
    static WASM_WORKER_BRIDGE: OnceLock<(Engine, Module)> = OnceLock::new();
    WASM_WORKER_BRIDGE.get_or_init(|| {
        let engine = Engine::new(
            Config::default()
                .cranelift_opt_level(OptLevel::Speed)
                .compiler_inlining(true)
                .consume_fuel(true)
        ).unwrap();
        let module = Module::new(&engine, decompress_bridge())
            .unwrap();

        (engine, module)
    })
}