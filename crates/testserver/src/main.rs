//! Standalone test server for manual testing and benchmarks.
//!
//! `crowdm-testserver [ADDR]` (default `127.0.0.1:8765`), then open
//! `http://ADDR/page` in a browser.

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let addr = std::env::args().nth(1).unwrap_or_else(|| "127.0.0.1:8765".into());
    let server = crowdm_testserver::TestServer::bind(addr.parse().expect("invalid address")).await?;
    println!("CrowDM test server listening on http://{}", server.addr());
    println!("Test page: {}", server.url("/page"));
    std::future::pending::<()>().await;
    Ok(())
}
