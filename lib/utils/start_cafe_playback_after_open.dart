Future<void> startCafePlaybackAfterOpen({
  required Future<bool> libraryOpened,
  required Future<void> Function() play,
}) async {
  if (await libraryOpened) await play();
}
