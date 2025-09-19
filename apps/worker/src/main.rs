use std::time::Duration;
use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter, Registry};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    tracing::info!("kb-worker starting");

    // 占位：轮询数据库/队列拉取 IndexJob，执行切分/嵌入/索引/图谱
    loop {
        tracing::debug!("polling jobs (stub)");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

fn init_tracing() {
    let fmt_layer = fmt::layer().with_target(false);
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap();
    let subscriber = Registry::default().with(filter).with(fmt_layer);
    tracing::subscriber::set_global_default(subscriber).ok();
}
