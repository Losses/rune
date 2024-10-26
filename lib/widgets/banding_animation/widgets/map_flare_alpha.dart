double mapFlareAlpha(double x, double a) {
  if (x <= a) {
    return x / a;
  } else if (x <= 1 - a) {
    return 1.0;
  } else {
    return (1 - x) / a;
  }
}
