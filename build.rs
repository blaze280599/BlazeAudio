//! Embeds the exe's Windows resources: the app icon (Explorer, shortcuts, Start, taskbar
//! pins) and the version info (file Properties → Details, and the name Task Manager shows).
//! The resource file is written directly in the `.res` format and handed to the linker, so
//! the build needs no resource compiler from the Windows SDK.

use std::path::PathBuf;

/// Icon sizes in the icon group: the shell's small, medium and large sizes at common DPI
/// scales, plus 256 for Explorer's large views.
const ICON_SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];
/// Resource id of the icon group; `main.rs` loads the window icons by this id.
const ICON_GROUP_ID: u16 = 1;

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
const LANG_EN_US: u16 = 0x0409;
const CODEPAGE_UNICODE: u16 = 1200;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = Vec::new();
    // A .res file starts with an empty entry that marks it as 32-bit.
    push_resource(&mut res, 0, 0, 0, &[]);

    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(ICON_SIZES.len() as u16).to_le_bytes());
    for (i, &n) in ICON_SIZES.iter().enumerate() {
        let rgba = icon_rgba(n);
        let image = if n >= 256 { png(&rgba, n) } else { dib(&rgba, n) };
        let id = i as u16 + 1;
        // GRPICONDIRENTRY: a width or height of 0 means 256.
        let side = if n >= 256 { 0 } else { n as u8 };
        group.extend_from_slice(&[side, side, 0, 0]);
        group.extend_from_slice(&1u16.to_le_bytes());
        group.extend_from_slice(&32u16.to_le_bytes());
        group.extend_from_slice(&(image.len() as u32).to_le_bytes());
        group.extend_from_slice(&id.to_le_bytes());
        push_resource(&mut res, RT_ICON, id, 0x1010, &image);
    }
    push_resource(&mut res, RT_GROUP_ICON, ICON_GROUP_ID, 0x1030, &group);
    push_resource(&mut res, RT_VERSION, 1, 0x0030, &version_info());

    let path = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("blazeaudio.res");
    std::fs::write(&path, &res).expect("write blazeaudio.res");
    // MSVC's link.exe (and lld-link) take a compiled .res file as an ordinary input.
    println!("cargo:rustc-link-arg-bins={}", path.display());
}

/// The app icon at `n`×`n` as straight-alpha RGBA: a ring of spectrum bars around a bright
/// core, shaded from pink on the left to blue on the right. 4×4 supersampled so the small
/// sizes stay smooth; small sizes use fewer, wider bars, which would otherwise blur.
fn icon_rgba(n: u32) -> Vec<u8> {
    const SS: u32 = 4;
    const SPECTRUM: [f32; 12] = [1.0, 0.86, 0.94, 0.72, 0.84, 0.6, 0.72, 0.5, 0.62, 0.44, 0.54, 0.4];
    let bars: f32 = match n {
        0..=16 => 8.0,
        17..=32 => 12.0,
        _ => 24.0,
    };
    let half = n as f32 / 2.0;
    let mut px = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let mut acc = [0.0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let dx = (x as f32 + (sx as f32 + 0.5) / SS as f32 - half) / half;
                    let dy = (y as f32 + (sy as f32 + 0.5) / SS as f32 - half) / half;
                    let r = (dx * dx + dy * dy).sqrt();
                    let t = dy.atan2(dx) / std::f32::consts::TAU + 0.5;
                    // Bar lengths read like a spectrum: bass on the left, treble on the right,
                    // mirrored top to bottom.
                    let i = ((t * bars) as usize).min(bars as usize - 1);
                    let j = i.min(bars as usize - 1 - i) * 24 / bars as usize;
                    let h = SPECTRUM[j.min(SPECTRUM.len() - 1)];
                    let in_bar = ((t * bars).fract() - 0.5).abs() < 0.3 && r > 0.52 && r < 0.52 + 0.46 * h;
                    let core = r < 0.38;
                    let a = if in_bar || core { 1.0 } else { 0.0 };
                    let s = 0.5 + 0.5 * dx.clamp(-1.0, 1.0);
                    let mut rgb = [1.0 + (0.12 - 1.0) * s, 0.22 + (0.7 - 0.22) * s, 0.62 + (1.0 - 0.62) * s];
                    if core {
                        let w = 0.55 * (1.0 - r / 0.38);
                        rgb = rgb.map(|c| c + (1.0 - c) * w);
                    }
                    for c in 0..3 {
                        acc[c] += rgb[c] * a;
                    }
                    acc[3] += a;
                }
            }
            let a = acc[3] / (SS * SS) as f32;
            let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            if a <= 0.0 {
                px.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                px.extend_from_slice(&[to8(acc[0] / acc[3]), to8(acc[1] / acc[3]), to8(acc[2] / acc[3]), to8(a)]);
            }
        }
    }
    px
}

/// An icon image as a 32-bit DIB: BITMAPINFOHEADER (double height, for the colour and mask
/// halves), BGRA rows bottom-up, then the 1-bit AND mask, set where the pixel is transparent.
fn dib(rgba: &[u8], n: u32) -> Vec<u8> {
    let mask_row = n.div_ceil(32) * 4;
    let mut out = Vec::new();
    for v in [40u32, n, n * 2] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    for v in [0u32, n * n * 4 + mask_row * n, 0, 0, 0, 0] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for y in (0..n).rev() {
        for x in 0..n {
            let i = ((y * n + x) * 4) as usize;
            out.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    for y in (0..n).rev() {
        let mut row = vec![0u8; mask_row as usize];
        for x in 0..n {
            if rgba[((y * n + x) * 4 + 3) as usize] == 0 {
                row[(x / 8) as usize] |= 0x80 >> (x % 8);
            }
        }
        out.extend_from_slice(&row);
    }
    out
}

/// An icon image as a compressed PNG, which Windows accepts in icon resources; used for 256,
/// where a DIB would add a quarter megabyte to the exe.
fn png(rgba: &[u8], n: u32) -> Vec<u8> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::ImageEncoder;
    let mut out = Vec::new();
    PngEncoder::new_with_quality(&mut out, CompressionType::Best, FilterType::Adaptive)
        .write_image(rgba, n, n, image::ExtendedColorType::Rgba8)
        .expect("encode icon png");
    out
}

/// Appends one resource with an ordinal type and name to a .res file.
fn push_resource(res: &mut Vec<u8>, kind: u16, id: u16, flags: u16, data: &[u8]) {
    res.extend_from_slice(&(data.len() as u32).to_le_bytes());
    res.extend_from_slice(&32u32.to_le_bytes());
    for v in [0xFFFF, kind, 0xFFFF, id] {
        res.extend_from_slice(&v.to_le_bytes());
    }
    res.extend_from_slice(&0u32.to_le_bytes());
    res.extend_from_slice(&flags.to_le_bytes());
    res.extend_from_slice(&(if kind == 0 { 0 } else { LANG_EN_US }).to_le_bytes());
    res.extend_from_slice(&[0; 8]);
    res.extend_from_slice(data);
    pad4(res);
}

/// VS_VERSIONINFO for the package version: the fixed numeric block plus English strings.
fn version_info() -> Vec<u8> {
    let part = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    let (major, minor, patch) = (part("CARGO_PKG_VERSION_MAJOR"), part("CARGO_PKG_VERSION_MINOR"), part("CARGO_PKG_VERSION_PATCH"));
    let version = format!("{major}.{minor}.{patch}.0");
    let (ms, ls) = ((major << 16) | minor, patch << 16);

    let mut fixed = Vec::new();
    // Signature, struct version, file and product versions, flags mask and flags, OS
    // (VOS_NT_WINDOWS32), type (VFT_APP), subtype, date.
    for v in [0xFEEF04BD, 0x0001_0000, ms, ls, ms, ls, 0x3F, 0, 0x0004_0004, 1, 0, 0, 0] {
        fixed.extend_from_slice(&v.to_le_bytes());
    }

    let strings = [
        ("CompanyName", "zack polc"),
        ("FileDescription", "BlazeAudio"),
        ("FileVersion", version.as_str()),
        ("InternalName", "blazeaudio"),
        ("LegalCopyright", "Copyright (c) 2026 zack polc. MIT License."),
        ("OriginalFilename", "blazeaudio.exe"),
        ("ProductName", "BlazeAudio"),
        ("ProductVersion", version.as_str()),
    ];
    let table = strings.iter().map(|(k, v)| node(k, Value::Text(v), &[])).collect::<Vec<_>>();
    let string_table = node(&format!("{LANG_EN_US:04X}{CODEPAGE_UNICODE:04X}"), Value::None, &table);
    let string_info = node("StringFileInfo", Value::None, &[string_table]);
    let mut translation = Vec::new();
    translation.extend_from_slice(&LANG_EN_US.to_le_bytes());
    translation.extend_from_slice(&CODEPAGE_UNICODE.to_le_bytes());
    let var = node("Translation", Value::Binary(&translation), &[]);
    let var_info = node("VarFileInfo", Value::None, &[var]);
    node("VS_VERSION_INFO", Value::Binary(&fixed), &[string_info, var_info])
}

enum Value<'a> {
    None,
    Binary(&'a [u8]),
    Text(&'a str),
}

/// One block of a version resource: length, value length, type, key, value, children, with
/// each part aligned to 4 bytes. The length excludes the block's own trailing padding.
fn node(key: &str, value: Value, children: &[Vec<u8>]) -> Vec<u8> {
    let (bytes, value_len, kind) = match value {
        Value::None => (Vec::new(), 0u16, 1u16),
        Value::Binary(b) => (b.to_vec(), b.len() as u16, 0),
        // A text value's length counts UTF-16 units, including the terminator.
        Value::Text(s) => {
            let w = utf16z(s);
            (w.clone(), (w.len() / 2) as u16, 1)
        }
    };
    let mut out = vec![0, 0];
    out.extend_from_slice(&value_len.to_le_bytes());
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&utf16z(key));
    pad4(&mut out);
    out.extend_from_slice(&bytes);
    for c in children {
        pad4(&mut out);
        out.extend_from_slice(c);
    }
    let len = out.len() as u16;
    out[0..2].copy_from_slice(&len.to_le_bytes());
    out
}

fn utf16z(s: &str) -> Vec<u8> {
    s.encode_utf16().chain([0]).flat_map(|u| u.to_le_bytes()).collect()
}

fn pad4(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(4) {
        v.push(0);
    }
}
