#[tokio::main]
async fn main() {
    if let Err(error) = dockerctl::cli::run().await {
        eprintln!("错误: {error}");
        std::process::exit(1);
    }
}
