use std::io::{BufRead, Write};
use proto::{Request, SystemInfo};

mod request_solver;

trait BufStream: BufRead + Write {}
impl<S: BufRead + Write> BufStream for S {}


// prevent monomorphization
#[inline(never)]
fn _run_worker(request_stream: &mut dyn BufStream) -> anyhow::Result<()> {
    let mut request_buffer = Vec::new();
    
    let info = SystemInfo::current_system(&mut request_buffer);
    info.write(request_stream)?;
    
    const SRINK_COUNT_TRIGGER_MAX: u32 = 16;

    let mut cnt = 0;
    loop {
        let request = Request::read(&mut request_buffer, request_stream)?;
        dbg!(request.name(), request.output_length());
        let buffer = request_solver::resolve_request(request)?;
        eprintln!("resolved request!");
        request_stream.write_all(&buffer)?;
        request_stream.flush()?;
        request_buffer.clear();

        if request_buffer.capacity().div_ceil(4) >= request_buffer.len() {
            if cnt == SRINK_COUNT_TRIGGER_MAX {
                request_buffer.shrink_to_fit();
                continue
            }

            cnt += 1
        }
    }
}

#[inline(never)]
pub fn run_worker(request_stream: &mut (impl BufRead + Write)) {
    _run_worker(request_stream).unwrap()
}
