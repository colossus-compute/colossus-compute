use std::collections::HashMap;
use std::fmt::{Debug, Display, Formatter};
use std::hash::Hash;
use std::{io, mem};
use std::collections::hash_map::Entry;
use std::io::IoSlice;
use std::mem::ManuallyDrop;
use std::net::SocketAddr;
use std::num::NonZero;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};
use parking_lot::RwLock;
use rand::Rng;
use serde::{Deserializer, Serializer};
use serde::de::{Error, Unexpected};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufStream, ReadBuf};
use tokio::sync::oneshot;
use tokio::task::AbortHandle;
use proto::{Request, SystemInfo};

#[derive(Debug, serde::Serialize)]
pub struct WorkerInfo {
    pub cpu_name: Box<str>,
    pub cpu_vendor_id: Box<str>,
    pub cpu_freq: Option<NonZero<u64>>,
    pub total_memory: Option<NonZero<u64>>,
    pub cpu_count: Option<NonZero<u32>>,
}

impl<'a> From<SystemInfo<'a>> for WorkerInfo {
    fn from(value: SystemInfo) -> Self {
        Self {
            cpu_name: Box::<str>::from(value.cpu_name()),
            cpu_vendor_id: Box::<str>::from(value.cpu_vendor_id()),
            cpu_freq: value.cpu_freq(),
            total_memory: value.total_memory(),
            cpu_count: value.cpu_count(),
        }
    }
}

trait AsyncReadWrite: AsyncRead + AsyncWrite {}

impl<RW: AsyncRead + AsyncWrite> AsyncReadWrite for RW {}


#[ouroboros::self_referencing]
pub struct OwnedRequest {
    buffer: actix_web::web::Bytes,
    #[borrows(buffer)]
    #[not_covariant]
    request: Request<'this>
}

impl OwnedRequest {
    pub fn decode(buffer: actix_web::web::Bytes) -> io::Result<Self> {
        Self::try_new(
            buffer,
            |buffer| Request::read_slice(buffer)
        )
    }

    pub fn name(&self) -> &str {
        self.with(|x| x.request.name())
    }
}


enum WorkerState {
    Waiting,
    PendingRequest((OwnedRequest, oneshot::Sender<io::Result<Box<[u8]>>>)),
    Quit(io::Result<()>)
}

struct SharedWorkerData {
    state: parking_lot::Mutex<WorkerState>,
    notification: tokio::sync::Notify
}

struct SharedWorker(Arc<SharedWorkerData>);

enum QueueWorkErrorType {
    Busy,
    Quit(io::Result<()>)
}

struct QueueWorkError<T> {
    request: T,
    ty: QueueWorkErrorType
}

impl SharedWorker {
    fn quit_inner(data: &SharedWorkerData, res: io::Result<()>) {
        let mut lock = data.state.lock();
        *lock = WorkerState::Quit(res);
        drop(lock);
        data.notification.notify_one()
    }

    pub fn quit(self, res: io::Result<()>) {
        let Self(arc) = &*ManuallyDrop::new(self);
        let arc = unsafe { std::ptr::read(arc) };
        Self::quit_inner(&arc, res)
    }

    #[allow(clippy::type_complexity)]
    pub fn submit(&self, request: OwnedRequest) -> Result<oneshot::Receiver<io::Result<Box<[u8]>>>, QueueWorkError<OwnedRequest>> {
        let mut lock = self.0.state.lock();
        let err_ty = match &mut *lock {
            state @ WorkerState::Waiting => {
                let (tx, rx) = oneshot::channel();
                *state = WorkerState::PendingRequest((request, tx));
                drop(lock);
                self.0.notification.notify_one();
                return Ok(rx)
            }
            WorkerState::PendingRequest(_) => QueueWorkErrorType::Busy,
            state @ WorkerState::Quit(_) => {
                let WorkerState::Quit(res) = mem::replace(state, WorkerState::Quit(Ok(()))) else {
                    unreachable!()
                };

                QueueWorkErrorType::Quit(res)
            }
        };

        Err(QueueWorkError {
            request,
            ty: err_ty
        })
    }
}

impl Drop for SharedWorker {
    fn drop(&mut self) {
        Self::quit_inner(&self.0, match std::thread::panicking() {
            true => Err(io::Error::other("worker panicked")),
            false => Ok(())
        })
    }
}


pub struct Worker {
    info: WorkerInfo,
    shared: SharedWorker
}

impl Debug for Worker {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut fmt = f.debug_struct("Worker");
        fmt.field("info", &self.info);
        fmt.finish_non_exhaustive()
    }
}

struct DynStream(Pin<Box<dyn AsyncReadWrite + Send + Sync>>);

impl AsyncRead for DynStream {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        self.0.as_mut().poll_read(cx, buf)
    }
}

impl AsyncWrite for DynStream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        self.0.as_mut().poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.0.as_mut().poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.0.as_mut().poll_shutdown(cx)
    }

    fn poll_write_vectored(mut self: Pin<&mut Self>, cx: &mut Context<'_>, bufs: &[IoSlice<'_>]) -> Poll<io::Result<usize>> {
        self.0.as_mut().poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.0.is_write_vectored()
    }
}

pub async fn worker_wait_channel<T>(request: oneshot::Receiver<io::Result<T>>) -> io::Result<T> {
    match request.await {
        Ok(res) => res,
        Err(_) => Err(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "worker abruptly stopped"
        ))
    }
}

impl Worker {
    async fn _start(stream: BufStream<DynStream>) -> io::Result<Self> {
        let mut stream = stream;
        let info = WorkerInfo::from(SystemInfo::read_async(&mut vec![], &mut stream).await?);

        let shared = Arc::new(SharedWorkerData {
            state: parking_lot::Mutex::new(WorkerState::Waiting),
            notification: tokio::sync::Notify::new()
        });

        tokio::spawn({
            let shared = SharedWorker(Arc::clone(&shared));
            async move {
                let runner = async {
                    let shared = &*shared.0;

                    loop {
                        let request = {
                            let mut guard = shared.state.lock();
                            match &mut *guard {
                                WorkerState::Waiting => None,
                                state @ WorkerState::PendingRequest(_) => {
                                    let state = mem::replace(state, WorkerState::Waiting);
                                    let WorkerState::PendingRequest(request) = state else {
                                        unreachable!()
                                    };
                                    Some(request)
                                }
                                WorkerState::Quit(_result) => {
                                    // other end of the pipe quit
                                    return Ok(())
                                }
                            }
                        };

                        if let Some((request, sender)) = request {
                            eprintln!("sending '{}' through worker IPC", request.name());

                            let stream = &mut stream;
                            let result = async move {
                                let (write, output_len) = request.with(|request| {
                                    (
                                        stream.write_all(request.buffer),
                                        usize::try_from(request.request.output_length()).map_err(drop)
                                    )
                                });

                                write.await?;
                                stream.flush().await?;
                                drop(request);



                                let output = output_len.and_then(proto::alloc::try_alloc_zeroed_slice::<u8>);
                                let Ok(mut output) = output else {
                                    return Err(io::Error::from(io::ErrorKind::OutOfMemory))
                                };
                                stream.read_exact(&mut output).await?;

                                Ok(output)
                            };

                            // we dont care if the reciver recived his request or not
                            // we DO NOT quit early so that the stream isn't
                            // in an inconsistent state
                            let _ = sender.send(result.await.inspect_err(|err| dbg!(err, ()).1));
                        }

                        shared.notification.notified().await;
                    }
                };

                let ret = runner.await;
                shared.quit(ret)
            }
        });

        Ok(Self {
            info,
            shared: SharedWorker(shared)
        })
    }

    pub async fn start(stream: impl AsyncRead + AsyncWrite + Send + Sync + 'static) -> io::Result<Self> {
        Self::_start(BufStream::new(DynStream(Box::pin(stream)))).await
    }

    pub fn info(&self) -> &WorkerInfo {
        &self.info
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct WorkerId(u128);

impl Display for WorkerId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

impl Debug for WorkerId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}

impl FromStr for WorkerId {
    type Err = <u128 as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        u128::from_str_radix(s, 16).map(Self)
    }
}



impl serde::Serialize for WorkerId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{self}"))
    }
}

impl<'de> serde::Deserialize<'de> for WorkerId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        <String as serde::Deserialize>::deserialize(deserializer)
            .and_then(|str| WorkerId::from_str(&str).map_err(|_| {
                D::Error::invalid_value(Unexpected::Str(&str), &"a worker id")
            }))
    }
}



pub struct WorkerManagerInner {
    default_worker_id: WorkerId,
    workers: RwLock<HashMap<WorkerId, Arc<Worker>>>,
    worker_available: tokio::sync::Notify,
    #[allow(clippy::type_complexity)]
    queue: flume::Sender<(OwnedRequest, oneshot::Sender<io::Result<Box<[u8]>>>)>,
    queue_poller: OnceLock<AbortHandle>
}

#[derive(Clone)]
pub struct WorkerManager(Arc<WorkerManagerInner>);

impl WorkerManager {
    pub fn new(default_worker: Worker) -> Self {
        let default_worker_id = WorkerId(0);
        let map = HashMap::from([
            (default_worker_id, Arc::new(default_worker))
        ]);

        let (tx, rx) = flume::bounded(16);
        let inner = Arc::new(WorkerManagerInner {
            default_worker_id,
            workers: RwLock::new(map),
            worker_available: tokio::sync::Notify::new(),
            queue: tx,
            queue_poller: OnceLock::new(),
        });

        let inner_clone = Arc::clone(&inner);
        let jh = tokio::spawn(async move {
            let mut workers_that_quit = vec![];

            while let Ok((mut request, mut sender)) = rx.recv_async().await {
                let reciver = {
                    'outer: loop {
                        let clean_workers = |workers_that_quit: &mut Vec<_>| {
                            if !workers_that_quit.is_empty() {
                                let mut lock = inner_clone.workers.write();
                                for (id, res) in workers_that_quit.drain(..) {
                                    assert_ne!(id, inner_clone.default_worker_id);

                                    // TODO propper logging
                                    let _ = res;
                                    lock.remove(&id);
                                }
                            }
                        };

                        {
                            let lock = inner_clone.workers.read();
                            let workers = &*lock;
                            let name = request.name().to_string();
                            for (&id, worker) in workers.iter() {
                                match worker.shared.submit(request) {
                                    Ok(reciver) => {
                                        eprintln!("submitted '{name}' to worker work");
                                        clean_workers(&mut workers_that_quit);
                                        break 'outer reciver
                                    },
                                    Err(err) => {
                                        if let QueueWorkErrorType::Quit(res) = err.ty {
                                            workers_that_quit.push((id, res))
                                        }
                                        request = err.request
                                    }
                                }
                            }
                        }

                        clean_workers(&mut workers_that_quit);

                        inner_clone.worker_available.notified().await
                    }
                };

                tokio::select! {
                    _ = sender.closed() => {},
                    requests_res = worker_wait_channel(reciver) => {
                        let _ = sender.send(requests_res);
                    }
                }
            }
        });

        inner.queue_poller.set(jh.abort_handle()).unwrap();

        Self(inner)
    }

    pub fn add_worker(&self, worker: Worker) -> WorkerId {
        let worker = Arc::new(worker);
        let mut lock = self.0.workers.write();
        let empty_slot = {
            let mut rng = rand::rng();
            let mut failed_attempts = 0_i16;
            loop {
                if let Entry::Vacant(slot) = lock.entry(WorkerId(rng.random())) {
                    break slot
                }

                let Some(new_attempts) = failed_attempts.checked_add(1) else {
                    unreachable!("failed to get entropy in reasonable time")
                };
                failed_attempts = new_attempts;
            }
        };
        
        let key = *empty_slot.insert_entry(worker).key();
        self.0.worker_available.notify_one();
        key
    }

    pub fn list_workers<T>(
        &self,
        fun: impl FnOnce(&mut dyn Iterator<Item=(WorkerId, &Arc<Worker>)>) -> T
    ) -> T {
        let lock = self.0.workers.read();
        fun(&mut lock.iter().map(|(&id, worker)| (id, worker)))
    }

    pub fn get_worker(&self, worker_id: WorkerId) -> Option<Arc<Worker>> {
        let lock = self.0.workers.read();
        lock.get(&worker_id).cloned()
    }

    pub async fn run(&self, request: OwnedRequest) -> io::Result<Box<[u8]>> {
        let (tx, rx) = oneshot::channel();
        self.0.queue.send((request, tx))
            .expect("worker manager thread should not panic");

        worker_wait_channel(rx).await
    }
}


pub(super) async fn run_worker_manager(
    server_bind: SocketAddr,
    workers: WorkerManager,
) -> io::Result<()> {
    eprintln!(
        "worker manager server listening for: {server_bind}"
    );

    let tcp_listener = tokio::net::TcpListener::bind(server_bind).await?;
    loop {
        let (stream, _peer) = tcp_listener
            .accept()
            .await?;

        let workers = workers.clone();
        tokio::spawn(async move {
            let worker = Worker::start(stream).await?;
            workers.add_worker(worker);
            Ok::<_, io::Error>(())
        });
    }
}
