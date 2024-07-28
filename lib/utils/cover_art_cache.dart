import 'dart:io';
import 'dart:typed_data';
import 'package:path_provider/path_provider.dart';
import 'package:path/path.dart' as path;
import 'package:crclib/catalog.dart';

class CoverArtCache {
  static final CoverArtCache _instance = CoverArtCache._internal();
  final Map<int, String> _idToHashMap = {};

  factory CoverArtCache() {
    return _instance;
  }

  CoverArtCache._internal();

  Future<String> _getCacheDir() async {
    final directory = await getTemporaryDirectory();
    return path.join(directory.path, 'cover_art_cache');
  }

  Future<void> _ensureCacheDirExists() async {
    final cacheDir = await _getCacheDir();
    final dir = Directory(cacheDir);
    if (!await dir.exists()) {
      await dir.create(recursive: true);
    }
  }

  String _generateHash(Uint8List data) {
    final crc32 = Crc32Xz();
    final hash = crc32.convert(data);
    return hash.toRadixString(16); // Convert to hexadecimal string
  }

  Future<void> saveCoverArt(int fileId, Uint8List data) async {
    await _ensureCacheDirExists();
    final hash = _generateHash(data);
    final cacheDir = await _getCacheDir();
    final filePath = path.join(cacheDir, hash);
    final file = File(filePath);
    if (!await file.exists()) {
      await file.writeAsBytes(data);
    }
    _idToHashMap[fileId] = hash;
  }

  Future<Uint8List?> getCoverArt(int fileId) async {
    if (_idToHashMap.containsKey(fileId)) {
      final hash = _idToHashMap[fileId]!;
      final cacheDir = await _getCacheDir();
      final filePath = path.join(cacheDir, hash);
      final file = File(filePath);
      if (await file.exists()) {
        return await file.readAsBytes();
      }
    }
    return null;
  }
}

