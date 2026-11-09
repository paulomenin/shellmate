fn main() {
    let t = std::time::Instant::now();
    let a = ureq::AgentBuilder::new().build();
    let r1 = t.elapsed();
    let t = std::time::Instant::now();
    let _ = a.get("https://127.0.0.1:1/").call(); // refused: includes TLS config build if eager
    let r2 = t.elapsed();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let t = std::time::Instant::now();
    let _c = reqwest::Client::new();
    let r3 = t.elapsed();
    let _ = rt;
    eprintln!("ureq agent build={:?} refused-https call={:?} reqwest Client::new={:?}", r1, r2, r3);
}
