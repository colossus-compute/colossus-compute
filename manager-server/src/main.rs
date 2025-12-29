use std::io;
use std::net::{IpAddr, SocketAddr};
use proto::buf_stream::BufStream;
use clap::Parser;
use tokio::try_join;
use crate::http::run_http_server;
use crate::worker_manager::{run_worker_manager, Worker, WorkerManager};

mod http;
mod worker_manager;

#[cfg(not(unix))]
mod io_stream;

fn real_main(
    http_addr: SocketAddr,
    worker_manager_addr: SocketAddr,
) -> io::Result<()> {
    let runtime = actix_web::rt::Runtime::new()?;

    let (pipe_async, pipe_sync) = {
        #[cfg(not(unix))]
        {
            let (rx1, tx1) = io::pipe()?;
            let (rx2, tx2) = io::pipe()?;
            let (mut pipe1_rx, mut pipe1_tx) = (rx1, tx2);
            let pipe2 = io_stream::Stream::new(rx2, tx1);
            let (async_pipe1, async_pipe2) =
                tokio::io::duplex(4 * 1024 * 1024);


            std::thread::spawn(move || {
                let (rx, tx) = tokio::io::split(async_pipe2);
                std::thread::scope(|s| {
                    fn run_with_rt(func: impl FnOnce() -> io::Result<()>) {
                        let runtime = actix_web::rt::Runtime::new().unwrap();
                        let _enter = runtime.tokio_runtime().enter();
                        func().unwrap()
                    }

                    let _jh = s.spawn(move || {
                        run_with_rt(move || {
                            io::copy(
                                &mut tokio_util::io::SyncIoBridge::new(rx),
                                &mut pipe1_tx
                            ).map(drop)
                        })
                    });

                    run_with_rt(move || {
                        io::copy(
                            &mut pipe1_rx,
                            &mut tokio_util::io::SyncIoBridge::new(tx),
                        ).map(drop)
                    })
                })
            });

            (async_pipe1, pipe2)
        }

        #[cfg(unix)]
        {
            let (p1, p2) = std::os::unix::net::UnixStream::pair()?;
            let _guard = runtime.tokio_runtime().enter();
            p1.set_nonblocking(true)?;
            (tokio::net::UnixStream::from_std(p1)?, p2)
        }
    };


    // use thread spawn for long lived thread
    std::thread::spawn(move || worker::run_worker(&mut BufStream::new(pipe_sync)));

    // spawn in task so it is part of the work stealing queue
    runtime.block_on(async {
        let default_worker = Worker::start(pipe_async).await?;
        let workers = WorkerManager::new(default_worker);

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
