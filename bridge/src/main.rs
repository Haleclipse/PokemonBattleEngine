fn main() {
    let port = std::env::var("BRIDGE_PORT").unwrap_or_else(|_| "8787".into());
    battle_bridge::serve(&port);
}
