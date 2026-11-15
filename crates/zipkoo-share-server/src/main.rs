//! Run: ZIPKOO_SHARE_DIR=/var/lib/zipkoo ZIPKOO_SHARE_ADDR=0.0.0.0:8080 zipkoo-share-server
//! Put it behind HTTPS (Caddy, nginx, or a platform proxy); links must be https.
use zipkoo_share_server::{app, sweep, Config};

#[tokio::main]
async fn main() {
    let env = |k: &str| std::env::var(k).ok();
    let cfg = Config {
        dir: env("ZIPKOO_SHARE_DIR")
            .unwrap_or_else(|| "./shares".into())
            .into(),
        max_bytes: env("ZIPKOO_SHARE_MAX_BYTES")
            .and_then(|v| v.parse().ok())
            .unwrap_or(20 << 30),
        max_hours: env("ZIPKOO_SHARE_MAX_HOURS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(24 * 7),
    };
    std::fs::create_dir_all(&cfg.dir).expect("create storage dir");
    let addr = env("ZIPKOO_SHARE_ADDR").unwrap_or_else(|| "127.0.0.1:8080".into());
    let (router, st) = app(cfg);
    tokio::spawn(async move {
        loop {
            sweep(&st).await;
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
        }
    });
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    println!("Zipkoo share server on http://{addr}");
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .unwrap();
}
