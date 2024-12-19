import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:rinf/rinf.dart';
import 'package:uuid/uuid.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:pointycastle/export.dart' as pc;

import '../utils/settings_manager.dart';
import '../messages/all.dart';

extension LoginRequestItemJson on LoginRequestItem {
  Map<String, dynamic> toMap() {
    return {
      'serviceId': serviceId,
      'username': username,
      'password': password,
      if (apiKey != "") 'api_key': apiKey,
      if (apiSecret != "") 'api_secret': apiSecret,
    };
  }
}

LoginRequestItem itemFromMap(Map<String, dynamic> json) {
  return LoginRequestItem(
    serviceId: json['serviceId'] as String,
    username: json['username'] as String,
    password: json['password'] as String,
    apiKey: json['api_key'] as String?,
    apiSecret: json['api_secret'] as String?,
  );
}

class ScrobbleProvider with ChangeNotifier {
  static const String _credentialsKey = 'login_credentials';
  static const String _encryptionKey = 'encryption_key';
  final SettingsManager _settingsManager = SettingsManager();
  late StreamSubscription<RustSignal<ScrobbleServiceStatusUpdated>>
      _statusSubscription;
  List<ScrobbleServiceStatus> _serviceStatuses = [];
  List<ScrobbleServiceStatus> get serviceStatuses => _serviceStatuses;
  String _encryptionKeyValue = '';

  ScrobbleProvider() {
    _init();
    _statusSubscription = ScrobbleServiceStatusUpdated.rustSignalStream
        .listen(_handleStatusUpdate);
  }

  Future<void> _init() async {
    _encryptionKeyValue = await _getOrGenerateEncryptionKey();
    List<LoginRequestItem> storedCredentials = await _getStoredCredentials();
    if (storedCredentials.isNotEmpty) {
      AuthenticateMultipleServiceRequest(requests: storedCredentials)
          .sendSignalToRust();
    }
  }

  Future<String> _getOrGenerateEncryptionKey() async {
    String? key = await _settingsManager.getValue<String>(_encryptionKey);
    if (key == null) {
      key = _generateRandomKey();
      await _settingsManager.setValue(_encryptionKey, key);
    }
    return key;
  }

  String _generateRandomKey() {
    return Uuid().v4();
  }

  Future<List<LoginRequestItem>> _getStoredCredentials() async {
    String? encryptedData =
        await _settingsManager.getValue<String>(_credentialsKey);
    if (encryptedData == null) return [];

    String decryptedData = _decrypt(encryptedData);
    List<dynamic> decodedList = jsonDecode(decryptedData);
    return decodedList.map((item) => itemFromMap(item)).toList();
  }

  Future<void> login(LoginRequestItem credentials) async {
    String encryptedData = _encrypt(jsonEncode(credentials.toMap()));
    await _settingsManager.setValue(_credentialsKey, encryptedData);
    AuthenticateSingleServiceRequest(request: credentials).sendSignalToRust();

    final rustSignal =
        await AuthenticateSingleServiceResponse.rustSignalStream.first;
    final response = rustSignal.message;

    if (!response.success) {
      throw response.error;
    }
  }

  void _handleStatusUpdate(RustSignal<ScrobbleServiceStatusUpdated> signal) {
    _serviceStatuses = signal.message.services;
    notifyListeners();
  }

  String _encrypt(String data) {
    final key = utf8.encode(_encryptionKeyValue.substring(0, 16));
    final iv = Uint8List(16);

    final pc.KeyParameter keyParam = pc.KeyParameter(Uint8List.fromList(key));
    final pc.ParametersWithIV<pc.KeyParameter> params =
        pc.ParametersWithIV(keyParam, iv);

    final pc.BlockCipher cipher = pc.PaddedBlockCipher('AES/CBC/PKCS7');
    cipher.init(true, params);

    final Uint8List inputData = Uint8List.fromList(utf8.encode(data));
    final Uint8List encryptedData = cipher.process(inputData);

    return base64UrlEncode(encryptedData);
  }

  String _decrypt(String encryptedData) {
    final key = utf8.encode(_encryptionKeyValue.substring(0, 16));
    final iv = Uint8List(16);

    final pc.KeyParameter keyParam = pc.KeyParameter(Uint8List.fromList(key));
    final pc.ParametersWithIV<pc.KeyParameter> params =
        pc.ParametersWithIV(keyParam, iv);

    final pc.BlockCipher cipher = pc.PaddedBlockCipher('AES/CBC/PKCS7');
    cipher.init(false, params);

    final Uint8List encryptedBytes = base64Url.decode(encryptedData);
    final Uint8List decryptedData = cipher.process(encryptedBytes);

    return utf8.decode(decryptedData);
  }

  @override
  void dispose() {
    _statusSubscription.cancel();
    super.dispose();
  }
}
