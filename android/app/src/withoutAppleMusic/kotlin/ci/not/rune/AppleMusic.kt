package ci.not.rune

import io.flutter.embedding.engine.plugins.FlutterPlugin

/**
 * This build has no Apple Music: it was not given MusicKit for Android (see settings.gradle), so
 * nothing answers on `ci.not.rune/apple_music`, and Dart offers no Apple Music.
 */
internal fun appleMusicPlugin(): FlutterPlugin? = null
