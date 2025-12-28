use proto::Request;

mod alloc;
mod request_solver;

pub fn run_worker(request: Request) -> anyhow::Result<()> {
    request_solver::resolve_request(request).map(drop)
}