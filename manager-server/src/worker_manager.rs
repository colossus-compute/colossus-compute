use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::hash::Hash;
use std::io;
use std::collections::hash_map::Entry;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::num::NonZero;
use std::sync::Arc;
use std::task::Poll;
use parking_lot::{Mutex, RwLock};
use rand::Rng;
use serde::{Deserialize, Serialize};
use proto::buf_stream::BufStream;
use proto::{Request, SystemInfo};
use crate::rt::asyncify;

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

trait ReadWrite: Read + Write {}

impl<RW: Read + Write> ReadWrite for RW {}

pub struct Worker {
    info: WorkerInfo,
    stream: Mutex<Box<BufStream<dyn ReadWrite + Send + Sync>>>,
}

impl Debug for Worker {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut fmt = f.debug_struct("Worker");
        fmt.field("info", &self.info);
        fmt.finish_non_exhaustive()
    }
}

#[ouroboros::self_referencing]
pub struct OwnedRequest {
    buffer: Vec<u8>,
    #[borrows(mut buffer)]
    #[not_covariant]
    request: Request<'this>
}

impl Worker {
    async fn _start(stream: Box<BufStream<dyn ReadWrite + Send + Sync>>) -> io::Result<Self> {
        let mut stream = stream;
        asyncify(move || {
            let info = WorkerInfo::from(
                SystemInfo::read(&mut vec![], &mut *stream)?
            );
            Ok(Self {
                info,
                stream: Mutex::new(stream)
            })
        }).await
    }

    pub async fn start(stream: impl Read + Write + Send + Sync + 'static) -> io::Result<Self> {
        Self::_start(Box::new(BufStream::new(stream))).await
    }

    pub async fn submit(self: &Arc<Self>, request: OwnedRequest) -> Poll<io::Result<Box<[u8]>>> {
        if self.stream.try_lock().is_none() {
            return Poll::Pending
        }

        let this = Arc::clone(self);
        asyncify(move || {
            let Some(mut lock) = this.stream.try_lock() else {
                return Poll::Pending
            };

            let stream = &mut **lock;

            let output = request.with_request(|request| {
                request.write(stream)?;
                Ok::<_, io::Error>(usize::try_from(request.output_length()).map_err(drop))
            })?;
            drop(request);

            let output = output.and_then(proto::alloc::try_alloc_zeroed_slice::<u8>);
            let Ok(mut output) = output else {
                return Poll::Ready(Err(io::Error::from(io::ErrorKind::OutOfMemory)))
            };

            stream.read_exact(&mut output)?;

            Poll::Ready(Ok(output))
        }).await
    }

    pub fn info(&self) -> &WorkerInfo {
        &self.info
    }
}

#[derive(Copy, Clone, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct WorkerId(u128);

struct WorkerMapInner {
    default_worker_id: WorkerId,
    map: RwLock<HashMap<WorkerId, Arc<Worker>>>,
}

#[derive(Clone)]
pub struct WorkerMap(Arc<WorkerMapInner>);

impl WorkerMap {
    pub fn new(default_worker: Worker) -> Self {
        let default_worker_id = WorkerId(0);
        let map = HashMap::from([
            (default_worker_id, Arc::new(default_worker))
        ]);

        Self(Arc::new(WorkerMapInner {
            default_worker_id,
            map: RwLock::new(map),
        }))
    }

    pub fn list_workers<T>(
        &self,
        fun: impl FnOnce(&mut dyn Iterator<Item=(WorkerId, &Arc<Worker>)>) -> T
    ) -> T {
        let lock = self.0.map.read();
        fun(&mut lock.iter().map(|(&id, worker)| (id, worker)))
    }

    pub fn add_worker(&self, worker: Worker) -> WorkerId {
        let worker = Arc::new(worker);
        let mut lock = self.0.map.write();
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
        
        *empty_slot.insert_entry(worker).key()
    }
}


pub(super) async fn run_worker_manager(
    server_bind: SocketAddr,
    workers: WorkerMap,
) -> io::Result<()> {
    eprintln!(
        "worker manager server listening for: {server_bind}"
    );
    let tcp_listener = actix_web::rt::net::TcpListener::bind(server_bind).await?;
    loop {
        let (stream, _peer) = tcp_listener
            .accept()
            .await?;

        let workers = workers.clone();
        tokio::task::spawn_local(async move {
            let worker = Worker::start(stream.into_std()?).await?;
            workers.add_worker(worker);
            Ok::<_, io::Error>(())
        });
    }
}
