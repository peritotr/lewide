//! Development only: `cargo run -p dbclient --features bridge --bin bridge [port]`.
fn main() -> std::io::Result<()> {
    let port = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(1430);
    dbclient_lib::bridge::serve(port)
}
