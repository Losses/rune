/// Narrowest strip at the left edge that starts the swipe back, in logical
/// pixels.
const edgeSwipeMinWidth = 20.0;

/// A rightward drag longer than this, or than a quarter of the width if that
/// is shorter, goes back when the finger lifts.
const edgeSwipeDistance = 60.0;

/// A rightward fling faster than this goes back whatever its length.
const edgeSwipeVelocity = 300.0;
