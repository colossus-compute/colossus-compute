(module
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
    (local $i i32)
    (local $in_base i32)
    (local $out_base i32)
    (local $u i32)
    (local $f f32)
    (local $bits i32)

    (local.set $i (i32.wrap_i64 (local.get $current)))
    ;; in_base = i * 4
    (local.set $in_base (i32.shl (local.get $i) (i32.const 2)))
    (local.set $out_base (local.get $in_base))

    ;; u32 = load_input_x4_le(in_base)
    (local.set $u (call $load_input_x4_le (local.get $in_base)))

    ;; f = u * 3.14 + 3.0
    (local.set $f (f32.reinterpret_i32 (local.get $u)))
    (local.set $f (f32.mul (local.get $f) (f32.const 3.14)))
    (local.set $f (f32.add (local.get $f) (f32.const 3.0)))

    ;; store f32 bits as little-endian dword
    (local.set $bits (i32.reinterpret_f32 (local.get $f)))
    (call $store_output_x4_le (local.get $out_base) (local.get $bits))
  )
)