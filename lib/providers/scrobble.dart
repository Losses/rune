import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:rinf/rinf.dart';
import 'package:uuid/uuid.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:pointycastle/export.dart' as pc;

import '../utils/settings_manager.dart';
import '../bindings/bindings.dart';

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

class ServiceStatus {
  final String serviceId;
  final bool isAvailable;
  final bool hasCredentials;
  final String error;

  ServiceStatus({
    required this.serviceId,
    required this.isAvailable,
    required this.hasCredentials,
    required this.error,
  });

  @override
  bool operator ==(Object other) {
    if (identical(this, other)) return true;

    return other is ServiceStatus &&
        other.serviceId == serviceId &&
        other.isAvailable == isAvailable &&
        other.hasCredentials == hasCredentials &&
        other.error == error;
  }

  @override
  int get hashCode {
    return serviceId.hashCode ^
        isAvailable.hashCode ^
        hasCredentials.hashCode ^
        error.hashCode;
  }
}

class ScrobbleProvider with ChangeNotifier {
  static const String encryptionKey = 'encryption_key';
  static const String credentialsKey = 'login_credentials';

  final SettingsManager _settingsManager = SettingsManager();
  late StreamSubscription<RustSignalPack<ScrobbleServiceStatusUpdated>>
      _statusSubscription;
  List<ServiceStatus> _serviceStatuses = [];
  List<ServiceStatus> get serviceStatuses => _serviceStatuses;
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
    String? key = await _settingsManager.getValue<String>(encryptionKey);
    if (key == null) {
      key = _generateRandomKey();
      await _settingsManager.setValue(encryptionKey, key);
    }
    return key;
  }

  String _generateRandomKey() {
    return Uuid().v4();
  }

  Future<List<LoginRequestItem>> _getStoredCredentials() async {
    String? encryptedData =
        await _settingsManager.getValue<String>(credentialsKey);
    if (encryptedData == null) return [];

    String decryptedData = _decrypt(encryptedData);
    List<dynamic> decodedList = jsonDecode(decryptedData);
    return decodedList.map((item) => itemFromMap(item)).toList();
  }

  Future<void> retryLogin(String serviceId) async {
    // Retrieve stored credentials
    List<LoginRequestItem> storedCredentials = await _getStoredCredentials();

    // Find the credentials for the given serviceId
    LoginRequestItem? credentials = storedCredentials.firstWhere(
      (item) => item.serviceId == serviceId,
      orElse: () =>
          throw Exception('No credentials found for serviceId: $serviceId'),
    );

    // Send login request using the found credentials
    AuthenticateSingleServiceRequest(request: credentials).sendSignalToRust();

    // Wait for the response
    final rustSignal =
        await AuthenticateSingleServiceResponse.rustSignalStream.first;
    final response = rustSignal.message;

    // Handle the response
    if (!response.success) {
      throw response.error ?? "Unknown error";
    }
  }

  Future<void> login(LoginRequestItem credentials) async {
    AuthenticateSingleServiceRequest(request: credentials).sendSignalToRust();

    final rustSignal =
        await AuthenticateSingleServiceResponse.rustSignalStream.first;
    final response = rustSignal.message;

    if (!response.success) {
      throw response.error ?? "Unknown error";
    }

    // Read existing credentials
    List<LoginRequestItem> storedCredentials = await _getStoredCredentials();

    // Check if the same serviceId already exists, update if it does, otherwise add
    int existingIndex = storedCredentials
        .indexWhere((item) => item.serviceId == credentials.serviceId);
    if (existingIndex != -1) {
      storedCredentials[existingIndex] = credentials;
    } else {
      storedCredentials.add(credentials);
    }

    // Save the updated list of credentials back
    String encryptedData = _encrypt(
        jsonEncode(storedCredentials.map((item) => item.toMap()).toList()));
    await _settingsManager.setValue(credentialsKey, encryptedData);
  }

  Future<void> logout(String serviceId) async {
    // Send logout request to the backend
    LogoutSingleServiceRequest(serviceId: serviceId).sendSignalToRust();

    // Remove the service from stored credentials
    List<LoginRequestItem> storedCredentials = await _getStoredCredentials();
    storedCredentials.removeWhere((item) => item.serviceId == serviceId);

    // Save the updated list of credentials
    String encryptedData = _encrypt(
        jsonEncode(storedCredentials.map((item) => item.toMap()).toList()));
    await _settingsManager.setValue(credentialsKey, encryptedData);
  }

  Future<void> _handleStatusUpdate(
      RustSignalPack<ScrobbleServiceStatusUpdated> signal) async {
    List<LoginRequestItem> storedCredentials = await _getStoredCredentials();
    Set<String> credentialServiceIds =
        storedCredentials.map((c) => c.serviceId).toSet();

    _serviceStatuses = signal.message.services
        .map((status) => ServiceStatus(
              serviceId: status.serviceId,
              isAvailable: status.isAvailable,
              hasCredentials: credentialServiceIds.contains(status.serviceId),
              error: status.error ?? "",
            ))
        .toList();

    notifyListeners();
  }

  String _encrypt(String data) {
    final key = utf8.encode(_encryptionKeyValue.substring(0, 16));
    final iv = Uint8List(16);

    final pc.KeyParameter keyParam = pc.KeyParameter(Uint8List.fromList(key));
    final pc.ParametersWithIV<pc.KeyParameter> params =
        pc.ParametersWithIV(keyParam, iv);

    final pc
        .PaddedBlockCipherParameters<pc.CipherParameters, pc.CipherParameters>
        paddedParams = pc.PaddedBlockCipherParameters(params, null);

    final pc.BlockCipher cipher = pc.PaddedBlockCipher('AES/CBC/PKCS7');
    cipher.init(true, paddedParams);

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

    final pc
        .PaddedBlockCipherParameters<pc.CipherParameters, pc.CipherParameters>
        paddedParams = pc.PaddedBlockCipherParameters(params, null);

    final pc.BlockCipher cipher = pc.PaddedBlockCipher('AES/CBC/PKCS7');
    cipher.init(false, paddedParams);

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
