// Scene: "Light Tunnel" — a swirling fibre-optic tunnel. Thin strands spiral into the dark
// and comets of light race down them into depth. Each strand belongs to a frequency band,
// which sets how often its comets fire; mids twist the tunnel, kicks flash the heads.

fn strand_layer(a: f32, r: f32, z: f32, lanes: f32, layer: f32, twist: f32, fog: f32) -> vec3f {
  let u = (a + twist) / TAU * lanes + layer * 0.37;
  let li = floor(u);
  let lf = fract(u) - 0.5;
  // Distance to the strand centerline in screen pixels keeps strands crisp at any depth.
  let dpx = abs(lf) * TAU / lanes * r * G.res.y * 0.5;
  let core = exp(-dpx * dpx * 0.35);
  let halo = exp(-dpx * 0.12) * 0.12;

  let h = hash11(li * 13.17 + layer * 71.3);
  let bx = abs(fract(li / lanes) * 2.0 - 1.0);
  let bv = band(bx);
  let tint = pal_grad(bx * 0.65 + layer * 0.2 + G.time * 0.01);
  var col = tint * (core * 0.22 + halo) * (0.18 + bv * 0.9) * fog;

  // One comet per strand per cycle; louder bands fire more often.
  let speed = 0.55 + h * 0.9;
  let cyc = G.flow * 0.11 * speed + h * 7.3;
  let ph = fract(cyc);
  let alive = step(1.0 - (0.2 + bv * 0.85 + G.hat * 0.25), hash11(li * 3.71 + floor(cyc) * 1.37 + layer));
  let zc = mix(0.35, 16.0, ph * ph);
  let dz = z - zc;
  let len = 0.6 + zc * 0.35;
  let head = exp(-dz * dz * 40.0 / (zc * 0.3 + 0.1));
  let tail = select(0.0, exp(dz / len * 2.2), dz < 0.0);
  let comet = (head * 2.6 + tail * 0.7) * (core + halo * 2.0) * alive;
  col += mix(tint, vec3f(1.0), 0.45) * comet * sqrt(fog) * (0.9 + G.kick * 1.6 * G.intensity);
  return col;
}

@fragment
fn fs_tunnel(in: VsOut) -> @location(0) vec4f {
  var p = aspect_uv(in.uv);
  p.y = -p.y;
  p -= vec2f(sin(G.time * 0.21), cos(G.time * 0.17)) * 0.05;
  let r = max(length(p), 1e-4);
  let a = atan2(p.y, p.x);
  let z = 0.35 / r;
  let fog = exp(-z * 0.16);
  let twist = G.flow * 0.22 + z * (0.07 + 0.08 * G.mid);

  var col = vec3f(0.0);
  col += strand_layer(a, r, z, 54.0, 0.0, twist, fog);
  col += strand_layer(a, r, z, 89.0, 1.0, twist * 1.25, fog);
  col += strand_layer(a, r, z, 144.0, 2.0, twist * 1.55, fog) * 0.8;

  col += mix(pal(0), vec3f(1.0), 0.3) * (0.006 / (r * r + 0.004)) * (0.25 + G.bass * 0.9 + G.kick * 0.7);
  col += pal(3) * 0.08 * (1.0 - fog);
  return vec4f(col, 1.0);
}
