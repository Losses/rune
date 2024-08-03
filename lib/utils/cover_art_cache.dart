import 'dart:io';
import 'dart:typed_data';
import 'package:path_provider/path_provider.dart';
import 'package:path/path.dart' as path;
import 'package:crclib/catalog.dart';
import 'package:rxdart/rxdart.dart';

import '../messages/cover_art.pb.dart';

class CoverArtCache {
  static final CoverArtCache _instance = CoverArtCache._internal();
  final Map<int, String?> _idToFilePathMap = {};
  final Map<int, int> _fileIdToCoverArtIdMap = {};
  final Map<String, BehaviorSubject<File?>> _requestSubjects = {};

  factory CoverArtCache() {
    return _instance;
  }

  CoverArtCache._internal() {
    _initializeListeners();
  }

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

  Future<void> saveCoverArt(int? fileId, Uint8List data) async {
    if (fileId == null) {
      throw 'Cover art id not defined';
    }

    if (data.isEmpty) {
      _idToFilePathMap[fileId] = null;
    } else {
      await _ensureCacheDirExists();
      final hash = _generateHash(data);
      final cacheDir = await _getCacheDir();
      final filePath = path.join(cacheDir, hash);
      final file = File(filePath);
      if (!await file.exists()) {
        await file.writeAsBytes(data);
      }
      _idToFilePathMap[fileId] = filePath;
    }
  }

  Future<bool?> isMagicCoverArt(int fileId) async {
    if (_idToFilePathMap.containsKey(fileId)) {
      final filePath = _idToFilePathMap[fileId];
      return filePath == null;
    }

    return null;
  }

  Future<File?> getCoverArt(int fileId) async {
    if (_idToFilePathMap.containsKey(fileId)) {
      final filePath = _idToFilePathMap[fileId];

      if (filePath != null) {
        final file = File(filePath);

        if (await file.exists()) {
          return file;
        }
      } else {
        return null;
      }
    }
    return null;
  }

  void _initializeListeners() {
    CoverArtByFileIdResponse.rustSignalStream.listen((event) async {
      final response = event.message;
      final coverArtData = Uint8List.fromList(response.coverArt);
      await saveCoverArt(response.coverArtId, coverArtData);
      _fileIdToCoverArtIdMap[response.fileId] = response.coverArtId;
      final coverArtFile = await getCoverArt(response.coverArtId);

      final subjectKey = 'fileId:${response.fileId}';
      if (_requestSubjects.containsKey(subjectKey)) {
        final subject = _requestSubjects[subjectKey]!;
        subject.add(coverArtFile);
        subject.close();
        _requestSubjects.remove(subjectKey);
      }
    });

    CoverArtByCoverArtIdResponse.rustSignalStream.listen((event) async {
      final response = event.message;
      final coverArtData = Uint8List.fromList(response.coverArt);
      await saveCoverArt(response.coverArtId, coverArtData);
      final coverArtFile = await getCoverArt(response.coverArtId);

      final subjectKey = 'coverArtId:${response.coverArtId}';
      if (_requestSubjects.containsKey(subjectKey)) {
        final subject = _requestSubjects[subjectKey]!;
        subject.add(coverArtFile);
        subject.close();
        _requestSubjects.remove(subjectKey);
      }
    });
  }

  Future<File?> requestCoverArt({int? fileId, int? coverArtId}) async {
    if ((fileId ?? coverArtId) == null) {
      throw 'Either fileId or coverArtId must be provided';
    }

    if (fileId != null && coverArtId != null) {
      throw 'Only one of fileId or coverArtId should be provided';
    }

    final subjectKey =
        fileId != null ? 'fileId:$fileId' : 'coverArtId:$coverArtId';

    if (_requestSubjects.containsKey(subjectKey)) {
      return _requestSubjects[subjectKey]!.first;
    }

    final subject = BehaviorSubject<File?>();
    _requestSubjects[subjectKey] = subject;

    File? cachedCoverArt;

    if (fileId != null) {
      // Check if coverArtId is already cached for this fileId
      if (_fileIdToCoverArtIdMap.containsKey(fileId)) {
        final cachedCoverArtId = _fileIdToCoverArtIdMap[fileId];
        cachedCoverArt = await getCoverArt(cachedCoverArtId!);
      }
    } else if (coverArtId != null) {
      final isMagic = await isMagicCoverArt(coverArtId);

      if (isMagic != null && isMagic) {
        return null;
      }

      cachedCoverArt = await getCoverArt(coverArtId);
    }

    if (cachedCoverArt != null) {
      subject.add(cachedCoverArt);
      subject.close();
      _requestSubjects.remove(subjectKey);
      return cachedCoverArt;
    }

    if (fileId != null) {
      GetCoverArtByFileIdRequest(fileId: fileId).sendSignalToRust();
    } else if (coverArtId != null) {
      GetCoverArtByCoverArtIdRequest(coverArtId: coverArtId).sendSignalToRust();
    }

    return subject.first;
  }
}
