pub fn asyncify<T: Send + 'static>(fun: impl FnOnce() -> T + Send + 'static) -> impl Future<Output=T> {
    let jh = actix_web::rt::task::spawn_blocking(fun);

    async move {
        jh
            .await
            .unwrap_or_else(|err| std::panic::resume_unwind(err.into_panic()))
    }
}
