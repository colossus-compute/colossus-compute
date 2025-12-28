use std::cell::RefCell;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use actix_web::{get, web, App, HttpResponse, HttpServer, Responder};
use serde::ser::SerializeSeq;
use serde::Serializer;
use crate::Worker;
use crate::worker_manager::{WorkerId, WorkerInfo, WorkerMap};

#[get("/workers")]
async fn list_workers(workers: web::Data<WorkerMap>) -> impl Responder {
    #[derive(serde::Serialize)]
    struct ListEntry<'a> {
        id: WorkerId,
        info: &'a WorkerInfo,
    }

    struct ListSerializer<'a, 'b>(
        RefCell<&'a mut dyn Iterator<Item=(WorkerId, &'b Arc<Worker>)>>
    );

    impl serde::Serialize for ListSerializer<'_, '_> {

        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where S: Serializer {
            let this = &mut **self.0.borrow_mut();
            let (low, hi) = this.size_hint();
            let mut seq = serializer
                .serialize_seq(hi.filter(|&hi| hi == low))?;

            for (id, worker) in this {
                seq.serialize_element(&ListEntry {
                    id,
                    info: worker.info(),
                })?
            }

            seq.end()
        }

    }


    let response = workers.list_workers(|iter| {
        HttpResponse::Ok().json(ListSerializer(RefCell::new(iter)))
    });

    response
}

pub async fn run_http_server(
    server_bind: SocketAddr,
    workers: WorkerMap
) -> io::Result<()> {
    let factory = move || {
        App::new()
            .app_data(web::Data::<WorkerMap>::new(workers.clone()))
            .service(list_workers)
    };

    eprintln!("http server listening for: {server_bind}");

    HttpServer::new(factory)
        .bind(server_bind)?
        .run()
        .await
}