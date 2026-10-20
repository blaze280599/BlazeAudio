//! Album-art palette extraction: k-means in Oklab, scored for vividness, then pushed
//! toward bright emissive colors that read well under bloom.

pub type Rgb = [f32; 3];

/// Four linear-RGB colors: primary, secondary, accent, background tint.
pub type Palette = [Rgb; 4];

pub const PRESETS: &[(&str, Palette)] = &[
    (
        "Neon Synth",
        [[1.0, 0.08, 0.55], [0.0, 0.75, 1.0], [0.55, 0.2, 1.0], [0.05, 0.0, 0.09]],
    ),
    (
        "Ember",
        [[1.0, 0.32, 0.04], [1.0, 0.72, 0.18], [0.9, 0.06, 0.12], [0.07, 0.015, 0.0]],
    ),
    (
        "Aurora",
        [[0.08, 1.0, 0.55], [0.15, 0.45, 1.0], [0.7, 0.25, 1.0], [0.0, 0.035, 0.05]],
    ),
    (
        "Glacier",
        [[0.35, 0.85, 1.0], [0.85, 0.95, 1.0], [0.25, 0.35, 1.0], [0.0, 0.02, 0.06]],
    ),
    (
        "Acid",
        [[0.65, 1.0, 0.05], [1.0, 0.9, 0.05], [0.05, 1.0, 0.7], [0.02, 0.04, 0.0]],
    ),
];

pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn luminance(c: Rgb) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

pub fn linear_to_oklab(c: Rgb) -> Rgb {
    let l = 0.4122214708 * c[0] + 0.5363325363 * c[1] + 0.0514459929 * c[2];
    let m = 0.2119034982 * c[0] + 0.6806995451 * c[1] + 0.1073969566 * c[2];
    let s = 0.0883024619 * c[0] + 0.2817188376 * c[1] + 0.6299787005 * c[2];
    let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

pub fn oklab_to_linear(c: Rgb) -> Rgb {
    let l = c[0] + 0.3963377774 * c[1] + 0.2158037573 * c[2];
    let m = c[0] - 0.1055613458 * c[1] - 0.0638541728 * c[2];
    let s = c[0] - 0.0894841775 * c[1] - 1.2914855480 * c[2];
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    [
        (4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s).max(0.0),
        (-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s).max(0.0),
        (-0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s).max(0.0),
    ]
}

fn dist2(a: Rgb, b: Rgb) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

fn chroma(c: Rgb) -> f32 {
    (c[1] * c[1] + c[2] * c[2]).sqrt()
}

/// Re-target an Oklab color to a given lightness and minimum chroma, keeping hue.
fn vivid(c: Rgb, l: f32, min_chroma: f32, boost: f32) -> Rgb {
    let ch = chroma(c);
    let target = (ch * boost).max(min_chroma).min(0.26);
    let (a, b) = if ch > 1e-4 {
        (c[1] / ch * target, c[2] / ch * target)
    } else {
        (0.0, 0.0)
    };
    let rgb = oklab_to_linear([l, a, b]);
    // Keep hue when out of gamut by scaling down instead of clipping per channel.
    let mx = rgb[0].max(rgb[1]).max(rgb[2]);
    if mx > 1.0 {
        [rgb[0] / mx, rgb[1] / mx, rgb[2] / mx]
    } else {
        rgb
    }
}

/// `rgba` is 8-bit sRGB, row-major, `w * h * 4` bytes.
pub fn extract(rgba: &[u8], w: usize, h: usize) -> Palette {
    // Subsample to <= ~2300 pixels.
    let step = ((w * h) as f32 / 2304.0).sqrt().ceil().max(1.0) as usize;
    let mut px: Vec<Rgb> = Vec::with_capacity(2304);
    for y in (0..h).step_by(step) {
        for x in (0..w).step_by(step) {
            let i = (y * w + x) * 4;
            let c = [
                srgb_to_linear(rgba[i] as f32 / 255.0),
                srgb_to_linear(rgba[i + 1] as f32 / 255.0),
                srgb_to_linear(rgba[i + 2] as f32 / 255.0),
            ];
            px.push(linear_to_oklab(c));
        }
    }
    if px.is_empty() {
        return PRESETS[0].1;
    }

    // k-means++ style deterministic init: start at the most chromatic pixel, then farthest points.
    const K: usize = 6;
    let mut centers: Vec<Rgb> = Vec::with_capacity(K);
    let first = px
        .iter()
        .copied()
        .max_by(|a, b| chroma(*a).partial_cmp(&chroma(*b)).unwrap())
        .unwrap();
    centers.push(first);
    while centers.len() < K {
        let far = px
            .iter()
            .copied()
            .max_by(|a, b| {
                let da = centers.iter().map(|c| dist2(*a, *c)).fold(f32::MAX, f32::min);
                let db = centers.iter().map(|c| dist2(*b, *c)).fold(f32::MAX, f32::min);
                da.partial_cmp(&db).unwrap()
            })
            .unwrap();
        centers.push(far);
    }
    let mut counts = [0usize; K];
    for _ in 0..10 {
        let mut sums = [[0.0f32; 3]; K];
        counts = [0; K];
        for p in &px {
            let (mut best, mut bd) = (0, f32::MAX);
            for (ci, c) in centers.iter().enumerate() {
                let d = dist2(*p, *c);
                if d < bd {
                    bd = d;
                    best = ci;
                }
            }
            counts[best] += 1;
            for j in 0..3 {
                sums[best][j] += p[j];
            }
        }
        for ci in 0..K {
            if counts[ci] > 0 {
                let n = counts[ci] as f32;
                centers[ci] = [sums[ci][0] / n, sums[ci][1] / n, sums[ci][2] / n];
            }
        }
    }

    let total = px.len() as f32;
    let mut scored: Vec<(f32, Rgb)> = centers
        .iter()
        .zip(counts.iter())
        .filter(|(_, &n)| n > 0)
        .map(|(c, &n)| {
            let share = n as f32 / total;
            let light = 1.0 - (c[0] - 0.65).abs();
            (share.sqrt() * (chroma(*c) + 0.015) * light, *c)
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

    // Pick up to three hue-diverse colors.
    let mut picks: Vec<Rgb> = Vec::new();
    for (_, c) in &scored {
        let hue = c[2].atan2(c[1]);
        let diverse = picks.iter().all(|p| {
            let ph = p[2].atan2(p[1]);
            let mut d = (hue - ph).abs();
            if d > std::f32::consts::PI {
                d = 2.0 * std::f32::consts::PI - d;
            }
            d > 0.45 || chroma(*p) < 0.03 || chroma(*c) < 0.03
        });
        if diverse {
            picks.push(*c);
        }
        if picks.len() == 3 {
            break;
        }
    }
    // Not enough distinct hues on the cover: take the next-best clusters that are at least
    // perceptibly different, then fall back to analogous shifts so we stay on-brand.
    for (_, c) in &scored {
        if picks.len() >= 3 {
            break;
        }
        if picks.iter().all(|p| dist2(*p, *c) > 0.004) && chroma(*c) > 0.03 {
            picks.push(*c);
        }
    }
    let mut shift = 0.55f32;
    while picks.len() < 3 {
        let base = picks.first().copied().unwrap_or([0.6, 0.15, -0.1]);
        let (s, c) = shift.sin_cos();
        picks.push([base[0], base[1] * c - base[2] * s, base[1] * s + base[2] * c]);
        shift = -shift * 1.4;
    }

    let max_chroma = picks.iter().map(|c| chroma(*c)).fold(0.0, f32::max);
    let greyscale = max_chroma < 0.045;

    let mut pal: Palette = [
        vivid(picks[0], 0.74, 0.12, 1.5),
        vivid(picks[1], 0.80, 0.10, 1.5),
        vivid(picks[2], 0.68, 0.12, 1.6),
        [0.0; 3],
    ];
    // Background: darkest dominant cluster, kept very dark but tinted.
    let darkest = centers
        .iter()
        .zip(counts.iter())
        .filter(|(_, &n)| n > 0)
        .min_by(|a, b| a.0[0].partial_cmp(&b.0[0]).unwrap())
        .map(|(c, _)| *c)
        .unwrap_or(picks[0]);
    let bg_src = if chroma(darkest) > 0.02 { darkest } else { picks[0] };
    pal[3] = vivid(bg_src, 0.2, 0.04, 1.0);

    if greyscale {
        // Monochrome covers look washed out as light; tint them with a cool neon base.
        let preset = PRESETS[3].1;
        for i in 0..4 {
            for j in 0..3 {
                pal[i][j] = pal[i][j] * 0.2 + preset[i][j] * 0.8;
            }
        }
    }
    pal
}

pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}
