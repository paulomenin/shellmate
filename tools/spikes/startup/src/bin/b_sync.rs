// sync: toml config parse + (lazy) ureq agent; does NOT hit the network unless --net URL
use serde::Deserialize;
#[derive(Deserialize, Debug)] struct Cfg { version: u32, #[serde(default)] providers: std::collections::BTreeMap<String, toml::Value> }
fn main() {
    let t = std::time::Instant::now();
    let s = std::fs::read_to_string(std::env::args().nth(1).unwrap()).unwrap();
    let c: Cfg = toml::from_str(&s).unwrap();
    let parse = t.elapsed();
    let mut net = None;
    if let Some(u) = std::env::args().nth(2) {
        let t = std::time::Instant::now();
        let r = ureq::get(&u).call().map(|r| r.status());
        net = Some((t.elapsed(), r.map_err(|e| e.to_string())));
    }
    eprintln!("v={} providers={} parse={:?} net={:?}", c.version, c.providers.len(), parse, net);
}
