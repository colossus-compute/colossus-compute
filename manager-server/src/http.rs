use std::cell::RefCell;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use actix_web::{get, post, web, App, HttpResponse, HttpServer, Responder};
use actix_web::http::header::HeaderValue;
use futures_lite::StreamExt;
use serde::ser::SerializeSeq;
use serde::Serializer;
use crate::Worker;
use crate::worker_manager::{OwnedRequest, WorkerId, WorkerInfo, WorkerManager};

#[get("/workers")]
async fn list_workers(workers: web::Data<WorkerManager>) -> impl Responder {
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

    workers.list_workers(|iter| {
        HttpResponse::Ok().json(ListSerializer(RefCell::new(iter)))
    })
}


#[get("/worker/status/{id}")]
async fn worker_status(id: web::Path<WorkerId>, workers: web::Data<WorkerManager>) -> impl Responder {
    let id = id.into_inner();
    match workers.get_worker(id) {
        Some(worker) => {
            HttpResponse::Ok().json(serde_json::json!({
                "id": id,
                "info": worker.info()
            }))
        }
        None => HttpResponse::NotFound().body(format!("could not find worker with id: `{id}`"))
    }
}

#[post("/submit")]
async fn submit(mut body: web::Payload, workers: web::Data<WorkerManager>) -> HttpResponse {
    let mut bytes = web::BytesMut::new();
    while let Some(item) = body.next().await {
        let item = match item {
            Ok(item) => item,
            Err(err) => return HttpResponse::from_error(err)
        };
        bytes.extend_from_slice(&item);
    }

    let request = match OwnedRequest::decode(bytes.freeze()) {
        Ok(request) => request,
        Err(err) => return HttpResponse::BadRequest().body(err.to_string())
    };

    eprintln!("recived: {}", request.name());
    match workers.run(request).await {
        Ok(body) => {
            let header = const {
                HeaderValue::from_static("application/octet-stream")
            };
            HttpResponse::Ok().content_type(header).body(body.into_vec())
        },
        Err(err) => HttpResponse::from_error(dbg!(err))
    }
}


pub async fn run_http_server(
    server_bind: SocketAddr,
    workers: WorkerManager
) -> io::Result<()> {
    let factory = move || {
        App::new()
            .app_data(web::Data::<WorkerManager>::new(workers.clone()))
            .service(list_workers)
            .service(worker_status)
            .service(submit)
    };

    eprintln!("http server listening for: {server_bind}");

    HttpServer::new(factory)
        .bind(server_bind)?
        .run()
        .await
}