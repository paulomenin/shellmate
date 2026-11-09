use serde::Deserialize;
#[derive(Deserialize, Debug)] struct Cfg { version: u32, #[serde(default)] providers: std::collections::BTreeMap<String, toml::Value> }
fn main() {
    let t0 = std::time::Instant::now();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let rt_t = t0.elapsed();
    rt.block_on(async {
        let t = std::time::Instant::now();
        let s = std::fs::read_to_string(std::env::args().nth(1).unwrap()).unwrap();
        let c: Cfg = toml::from_str(&s).unwrap();
        let parse = t.elapsed();
        let mut net = None;
        if let Some(u) = std::env::args().nth(2) {
            let t = std::time::Instant::now();
            let cl = reqwest::Client::new();
            let r = cl.get(&u).send().await.map(|r| r.status());
            net = Some((t.elapsed(), r.map_err(|e| e.to_string())));
        }
        eprintln!("v={} providers={} rt={:?} parse={:?} net={:?}", c.version, c.providers.len(), rt_t, parse, net);
    });
}
