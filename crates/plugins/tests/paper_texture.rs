//! The Paper Texture example plug-in (`examples/plugins/paper-texture-c`) run through the real
//! sandbox: it must be deterministic per seed, seamless across bands, honour every intensity, and
//! stay inside the host's limits.

use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_plugins::{Limits, Plugin};
use photocraft_raster::Surface;
use serde_json::{Value, json};

const WASM: &[u8] = include_bytes!("fixtures/paper_texture.wasm");
const W: i32 = 96;
const H: i32 = 72;

fn plugin(limits: Limits) -> Plugin {
    Plugin::load(WASM, limits).expect("paper texture loads")
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

/// Every intensity at zero: a flat sheet of paper.
fn off() -> Value {
    json!({"texture":0,"wrinkles":0,"creases":0,"folds":0,"stains":0,"aging":0,"burns":0,"rips":0})
}

fn with(base: Value, k: &str, v: Value) -> Value {
    let mut b = base;
    b[k] = v;
    b
}

fn differs(a: &Surface, b: &Surface) -> bool {
    (0..H).any(|y| (0..W).any(|x| a.pixel(x, y) != b.pixel(x, y)))
}

#[test]
fn manifest_has_every_effect_with_an_intensity() {
    let m = plugin(Limits::default()).manifest().clone();
    assert_eq!(m.id, "org.photocraft.paper-texture");
    let names: Vec<_> = m.params.iter().map(|(n, _)| n.as_str()).collect();
    for want in ["seed", "paper", "texture", "wrinkles", "creases", "folds", "stains", "aging", "burns", "rips"] {
        assert!(names.contains(&want), "missing {want}: {names:?}");
    }
}

#[test]
fn zero_intensity_gives_flat_paper_and_each_effect_changes_it() {
    let p = plugin(Limits::default());
    let flat = run(&p, PixelFormat::RGBA8, off());
    let first = flat.pixel(0, 0);
    assert!(first[3] > 0.99);
    assert!((0..H).all(|y| (0..W).all(|x| flat.pixel(x, y) == first)), "all effects off is one flat colour");
    for (k, extra) in [
        ("texture", json!({})),
        ("wrinkles", json!({})),
        ("creases", json!({})),
        ("folds", json!({})),
        ("stains", json!({"stain_count": 8})),
        ("aging", json!({})),
        ("burns", json!({"burn_through": 100})),
        ("rips", json!({})),
    ] {
        let mut params = with(off(), k, json!(100));
        for (ek, ev) in extra.as_object().expect("object") {
            params[ek] = ev.clone();
        }
        assert!(differs(&flat, &run(&p, PixelFormat::RGBA8, params)), "{k} had no effect at 100");
    }
}

#[test]
fn the_same_seed_repeats_and_another_seed_differs() {
    let p = plugin(Limits::default());
    let a = run(&p, PixelFormat::RGBA8, json!({"seed": 5}));
    assert_eq!(a, run(&p, PixelFormat::RGBA8, json!({"seed": 5})));
    assert!(differs(&a, &run(&p, PixelFormat::RGBA8, json!({"seed": 6}))));
}

#[test]
fn bands_are_seamless() {
    let all = json!({"seed": 9, "wrinkles": 80, "creases": 80, "folds": 80, "fold_layout": "both", "stains": 80, "aging": 80, "burns": 60, "rips": 40});
    let one = run(&plugin(Limits::default()), PixelFormat::RGBA8, all.clone());
    let tiny = Limits { band_bytes: (W as usize) * 4 * 4 * 5, ..Limits::default() };
    assert_eq!(one, run(&plugin(tiny), PixelFormat::RGBA8, all));
}

#[test]
fn rips_and_burn_through_make_transparent_pixels_only_when_asked() {
    let p = plugin(Limits::default());
    let flat = run(&p, PixelFormat::RGBA8, off());
    assert!((0..H).all(|y| (0..W).all(|x| flat.pixel(x, y)[3] > 0.99)));
    let torn = run(&p, PixelFormat::RGBA8, with(off(), "rips", json!(100)));
    assert!(torn.pixel(0, 0)[3] < 0.5 || torn.pixel(W / 2, 0)[3] < 0.5, "the torn edge cuts the sheet");
    assert!(torn.pixel(W / 2, H / 2)[3] > 0.99, "the middle stays");
    let burnt = run(&p, PixelFormat::RGBA8, with(with(off(), "burns", json!(100)), "burn_through", json!(100)));
    assert!(burnt.pixel(0, 0)[3] < 0.5, "burnt-through corner");
}

#[test]
fn multiply_mode_keeps_the_layer_and_other_colour_modes_work_or_refuse() {
    let p = plugin(Limits::default());
    let m = run(&p, PixelFormat::RGBA8, with(off(), "apply", json!("multiply over layer")));
    let px = m.pixel(10, 10);
    assert!(px[0] < 0.5 && px[0] > 0.3, "0.5 grey times cream paper: {px:?}");
    let gray = run(&p, PixelFormat::GRAYA8, json!({}));
    assert!(gray.pixel(5, 5)[0] > 0.0);
    let rgb = run(&p, PixelFormat { alpha: false, ..PixelFormat::RGBA8 }, json!({"rips": 100}));
    assert!(rgb.pixel(0, 0)[0] < 0.2, "torn-away corner of an opaque layer shows the dark backdrop");
    assert!(p.apply(&blank(PixelFormat::CMYKA8), Rect::new(0, 0, W, H), None, &json!({})).is_err(), "CMYK is refused, not corrupted");
}

#[test]
fn the_worst_case_fits_the_instruction_budget_in_every_colour_mode() {
    let all = json!({"wrinkles": 100, "creases": 100, "crease_count": 24, "folds": 100, "fold_count": 6, "fold_layout": "both",
        "stains": 100, "stain_count": 8, "aging": 100, "burns": 100, "burn_spots": 8, "rips": 100, "texture": 100});
    let p = plugin(Limits::default());
    let gray = PixelFormat { alpha: false, ..PixelFormat::GRAYA8 };
    for fmt in [PixelFormat::RGBA8, PixelFormat { alpha: false, ..PixelFormat::RGBA8 }, PixelFormat::GRAYA8, gray] {
        p.apply(&blank(fmt), Rect::new(0, 0, W, H), None, &all).unwrap_or_else(|e| panic!("{fmt:?}: {e}"));
    }
}

#[test]
fn grayscale_bands_are_seamless_too() {
    let all = json!({"seed": 4, "wrinkles": 80, "creases": 80, "folds": 80, "stains": 80, "aging": 80, "burns": 60, "rips": 40});
    let tiny = Limits { band_bytes: (W as usize) * 4 * 5, ..Limits::default() };
    for fmt in [PixelFormat::GRAYA8, PixelFormat { alpha: false, ..PixelFormat::GRAYA8 }] {
        assert_eq!(run(&plugin(Limits::default()), fmt, all.clone()), run(&plugin(tiny.clone()), fmt, all.clone()), "{fmt:?}");
    }
}
