const double hashScale = 0.1031;

double frac(double value) {
  return value - value.floorToDouble();
}

double hash13(double x, double y, double z) {
  x = frac(x * hashScale);
  y = frac(y * hashScale);
  z = frac(z * hashScale);

  double dotProduct = x * (y + 19.19) + y * (z + 19.19) + z * (x + 19.19);

  x += dotProduct;
  y += dotProduct;
  z += dotProduct;

  return frac((x + y) * z);
}
