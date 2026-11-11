//! Prints a demo station's state and settings as the apps see them.
fn main() {
    let dir = std::env::temp_dir().join("ws-dump");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("whistlestop.toml"), "demo = true\ndemo_real_mercury = false\ncallsign = \"N0PHN\"\n").unwrap();
    whistlestop_mobile::start(dir.to_str().unwrap()).unwrap();
    std::thread::sleep(std::time::Duration::from_secs(4));
    whistlestop_mobile::send_json(r#"{"Connect":"K7BOT"}"#).unwrap();
    std::thread::sleep(std::time::Duration::from_secs(20));
    println!("{}", whistlestop_mobile::state_json());
    println!("{}", whistlestop_mobile::config_json());
    println!("{}", whistlestop_mobile::rows_json().len());
}
