use std::ops::Deref;
use wasmtime::{Caller, Linker, Module, Store};
use proto::Request;
use crate::request_solver::buffer::{InputBuffer, OutputBuffer};

mod cached_wasm;
mod buffer;

fn run_range(
    start: u64,
    length: u32,
    range_start: u64,
    range_length: u32,
    task: &Module,
    input: InputBuffer,
    output: OutputBuffer
) -> anyhow::Result<()> {
    let (engine, bridge) = cached_wasm::get_cached();

    let (input_len, output_len) = (input.len(), output.len());

    let mut store = Store::new(engine, (input, output));
    let mut linker = Linker::new(engine);


    macro_rules! define_load_op {
        ($op: ident => |$input: ident, $index: ident| $op_expr: expr) => {
            linker.func_wrap("runner", stringify!($op), |caller: Caller<'_, (InputBuffer, OutputBuffer)>, $index: u32| {
                let ($input, _) = caller.data();
                $op_expr.map_err(|()| anyhow::Error::new(wasmtime::Trap::ArrayOutOfBounds))
            })?;
        };
    }

    define_load_op! { load_input => |inp, idx| inp.load(idx).map(u32::from) }

    define_load_op! { load_input_x2_le => |inp, idx| inp.load_x2_le(idx).map(u32::from) }
    define_load_op! { load_input_x4_le => |inp, idx| inp.load_x4_le(idx) }
    define_load_op! { load_input_x8_le => |inp, idx| inp.load_x8_le(idx) }

    define_load_op! { load_input_x2_be => |inp, idx| inp.load_x2_be(idx).map(u32::from) }
    define_load_op! { load_input_x4_be => |inp, idx| inp.load_x4_be(idx) }
    define_load_op! { load_input_x8_be => |inp, idx| inp.load_x8_be(idx) }



    macro_rules! define_store_op {
        ($op: ident => |$output: ident, $index: ident, $byte: ident: $ty: ty| $op_expr: expr) => {
            linker.func_wrap("runner", stringify!($op), |caller: Caller<'_, (InputBuffer, OutputBuffer)>, $index: u32, $byte: $ty| {
                let (_, $output) = caller.data();
                $op_expr.map_err(|()| anyhow::Error::new(wasmtime::Trap::ArrayOutOfBounds))

            })?;
        };
    }


    define_store_op! { store_output => |out, idx, byte: u32| out.store(idx, byte as u8) }

    define_store_op! { store_output_x2_le => |out, idx, word: u32| out.store_x2_le(idx, word as u16) }
    define_store_op! { store_output_x4_le => |out, idx, dword: u32| out.store_x4_le(idx, dword) }
    define_store_op! { store_output_x8_le => |out, idx, qword: u64| out.store_x8_le(idx, qword) }

    define_store_op! { store_output_x2_be => |out, idx, word: u32| out.store_x2_be(idx, word as u16) }
    define_store_op! { store_output_x4_be => |out, idx, dword: u32| out.store_x4_be(idx, dword) }
    define_store_op! { store_output_x8_be => |out, idx, qword: u64| out.store_x8_be(idx, qword) }


    linker.module(&mut store, "task", task)?;

    let instance = linker.instantiate(&mut store, bridge)?;

    // we want (u32::MAX/range_length) * length
    // which is the same as (u32::MAX * length) / range_length
    let fuel = (u64::from(u32::MAX) * u64::from(length))
        .div_ceil(u64::from(range_length));

    store.set_fuel(fuel.saturating_mul(3))?;

    let run_worker = instance.get_typed_func::<(u64, u32, u64, u32, u32, u32), ()>(&mut store, "run_worker")?;
    run_worker.call(&mut store, (
        start,
        length,
        range_start,
        range_length,
        input_len,
        output_len
    ))
}


pub(super) fn resolve_request(request: Request) -> anyhow::Result<impl Deref<Target=[u8]> + Send + Sync + 'static> {
    let (engine, _) = cached_wasm::get_cached();
    let task = Module::new(engine, request.wasm_object())?;
    let start = request.start();
    let length = request.length();

    let input_buffer = InputBuffer::from_slice(request.input())?;
    let output_buffer = OutputBuffer::new(request.output_length())?;

    let mut error = parking_lot::Mutex::new(None);

    rayon::scope(|scope| {
        if length == 0 {
            return
        }

        let num_workers = u32::try_from(rayon::current_num_threads())
            .unwrap_or(u32::MAX)
            .clamp(1, length.div_ceil(64));

        let base = length / num_workers;
        let remainder = length % num_workers;

        let mut current = start;

        for i in 0..num_workers {
            let extra = if i < remainder { 1 } else { 0 };
            let len = base + extra;

            // only way len == 0 is if extra == 0
            // and if base == 0; but base never changes if its 0 now it will always be 0
            // and when extra becomes 0 it will always be 0
            if len == 0 {
                break
            }

            let error = &error;
            let task = &task;
            let input_buffer = input_buffer.clone();
            let output_buffer = output_buffer.clone();

            let run = move || {
                let res = run_range(
                    current,
                    len,
                    start,
                    length,
                    task,
                    input_buffer,
                    output_buffer,
                );

                if let Err(err) = res {
                    *error.lock() = Some(err)
                }
            };

            match i == num_workers - 1 {
                true => run(),
                false => {
                    scope.spawn(|_| run());
                    current += u64::from(len);
                }
            }
        }
    });

    if let Some(err) = error.get_mut().take() {
        return Err(err)
    }


    output_buffer.into_bytes().map_err(|()| anyhow::anyhow!("failed to aquire output buffer"))
}


#[cfg(test)]
mod test {
    use rand::{Rng, SeedableRng};
    use super::*;

    macro_rules! wat_kernel_raw {
        ($($body:literal),*) => {
            concat!(
r#"(module
  ;; Host APIs:
  ;; - load_input(index: u32) -> u32   (returns the byte at input[index])
  ;; - store_output(index: u32, byte: u32) (stores the low 8 bits to output[index])
  (import "runner" "load_input"   (func $load_input   (param i32) (result i32)))
  (import "runner" "store_output" (func $store_output (param i32 i32)))

  ;; - load_input_x2_be(index: u32) -> u32   (returns the word at input[index..index+2] in little endian order)
  ;; - load_input_x4_be(index: u32) -> u32   (returns the dword at input[index..index+4] in little endian order)
  ;; - load_input_x2_be(index: u32) -> u64   (returns the qword at input[index..index+8] in little endian order)
  (import "runner" "load_input_x2_le" (func $load_input_x2_le (param i32) (result i32)))
  (import "runner" "load_input_x4_le" (func $load_input_x4_le (param i32) (result i32)))
  (import "runner" "load_input_x8_le" (func $load_input_x8_le (param i32) (result i64)))

  ;; - load_input_x2_be(index: u32) -> u32   (returns the word at input[index..index+2] in big endian order)
  ;; - load_input_x4_be(index: u32) -> u32   (returns the dword at input[index..index+4] in big endian order)
  ;; - load_input_x2_be(index: u32) -> u64   (returns the qword at input[index..index+8] in big endian order)
  (import "runner" "load_input_x2_be" (func $load_input_x2_be (param i32) (result i32)))
  (import "runner" "load_input_x4_be" (func $load_input_x4_be (param i32) (result i32)))
  (import "runner" "load_input_x8_be" (func $load_input_x8_be (param i32) (result i64)))

  ;; - store_output_x2_le(index: u32, word: u32) (stores the low 16 bits to output[index..index+2] in little endian order)
  ;; - store_output_x4_le(index: u32, dword: u32) (stores dword bits to output[index..index+4] in little endian order)
  ;; - store_output_x8_le(index: u32, qword: u64) (stores qword bits to output[index..index+8] in little endian order)
  (import "runner" "store_output_x2_le" (func $store_output_x2_le (param i32 i32)))
  (import "runner" "store_output_x4_le" (func $store_output_x4_le (param i32 i32)))
  (import "runner" "store_output_x8_le" (func $store_output_x8_le (param i32 i64)))

  ;; - store_output_x2_be(index: u32, word: u32) (stores the low 16 bits to output[index..index+2] in big endian order)
  ;; - store_output_x4_be(index: u32, dword: u32) (stores dword bits to output[index..index+4] in big endian order)
  ;; - store_output_x8_be(index: u32, qword: u64) (stores qword bits to output[index..index+8] in big endian order)
  (import "runner" "store_output_x2_be" (func $store_output_x2_be (param i32 i32)))
  (import "runner" "store_output_x4_be" (func $store_output_x4_be (param i32 i32)))
  (import "runner" "store_output_x8_be" (func $store_output_x8_be (param i32 i64)))

  ;; Signature (Rust-side ABI):
  ;; unsafe fn kernel_call(
  ;;   input_len: u32,
  ;;   output_len: u32,
  ;;   start_range: u64,
  ;;   range_length: u32,
  ;;   current: u64,
  ;; )
  (func (export "kernel_call")
    (param $input_len i32)
    (param $output_len i32)
    (param $start_range i64)
    (param $range_length i32)
    (param $current i64)
"#,
                $($body,)*
r#"
  )
)
"#
            )
        };
    }

    macro_rules! wat_kernel_i {
        (locals: $locals:literal, body: $body:literal $(,)?) => {
            wat_kernel_raw!(
                r#"(local $i i32)"#,
                $locals,
                r#"(local.set $i (i32.wrap_i64 (local.get $current)))"#,
                $body
            )
        };
        (body: $body:literal $(,)?) => {
            wat_kernel_i!(
                locals: "",
                body: $body
            )
        };
        ($body:literal $(,)?) => {
            wat_kernel_i!(body: $body)
        };
    }


    /// `out[i*2] = out[i*2+1] = input[i] * 2`
    #[test]
    fn double_and_splat_numbers() {
        const MODULE: &str = wat_kernel_i!{
            locals: r#"(local $tmp i32) (local $dbl i32) (local $out0 i32)"#,
            body: r#"
                (local.set $tmp (call $load_input (local.get $i)))
                (local.set $dbl (i32.shl (local.get $tmp) (i32.const 1)))
                (local.set $out0 (i32.shl (local.get $i) (i32.const 1)))

                (call $store_output (local.get $out0) (local.get $dbl))
                (call $store_output
                  (i32.add (local.get $out0) (i32.const 1))
                  (local.get $dbl)
                )
            "#
        };

        let input = (0..32 * 1024).map(|i| i as u8).collect::<Vec<_>>();

        let request = Request::new(
            "double numbers",
            0,
            u32::try_from(input.len()).unwrap(),
            u32::try_from(input.len() * 2).unwrap(),
            &input,
            MODULE.as_bytes(),
        );

        let output = resolve_request(request.unwrap()).unwrap();
        assert!(output.iter().copied().eq(
            input.iter().flat_map(|&b| [b.wrapping_mul(2); 2])
        ));
    }

    /// `output[i] = input[i]`
    #[test]
    fn copy_input_identity() {
        const MODULE: &str = wat_kernel_i!{
            locals: "(local $tmp i32)",
            body: r#"
                (local.set $tmp (call $load_input (local.get $i)))
                (call $store_output (local.get $i) (local.get $tmp))
            "#
        };

        let input = std::array::from_fn::<_, 4096, _>(|i| (i as u8).wrapping_mul(3));

        let request = Request::new(
            "copy input",
            0,
            u32::try_from(input.len()).unwrap(),
            u32::try_from(input.len()).unwrap(),
            &input,
            MODULE.as_bytes(),
        );

        let output = resolve_request(request.unwrap()).unwrap();
        assert!(output.iter().copied().eq(input.iter().copied()));
    }

    /// `output[i] = input[input_len - 1 - i]`
    #[test]
    fn reverse_input_using_input_len_param() {
        const MODULE: &str = wat_kernel_i!{
            locals: "(local $tmp i32) (local $idx i32)",
            body: r#"
                ;; idx = input_len - 1 - i
                (local.set $idx
                  (i32.sub
                    (i32.sub (local.get $input_len) (i32.const 1))
                    (local.get $i)
                  )
                )

                (local.set $tmp (call $load_input (local.get $idx)))
                (call $store_output (local.get $i) (local.get $tmp))
            "#
        };

        let input = std::array::from_fn::<_, 2048, _>(|i| (i as u8).wrapping_add(17));

        let request = Request::new(
            "reverse input",
            0,
            u32::try_from(input.len()).unwrap(),
            u32::try_from(input.len()).unwrap(),
            &input,
            MODULE.as_bytes(),
        );

        let output = resolve_request(request.unwrap()).unwrap();
        assert!(
            output
                .iter()
                .copied()
                .eq(input.iter().rev().copied())
        );
    }

    /// `output[i] = input[i] XOR (i as u8)`
    #[test]
    fn xor_with_index_uses_current() {
        const MODULE: &str = wat_kernel_i!{
            locals: "(local $tmp i32) (local $out i32)",
            body: r#"
                (local.set $tmp (call $load_input (local.get $i)))
                (local.set $out (i32.xor (local.get $tmp) (local.get $i)))
                (call $store_output (local.get $i) (local.get $out))
            "#
        };

        let input = std::array::from_fn::<_, 8192, _>(|i| (i as u8).rotate_left(3));

        let request = Request::new(
            "xor with index",
            0,
            u32::try_from(input.len()).unwrap(),
            u32::try_from(input.len()).unwrap(),
            &input,
            MODULE.as_bytes(),
        );

        let output = resolve_request(request.unwrap()).unwrap();
        assert!(
            output
                .iter()
                .copied()
                .eq(input.iter().copied().enumerate().map(|(i, b)| b ^ (i as u8)))
        );
    }

    /// ```no_compile
    /// output[i*3 + 0] = input[i] + 0
    /// output[i*3 + 1] = input[i] + 1
    /// output[i*3 + 2] = input[i] + 2
    /// ```
    #[test]
    fn fanout_triple_write() {
        const MODULE: &str = wat_kernel_i!{
            locals: "(local $tmp i32) (local $base i32)",
            body: r#"
                (local.set $tmp (call $load_input (local.get $i)))
                (local.set $base (i32.mul (local.get $i) (i32.const 3)))

                (call $store_output (local.get $base) (local.get $tmp))
                (call $store_output
                  (i32.add (local.get $base) (i32.const 1))
                  (i32.add (local.get $tmp) (i32.const 1))
                )
                (call $store_output
                  (i32.add (local.get $base) (i32.const 2))
                  (i32.add (local.get $tmp) (i32.const 2))
                )
            "#
        };


        let input = std::array::from_fn::<_, 4096, _>(|i| (i as u8).wrapping_mul(7));
        let out_len = input.len() * 3;

        let request = Request::new(
            "fanout triple",
            0,
            u32::try_from(input.len()).unwrap(),
            u32::try_from(out_len).unwrap(),
            &input,
            MODULE.as_bytes(),
        );

        let output = resolve_request(request.unwrap()).unwrap();
        assert!(output.iter().copied().eq(
            input.iter().copied().flat_map(|b| [b, b.wrapping_add(1), b.wrapping_add(2)])
        ));
    }

    /// Copies only a subrange into output[0..range_length].
    ///
    /// This module is written to work whether `current` is:
    /// - absolute (start_range..start_range+range_length), OR
    /// - relative (0..range_length),
    ///   as long as start_range > 0.
    #[test]
    fn copy_subrange_to_compact_output() {
        const MODULE: &str = wat_kernel_raw!{r#"
            (local $cur i32)
            (local $base i32)
            (local $abs i32)
            (local $out i32)
            (local $tmp i32)

            (local.set $cur  (i32.wrap_i64 (local.get $current)))
            (local.set $base (i32.wrap_i64 (local.get $start_range)))

            ;; If cur < base, treat cur as relative; otherwise treat as absolute.
            (if (i32.lt_u (local.get $cur) (local.get $base))
              (then
                (local.set $abs (i32.add (local.get $base) (local.get $cur)))
                (local.set $out (local.get $cur))
              )
              (else
                (local.set $abs (local.get $cur))
                (local.set $out (i32.sub (local.get $cur) (local.get $base)))
              )
            )

            (local.set $tmp (call $load_input (local.get $abs)))
            (call $store_output (local.get $out) (local.get $tmp))
        "#};


        let input = std::array::from_fn::<_, 4096, _>(|i| (i as u8).wrapping_add(1));

        let start_range: u64 = 39;
        let range_length: usize = 527;

        let request = Request::new(
            "copy subrange",
            start_range,
            u32::try_from(range_length).unwrap(),
            u32::try_from(range_length).unwrap(),
            &input,
            MODULE.as_bytes(),
        );

        let output = resolve_request(request.unwrap()).unwrap();

        let start = usize::try_from(start_range).unwrap();
        let expected = &input[start..start + range_length];
        assert!(output.iter().copied().eq(expected.iter().copied()));
    }

    /// `Intentionally loads input[input_len] (OOB) to ensure traps/errors propagate.`
    #[test]
    fn oob_load_should_error() {
        const MODULE: &str = wat_kernel_i!{r#"
            ;; OOB: valid last index is input_len - 1
            (drop (call $load_input (local.get $input_len)))
        "#};

        let input = std::array::from_fn::<_, 16, _>(|i| i as u8);

        let request = Request::new(
            "oob load",
            0,
            1, // only need one invocation (i == 0)
            1,
            &input,
            MODULE.as_bytes(),
        );

        assert!(resolve_request(request.unwrap()).is_err());
    }

    /// `Intentionally stores output[output_len] (OOB) to ensure traps/errors propagate.`
    #[test]
    fn oob_store_should_error() {
        const MODULE: &str = wat_kernel_raw!(r#"
            ;; OOB: valid last index is output_len - 1
            (call $store_output (local.get $output_len) (i32.const 123))
        "#);

        let input = std::array::from_fn::<_, 16, _>(|i| i as u8);

        let request = Request::new(
            "oob store",
            0,
            1,
            1,
            &input,
            MODULE.as_bytes(),
        );

        assert!(resolve_request(request.unwrap()).is_err());
    }

    /// Input:  [u32; N] as little-endian bytes
    /// Output: [f32; N] as little-endian bytes where out[i] = (input[i] as f32) * 3.4 + 3.0
    #[test]
    fn u32_le_times_3_14_plus_3_to_f32_le() {
        const MODULE: &str = wat_kernel_i! {
            locals: r#"
                (local $in_base i32)
                (local $out_base i32)
                (local $u i32)
                (local $f f32)
                (local $bits i32)
            "#,
            body: r#"
                ;; in_base = i * 4
                (local.set $in_base (i32.shl (local.get $i) (i32.const 2)))
                (local.set $out_base (local.get $in_base))

                ;; u32 = load_input_x4_le(in_base)
                (local.set $u (call $load_input_x4_le (local.get $in_base)))

                ;; f = (u as f32) * 3.14 + 3.0
                (local.set $f (f32.convert_i32_u (local.get $u)))
                (local.set $f (f32.mul (local.get $f) (f32.const 3.14)))
                (local.set $f (f32.add (local.get $f) (f32.const 3.0)))

                ;; store f32 bits as little-endian dword
                (local.set $bits (i32.reinterpret_f32 (local.get $f)))
                (call $store_output_x4_le (local.get $out_base) (local.get $bits))
            "#
        };

        let n: usize = 2 * 1024 * 1024;

        // input u32s -> bytes (LE)
        let input_u32s: Vec<u32> = {
            let mut input = vec![0_u32; n];
            rand::rngs::SmallRng::from_os_rng().fill(&mut *input);
            input
        };

        let mut input = Vec::with_capacity(n * 4);
        for &v in &input_u32s {
            input.extend_from_slice(&v.to_le_bytes());
        }

        let request = Request::new(
            "u32->f32 scale add",
            0,
            u32::try_from(n).unwrap(),                 // iterations = number of u32s
            u32::try_from(size_of_val(input_u32s.as_slice())).unwrap(),    // output bytes
            &input,
            MODULE.as_bytes(),
        );

        let output = resolve_request(request.unwrap()).unwrap();

        let (chunks, remainder) = output.as_chunks::<4>();
        assert!(remainder.is_empty());

        let mut output = chunks
            .iter()
            .copied()
            .map(f32::from_le_bytes);

        let mut expected = input_u32s
            .iter()
            .map(#[allow(clippy::approx_constant)] |&v| ((v as f32) * 3.14) + 3.0);

        let mut iter = std::iter::zip(&mut output, &mut expected);
        assert!(iter.all(|(x, y)| (x - y).abs() <= 0.1));
        assert!(output.next().is_none() && expected.next().is_none());
    }
}