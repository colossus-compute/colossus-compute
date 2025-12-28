use std::io;
use std::net::{IpAddr, SocketAddr};
use proto::buf_stream::BufStream;
use clap::Parser;
use tokio::try_join;
use crate::http::run_http_server;
use crate::worker_manager::{run_worker_manager, Worker, WorkerMap};

mod http;
mod worker_manager;
mod rt;
mod io_stream;

fn real_main(
    http_addr: SocketAddr,
    worker_manager_addr: SocketAddr,
) -> io::Result<()> {
    let (pipe1, pipe2) = {
        #[cfg(not(unix))]
        {
            let (rx1, tx1) = io::pipe()?;
            let (rx2, tx2) = io::pipe()?;
            let pipe1 = Stream::new(rx1, tx2);
            let pipe2 = Stream::new(rx2, tx1);
            (pipe1, pipe2)
        }

        #[cfg(unix)]
        {
            std::os::unix::net::UnixStream::pair()?
        }
    };

    // use thread spawn for long lived thread
    std::thread::spawn(move || worker::run_worker(&mut BufStream::new(pipe2)).unwrap());

    let runtime = actix_web::rt::Runtime::new()?;

    // spawn in task so it is part of the work stealing queue
    runtime.block_on(async {
        let default_worker = Worker::start(pipe1).await?;
        let workers = WorkerMap::new(default_worker);

        let worker_manager = run_worker_manager(
            worker_manager_addr,
            workers.clone()
        );
        let http_server = run_http_server(http_addr, workers);

        try_join!(worker_manager, http_server).map(|((), ())| ())
    })
}


#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// adress to listen to
    #[arg(long)]
    workers_listen: IpAddr,

    /// port to listen to
    #[arg(long, default_value_t = 42370)]
    workers_port: u16,

    /// adress to listen to
    #[arg(long)]
    http_listen: IpAddr,

    /// port to listen to
    #[arg(long, default_value_t = 42371)]
    http_port: u16,
}

fn main() -> io::Result<()> {
    let args = Args::parse();

    real_main(
        (args.http_listen, args.http_port).into(),
        (args.workers_listen, args.workers_port).into(),
    )
}
