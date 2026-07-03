//! Dev tool: render a RON score to a WAV without the full CLI.
//! `cargo run -p cochlea-render --example render_ron -- score.ron out.wav`
//! (The `cochlea render` subcommand is the real interface; this exists so
//! the render crate is exercisable standalone.)

fn main() {
    let mut args = std::env::args().skip(1);
    let (score_path, out_path) = (
        args.next()
            .expect("usage: render_ron <score.ron> <out.wav>"),
        args.next()
            .expect("usage: render_ron <score.ron> <out.wav>"),
    );
    let text = std::fs::read_to_string(&score_path).expect("read score");
    let score = cochlea_score::Score::from_ron(&text).expect("parse score");
    let rendered = cochlea_render::render(&score).expect("render");
    rendered.write_wav(&out_path).expect("write wav");
    eprintln!(
        "rendered {} frames at {} Hz -> {}",
        rendered.frames(),
        rendered.sample_rate().0,
        out_path
    );
}
