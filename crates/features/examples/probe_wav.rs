//! Dev tool: probe a WAV without the full CLI.
//! `cargo run -p cochlea-features --example probe_wav -- input.wav`
//! (The `cochlea probe` subcommand is the real interface; this exists so
//! the features crate is exercisable standalone.)

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: probe_wav <input.wav>");
    let audio = cochlea_features::Audio::from_wav(std::path::Path::new(&path)).expect("read wav");
    let report = cochlea_features::probe(&audio, &cochlea_features::ProbeOpts::default());
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("serialize")
    );
}
