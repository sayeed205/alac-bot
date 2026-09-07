//! One-off live smoke check for the production transport + catalog mapping
//! against the real iTunes API. NOT part of the test suite (network).
//!
//! Run: `cargo run -p engine --example live_check`

use engine::catalog::{Catalog, ReqwestTransport};

#[tokio::main]
async fn main() {
    let catalog = Catalog::new(ReqwestTransport::new());

    let meta = catalog
        .fetch_track_meta("1440841730", "us")
        .await
        .expect("live track lookup");
    println!(
        "track: {} — {} [{}] {}s",
        meta.artist, meta.title, meta.album, meta.duration_secs
    );
    println!("  artwork: {}", meta.artwork_url);
    println!("  release: {}", meta.release_date);

    let search = catalog
        .search_catalog("the weeknd blinded lights", 3, "us")
        .await
        .expect("live search");
    println!("search: {} results", search.len());
    for t in search.iter().take(3) {
        println!("  {} — {} (id {})", t.artist, t.title, t.id);
    }
}
