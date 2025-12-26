// use quinn::Connection;
// use tokio::io;
// use wasmtime::{Linker, Module, Store};
// use proto::{Request, ResponseBuffer};
//
//
// const _: &str = r#"
//     (module
//         (func (export "kernel-run") (param i32 i32 i32 i64) (res i32 i32)
//
//         )
//     )
// "#;
//
// fn run(
//     engine: &wasmtime::Engine,
//     request: Request<'_>,
// ) -> anyhow::Result<ResponseBuffer> {
//     let module = Module::new(engine, request.wasm_object())?;
//
//     let mut linker = Linker::new(engine);
//
//     let mut store: Store<ResponseBuffer> = Store::new(
//         &engine,
//         request.make_output_buffer()?
//     );
//
//     linker.module(&mut store, "task", &module)?;
//
//     // Modules can be compiled through either the text or binary format
//     let wat = r#"
//         (module
//             (import "task" "kernel-run"
//                     (func $kernel-call (param i32 i32 i32 i64) (res i32 i32))
//             )
//
//             (func (export "run-worker") (param i64 i32 i32)
//
//             )
//         )
//     "#;
//
//     let hello = instance
//         .get_typed_func::<(u64, u32, u32), ()>(&mut store, "hello")?;
//
//     // And finally we can call the wasm!
//     hello.call(&mut store, ())?;
//
//     Ok(store.into_data())
// }
//
// fn run_worker(
//     engine: wasmtime::Engine,
//     endpoint: Connection,
//     runtime: tokio::runtime::Handle
// ) -> io::Result<()> {
//     // TODO: buffer management
//     //       re-use the buffer but prevernt spikes
//     loop {
//         let mut buffer = Vec::new();
//         let (tx, rx) = endpoint.open_bi().await?;
//         let mut rx = io::BufReader::new(rx);
//         let (request, _len) = Request::read(&mut buffer, &mut rx).await?;
//         request.name();
//     }
// }

fn main() {
    use clap::Parser;
    #[derive(Parser, Debug)]
    #[command(version, about, long_about = None)]
    struct Args {
        endpoint: String,
    }

}
