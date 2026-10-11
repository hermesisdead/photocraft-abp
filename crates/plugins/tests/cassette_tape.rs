//! The Cassette Tape example plug-in (`examples/plugins/cassette-tape-c`) run through the real
//! sandbox: deterministic per seed, seamless across bands, every control does something, and the
//! worst case fits the host's instruction budget in every colour mode.

use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_plugins::{Limits, Plugin};
use photocraft_raster::Surface;
use serde_json::{Value, json};

const WASM: &[u8] = include_bytes!("fixtures/cassette_tape.wasm");
const W: i32 = 150;
const H: i32 = 100;

fn plugin(limits: Limits) -> Plugin {
    Plugin::load(WASM, limits).expect("cassette tape loads")
}

fn blank(fmt: PixelFormat) -> Surface {
    let mut s = Surface::new(fmt);
    let px: Vec<f32> = (0..fmt.channels()).map(|c| if fmt.alpha && c == fmt.channels() - 1 { 1.0 } else { 0.5 }).collect();
    s.fill_rect(Rect::new(0, 0, W, H), &px);
    s
}

fn run(p: &Plugin, fmt: PixelFormat, params: Value) -> Surface {
    p.apply(&blank(fmt), Rect::new(0, 0, W, H), None, &params).expect("runs")
}

fn differs(a: &Surface, b: &Surface) -> bool {
    (0..H).any(|y| (0..W).any(|x| a.pixel(x, y) != b.pixel(x, y)))
}

/// Every wear control off: a clean cassette.
fn clean() -> Value {
    json!({"shell_texture": 0, "label_texture": 0, "handwriting": 0, "label_wear": 0, "label_stains": 0, "label_rips": 0,
           "tape_texture": 0, "window_glare": 0, "scratches": 0, "scuffs": 0, "dust": 0, "grime": 0, "edge_wear": 0})
}

fn with(mut base: Value, pairs: &[(&str, Value)]) -> Value {
    for (k, v) in pairs {
        base[*k] = v.clone();
    }
    base
}

#[test]
fn manifest_lists_the_shell_label_tape_and_wear_controls() {
    let m = plugin(Limits::default()).manifest().clone();
    assert_eq!(m.id, "org.photocraft.cassette-tape");
    let names: Vec<_> = m.params.iter().map(|(n, _)| n.as_str()).collect();
    for want in [
        "seed",
        "shell_color",
        "shell_finish",
        "label_paper",
        "label_style",
        "handwriting",
        "label_rips",
        "rip_side",
        "tape_type",
        "tape_amount",
        "scratches",
        "scratch_count",
        "scuffs",
        "dust",
        "grime",
        "edge_wear",
        "backdrop",
        "shadow",
    ] {
        assert!(names.contains(&want), "missing {want}: {names:?}");
    }
}

#[test]
fn every_control_changes_the_picture() {
    let p = plugin(Limits::default());
    let base = run(&p, PixelFormat::RGBA8, clean());
    for (k, v) in [
        ("shell_color", json!("red")),
        ("shell_finish", json!("speckled")),
        ("shell_texture", json!(100)),
        ("label_paper", json!("kraft")),
        ("label_style", json!("double stripe")),
        ("label_texture", json!(100)),
        ("label_wear", json!(100)),
        ("label_stains", json!(100)),
        ("label_rips", json!(100)),
        ("tape_type", json!("chrome")),
        ("tape_amount", json!(90)),
        ("tape_texture", json!(100)),
        ("window_glare", json!(100)),
        ("scratches", json!(100)),
        ("scuffs", json!(100)),
        ("dust", json!(100)),
        ("grime", json!(100)),
        ("edge_wear", json!(100)),
        ("backdrop", json!("charcoal")),
        ("angle", json!(20)),
        ("size", json!(40)),
        ("shadow", json!(0)),
    ] {
        let out = run(&p, PixelFormat::RGBA8, with(clean(), &[(k, v.clone())]));
        assert!(differs(&base, &out), "{k}={v} changed nothing");
    }
}

#[test]
fn seed_repeats_and_changes_the_wear() {
    let p = plugin(Limits::default());
    let wear = json!({"scratches": 80, "scuffs": 60, "dust": 60, "label_rips": 60, "label_stains": 60, "grime": 50});
    let a = run(&p, PixelFormat::RGBA8, with(wear.clone(), &[("seed", json!(5))]));
    assert_eq!(a, run(&p, PixelFormat::RGBA8, with(wear.clone(), &[("seed", json!(5))])));
    assert!(differs(&a, &run(&p, PixelFormat::RGBA8, with(wear, &[("seed", json!(6))]))));
}

#[test]
fn bands_are_seamless_in_colour_and_grayscale() {
    let all = json!({"seed": 3, "angle": 9, "scratches": 70, "scuffs": 50, "dust": 40, "grime": 40, "edge_wear": 50, "label_rips": 50,
        "label_wear": 50, "label_stains": 50, "handwriting": 70, "label_style": "stripe and ruled"});
    let tiny = Limits { band_bytes: (W as usize) * 4 * 4 * 5, ..Limits::default() };
    for fmt in [PixelFormat::RGBA8, PixelFormat::GRAYA8, PixelFormat { alpha: false, ..PixelFormat::GRAYA8 }] {
        let band = if fmt.channels() == 4 { tiny.clone() } else { Limits { band_bytes: (W as usize) * 4 * 5, ..Limits::default() } };
        assert_eq!(run(&plugin(Limits::default()), fmt, all.clone()), run(&plugin(band), fmt, all.clone()), "{fmt:?}");
    }
}

#[test]
fn transparent_backdrop_leaves_the_corners_clear() {
    let p = plugin(Limits::default());
    let out = run(&p, PixelFormat::RGBA8, with(clean(), &[("backdrop", json!("transparent")), ("shadow", json!(0))]));
    assert!(out.pixel(0, 0)[3] < 0.01, "corner is transparent");
    assert!(out.pixel(W / 2, H / 2)[3] > 0.99, "the cassette is opaque");
    let solid = run(&p, PixelFormat::RGBA8, clean());
    assert!(solid.pixel(0, 0)[3] > 0.99);
}

#[test]
fn other_colour_modes_are_refused_not_corrupted() {
    let p = plugin(Limits::default());
    assert!(p.apply(&blank(PixelFormat::CMYKA8), Rect::new(0, 0, W, H), None, &json!({})).is_err());
}

#[test]
fn the_worst_case_fits_the_instruction_budget_in_every_colour_mode() {
    let all = json!({"scratches": 100, "scratch_count": 48, "scratch_style": "mixed", "scuffs": 100, "dust": 100, "grime": 100, "edge_wear": 100,
        "label_rips": 100, "rip_side": "all", "label_wear": 100, "label_stains": 100, "handwriting": 100, "label_style": "stripe and ruled",
        "label_texture": 100, "shell_texture": 100, "shell_finish": "speckled", "tape_texture": 100, "window_glare": 100, "angle": 12});
    let p = plugin(Limits::default());
    for fmt in
        [PixelFormat::RGBA8, PixelFormat { alpha: false, ..PixelFormat::RGBA8 }, PixelFormat::GRAYA8, PixelFormat { alpha: false, ..PixelFormat::GRAYA8 }]
    {
        p.apply(&blank(fmt), Rect::new(0, 0, W, H), None, &all).unwrap_or_else(|e| panic!("{fmt:?}: {e}"));
    }
}
