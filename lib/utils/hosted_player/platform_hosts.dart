import '../../bindings/bindings.dart';
import '../apple_music_sign_in.dart';
import '../device_sign_in.dart';
import 'apple_music_host.dart';

/// Adds the hosts this platform provides, before the player bridge or a sign-in
/// looks one up.
void registerPlatformHosts() {
  AppleMusicHost.registerOnPlatform();
}

/// How the libraries that sign in through those hosts do it, by the type of
/// the kind their records say.
const platformSignInMethods = <Type, DeviceSignInMethod>{
  LibraryRecordKindSummaryAppleMusic: appleMusicSignInMethod,
};
