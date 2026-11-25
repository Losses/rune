import 'apple_music_host.dart';

/// Adds the hosts this platform provides, before the player bridge or a sign-in looks one up.
void registerPlatformHosts() {
  AppleMusicHost.registerOnPlatform();
}
