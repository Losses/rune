import 'dart:typed_data';

/// The corner gradients of the Perlin noise in lib/shaders/gradient.frag.
///
/// The shader reads the noise at (x, y, t): x and y within (-1, 1) and t the
/// same for every pixel. Those points lie in the lattice cells between -1 and 1
/// on x and y, between floor(t) and the next integer on t, so 3 x 3 x 2 corner
/// gradients serve the whole window. They used to be hashed again for every
/// pixel; they are hashed here, once for each integer of t, with the same
/// hash, and given to the shader as uniforms.
class NoiseLattice {
  int? _floor;
  final Float64List _gradients = Float64List(54);

  /// The gradients of the corners around t, in the order of the shader's
  /// uniforms: u_lattice0_mm to u_lattice0_pp, then u_lattice1_mm to
  /// u_lattice1_pp, three numbers each. Within a layer y goes from -1 to 1,
  /// and x from -1 to 1 within each y.
  ///
  /// [t] must be the value the shader receives, a 32-bit float, so that both
  /// take the same integer below it.
  Float64List at(double t) {
    final floor = t.floor();
    if (_floor != floor) {
      _floor = floor;
      var i = 0;
      for (final z in [floor, floor + 1]) {
        for (var y = -1; y <= 1; y++) {
          for (var x = -1; x <= 1; x++) {
            final g = gradientAt(x, y, z);
            _gradients[i++] = g.$1;
            _gradients[i++] = g.$2;
            _gradients[i++] = g.$3;
          }
        }
      }
    }
    return _gradients;
  }
}

final _float32 = Float32List(1);

/// [value] as the shader receives and computes it, a 32-bit float.
double asFloat32(double value) {
  _float32[0] = value;
  return _float32[0];
}

double _mod289(double x) => x - (x / 289.0).floorToDouble() * 289.0;

double _permute(double x) => _mod289(((x * 34.0) + 1.0) * x);

// The shader's compiler folds its divisions by 7 into products with 1 / 7
// rounded to a 32-bit float (see the compiled SkSL), on every device.
final double _seventh = asFloat32(1.0 / 7.0);

double _fract32(double x) => asFloat32(x - x.floorToDouble());

/// The normalized gradient of the noise at the lattice point (x, y, z), as
/// the shader's classic Perlin noise hashed it.
(double, double, double) gradientAt(int x, int y, int z) {
  // Whole numbers below 2^24 all the way, so exact in a 32-bit float too, and
  // the same whichever way a GPU computes mod.
  final hash = _permute(
    _permute(_permute(_mod289(x.toDouble())) + _mod289(y.toDouble())) +
        _mod289(z.toDouble()),
  );
  // The rest in 32-bit floats, as the shader does it: for some hashes gz is
  // zero but for the rounding, and the sign the rounding leaves decides the
  // gradient.
  final q = asFloat32(hash * _seventh);
  var gx = _fract32(q);
  var gy = asFloat32(_fract32(asFloat32(q.floorToDouble() * _seventh)) - 0.5);
  final gz = asFloat32(asFloat32(0.5 - gx.abs()) - gy.abs());
  // step(gz, 0.0) in the shader.
  if (!(0.0 < gz)) {
    gx = asFloat32(gx - ((gx >= 0.0 ? 1.0 : 0.0) - 0.5));
    gy = asFloat32(gy - ((gy >= 0.0 ? 1.0 : 0.0) - 0.5));
  }
  final norm = asFloat32(
    1.79284291400159 - 0.85373472095314 * (gx * gx + gy * gy + gz * gz),
  );
  return (asFloat32(gx * norm), asFloat32(gy * norm), asFloat32(gz * norm));
}
