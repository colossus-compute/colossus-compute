use std::net::TcpStream;
use clap::Parser;



#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    endpoint: String,
}


fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let tcp_connection = TcpStream::connect(args.endpoint)?;
    worker::run_worker(&mut proto::buf_stream::BufStream::new(tcp_connection))
}
