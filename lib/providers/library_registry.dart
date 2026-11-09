import 'dart:async';
import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:rinf/rinf.dart';

import '../bindings/bindings.dart';
import '../utils/api/request_failure.dart';
import '../utils/rune_log.dart';
import '../utils/dialogs/select_library_mode/show_select_library_mode_dialog.dart';
import '../utils/file_storage/mac_secure_manager.dart';

LibraryInitializeMode? stringToLibraryInitializeMode(String? value) {
  if (value == null) return null;
  return value == 'Redirected'
      ? LibraryInitializeMode.redirected
      : LibraryInitializeMode.portable;
}

class LibraryRegistryFailure implements Exception {
  const LibraryRegistryFailure(this.kind, this.detail);

  final Object kind;
  final String detail;
}

Future<LibraryRecordSummary?> registerAndOpenLocalFolder(
  BuildContext context,
  LibraryRegistryProvider registry,
  String path,
  String alias,
) async {
  var response = await registry.registerLocalFolder(path, alias, null);
  if (response.outcome is RegisterLibraryOutcomeNeedsInitializeMode) {
    if (!context.mounted) return null;
    final mode = Platform.isAndroid
        ? LibraryInitializeMode.redirected
        : stringToLibraryInitializeMode(
            await showSelectLibraryModeDialog(context),
          );
    if (mode == null || !context.mounted) return null;
    response = await registry.registerLocalFolder(path, alias, mode);
  }
  if (response.outcome is! RegisterLibraryOutcomeRegistered ||
      response.record == null) {
    return null;
  }
  // A sandboxed macOS app can reopen a user-picked folder after a restart only
  // through a security-scoped bookmark taken now; other platforms skip this.
  await MacSecureManager().saveBookmark(path);
  await registry.open(response.record!.recordId);
  return response.record;
}

class LibraryRegistryProvider extends ChangeNotifier {
  LibraryRegistryProvider({
    Future<ListLibraryRecordsResponse> Function()? loadRecords,
    Future<OpenLibraryResponse> Function(String recordId, int callId)?
    openRecord,
    Future<RegisterLibraryResponse> Function(
      String path,
      String alias,
      LibraryInitializeMode? mode,
    )?
    registerLocal,
    Future<RegisterLibraryResponse> Function(String fingerprint, String alias)?
    registerRemote,
    Future<void> Function()? closeRecord,
    Future<RemoveLibraryRecordResponse> Function(String recordId)? removeRecord,
    Stream<RustSignalPack<LibraryRecordsChanged>>? recordsChanged,
  }) : _loadRecords = loadRecords,
       _openRecord = openRecord,
       _registerLocal = registerLocal,
       _registerRemote = registerRemote,
       _closeRecord = closeRecord,
       _removeRecord = removeRecord {
    _recordsSubscription =
        (recordsChanged ?? LibraryRecordsChanged.rustSignalStream).listen(
          (signal) => _setRecords(
            signal.message.records,
            signal.message.invalidRecords,
            signal.message.open,
          ),
        );
    ready = refresh();
  }

  List<LibraryRecordSummary> _records = const [];
  List<InvalidLibraryRecordSummary> _invalidRecords = const [];
  String? _openRecordId;
  int? _openGeneration;
  String? _lastOpenRecordId;
  int _nextCallId = DateTime.now().microsecondsSinceEpoch;
  bool _disposed = false;
  StreamSubscription<dynamic>? _recordsSubscription;
  late final Future<void> ready;
  final Future<ListLibraryRecordsResponse> Function()? _loadRecords;
  final Future<OpenLibraryResponse> Function(String, int)? _openRecord;
  final Future<RegisterLibraryResponse> Function(
    String,
    String,
    LibraryInitializeMode?,
  )?
  _registerLocal;
  final Future<RegisterLibraryResponse> Function(String, String)?
  _registerRemote;
  final Future<void> Function()? _closeRecord;
  final Future<RemoveLibraryRecordResponse> Function(String)? _removeRecord;

  List<LibraryRecordSummary> get records => List.unmodifiable(_records);
  List<InvalidLibraryRecordSummary> get invalidRecords =>
      List.unmodifiable(_invalidRecords);
  String? get openRecordId => _openRecordId;
  int? get openGeneration => _openGeneration;
  String? get lastOpenRecordId => _lastOpenRecordId;
  LibraryRecordSummary? get openRecord {
    final id = _openRecordId;
    if (id == null) return null;
    for (final record in _records) {
      if (record.recordId == id) return record;
    }
    return null;
  }

  void onClosed() {
    _openRecordId = null;
    _openGeneration = null;
    if (!_disposed) notifyListeners();
  }

  void _setRecords(
    List<LibraryRecordSummary> records,
    List<InvalidLibraryRecordSummary> invalidRecords,
    OpenRecord? open,
  ) {
    _records = List.unmodifiable(records);
    _invalidRecords = List.unmodifiable(invalidRecords);
    _openRecordId = open?.recordId;
    _openGeneration = open?.generation.toInt();
    if (!_disposed) notifyListeners();
  }

  Future<void> refresh() async {
    final response = _loadRecords == null
        ? (await awaitResponse(
            'ListLibraryRecordsRequest',
            ListLibraryRecordsResponse.rustSignalStream,
            send: () => const ListLibraryRecordsRequest().sendSignalToRust(),
          )).message
        : await _loadRecords();
    _setRecords(response.records, response.invalidRecords, response.open);
    _lastOpenRecordId = response.lastOpenRecord;
    if (!_disposed) notifyListeners();
  }

  Future<OpenLibraryResponse> open(String recordId) async {
    final callId = Uint64.fromBigInt(BigInt.from(_nextCallId++));
    final response = _openRecord == null
        ? (await awaitResponse(
            'OpenLibraryRequest',
            OpenLibraryResponse.rustSignalStream,
            callId: callId.toInt(),
            responseCallId: (response) => response.message.callId.toInt(),
            send: () => OpenLibraryRequest(
              callId: callId,
              recordId: recordId,
            ).sendSignalToRust(),
          )).message
        : await _openRecord(recordId, callId.toInt());
    if (response.result case OpenLibraryResultFailed(:final value)) {
      final failure = value;
      error$('Could not open library record: ${failure.detail}');
      throw LibraryRegistryFailure(failure.kind, failure.detail);
    }
    _openRecordId = response.recordId;
    _openGeneration = response.generation.toInt();
    _lastOpenRecordId = response.recordId;
    if (!_disposed) notifyListeners();
    return response;
  }

  Future<RegisterLibraryResponse> registerLocalFolder(
    String path,
    String alias,
    LibraryInitializeMode? mode,
  ) async {
    final response = _registerLocal == null
        ? await _register(
            'RegisterLocalLibraryRequest',
            RegisterLibraryResponse.rustSignalStream,
            () => RegisterLocalLibraryRequest(
              path: path,
              alias: alias,
              mode: mode,
            ).sendSignalToRust(),
          )
        : await _registerLocal(path, alias, mode);
    return _acceptRegisterResponse(response);
  }

  Future<RegisterLibraryResponse> registerRemoteControl(
    String fingerprint,
    String alias,
  ) async {
    final response = _registerRemote == null
        ? await _register(
            'RegisterRemoteControlRequest',
            RegisterLibraryResponse.rustSignalStream,
            () => RegisterRemoteControlRequest(
              fingerprint: fingerprint,
              alias: alias,
            ).sendSignalToRust(),
          )
        : await _registerRemote(fingerprint, alias);
    return _acceptRegisterResponse(response);
  }

  Future<RegisterLibraryResponse> _register(
    String request,
    Stream<dynamic> responses,
    void Function() send,
  ) async {
    final signal = await awaitResponse(request, responses, send: send);
    return signal.message as RegisterLibraryResponse;
  }

  RegisterLibraryResponse _acceptRegisterResponse(
    RegisterLibraryResponse response,
  ) {
    if (response.outcome case RegisterLibraryOutcomeFailed(:final value)) {
      final detail = response.detail ?? value.toString();
      error$('Could not register library: $detail');
      throw LibraryRegistryFailure(value, detail);
    }
    if (response.record != null) {
      _records = [
        ..._records.where((r) => r.recordId != response.record!.recordId),
        response.record!,
      ];
      if (!_disposed) notifyListeners();
    }
    return response;
  }

  Future<void> remove(String recordId) async {
    if (_openRecordId == recordId) {
      if (_closeRecord != null) {
        await _closeRecord();
      } else {
        await awaitResponse(
          'CloseLibraryRequest',
          CloseLibraryResponse.rustSignalStream,
          send: () => CloseLibraryRequest().sendSignalToRust(),
        );
      }
      _openRecordId = null;
      _openGeneration = null;
    }
    final response = _removeRecord == null
        ? (await awaitResponse(
            'RemoveLibraryRecordRequest',
            RemoveLibraryRecordResponse.rustSignalStream,
            send: () => RemoveLibraryRecordRequest(
              recordId: recordId,
            ).sendSignalToRust(),
          )).message
        : await _removeRecord(recordId);
    if (response.outcome case RemoveLibraryRecordOutcomeFailed(:final value)) {
      error$('Could not remove library record: $value');
      throw LibraryRegistryFailure(value, value.toString());
    }
    _records = _records.where((record) => record.recordId != recordId).toList();
    _invalidRecords = _invalidRecords
        .where((record) => record.recordId != recordId)
        .toList();
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _recordsSubscription?.cancel();
    super.dispose();
  }
}
