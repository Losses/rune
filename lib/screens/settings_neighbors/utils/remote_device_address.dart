import '../../../utils/discovery_url.dart';

/// The library path under which a remote device is stored and opened.
String remoteLibraryPath(List<String> hosts) =>
    '@RR|${encodeRnSrvUrl(hosts)}';
