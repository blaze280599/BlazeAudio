// Scene: "Plasma" — volumetric folds of light: a turbulent shell raymarched through a cosine
// turbulence field. Energy speeds the turbulence, bass swells the shell, kicks flare it.

@fragment
fn fs_plasma(in: VsOut) -> @location(0) vec4f {
  var uv = aspect_uv(in.uv);
  uv.y = -uv.y;
  let punch = G.kick * G.intensity;
  let t = G.flow * 0.45;
  let rd = normalize(vec3f(uv * 0.85, 1.0));
  let radius = 1.0 + 0.28 * G.bass_slow * G.intensity + 0.12 * punch;
  let turb = 0.55 + 0.55 * G.mid * G.intensity + 0.25 * G.hat;
  let ra = rot2(t * 0.21);
  let rb = rot2(t * 0.13 + 0.7);

  var z = 0.0;
  var acc = vec3f(0.0);
  for (var i = 0; i < 46; i++) {
    var p = rd * z - vec3f(0.0, 0.0, 3.4);
    let pxz = ra * p.xz;
    p = vec3f(pxz.x, p.y, pxz.y);
    let pxy = rb * p.xy;
    p = vec3f(pxy.x, pxy.y, p.z);
    var d = 1.0;
    for (var k = 0; k < 6; k++) {
      p += cos(p.yzx * d + vec3f(t, t * 0.7, -t * 0.5)) / d * turb;
      d *= 1.65;
    }
    let dist = 0.005 + abs(length(p) - radius) * 0.11;
    z += dist;
    let c = pal_grad(length(p) * 0.22 + z * 0.06 + G.time * 0.015);
    acc += c / dist;
    if (z > 7.5) { break; }
  }
  var col = acc * 0.00042 * (0.55 + G.energy * 1.1 + punch * 0.9);
  col += pal(3) * 0.06;
  return vec4f(col, 1.0);
}
