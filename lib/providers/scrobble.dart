import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:rinf/rinf.dart';
import 'package:uuid/uuid.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:pointycastle/export.dart' as pc;

import '../l10n/app_localizations.dart';
import '../utils/settings_manager.dart';
import '../bindings/bindings.dart';

extension LoginRequestItemJson on LoginRequestItem {
  String? get credentialRef => secretInput is ScrobbleSecretInputSaved
      ? (secretInput as ScrobbleSecretInputSaved).credentialRef
      : null;

  bool get hasCredentials =>
      secretInput is ScrobbleSecretInputSaved ||
      secretInput is ScrobbleSecretInputPassword;

  Map<String, dynamic> toMap() {
    return {
      'serviceId': serviceId,
      'username': username,
      if (credentialRef != null) 'credential_ref': credentialRef,
    };
  }
}

LoginRequestItem itemFromMap(Map<String, dynamic> json) {
  return LoginRequestItem(
    serviceId: json['serviceId'] as String,
    username: json['username'] as String,
    secretInput: ScrobbleSecretInputSaved(
      credentialRef: json['credential_ref'] as String,
    ),
  );
}

String scrobbleAuthenticationFailureMessage(
  AuthenticateSingleServiceResultFailure result,
  AppLocalizations strings,
) {
  final failure = result.failure;
  return switch (failure) {
    ScrobbleAuthenticationFailureCredentialsRejected() =>
      strings.scrobbleCredentialsRejected,
    ScrobbleAuthenticationFailureServiceUnreachable() =>
      strings.scrobbleServiceUnreachable,
    ScrobbleAuthenticationFailureLastFmApiCredentialsMissing(:final field) =>
      strings.scrobbleLastFmApiCredentialsMissing(
        field == LastFmApiCredentialField.apiKey
            ? strings.apiKey
            : strings.apiSecret,
      ),
    ScrobbleAuthenticationFailureLastFmApiCredentialsInvalid() =>
      strings.scrobbleLastFmApiCredentialsInvalid,
    ScrobbleAuthenticationFailureOther(:final detail) =>
      '${strings.loginFailedSubtitle}: $detail',
    _ => strings.loginFailedSubtitle,
  };
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
    _statusSubscription = ScrobbleServiceStatusUpdated.rustSignalStream.listen(
      _handleStatusUpdate,
    );
  }

  Future<void> _init() async {
    _encryptionKeyValue = await _getOrGenerateEncryptionKey();
    late final List<LoginRequestItem> storedCredentials;
    try {
      storedCredentials = await _getStoredCredentials();
    } catch (_) {
      await _settingsManager.removeValue(credentialsKey);
      return;
    }
    if (storedCredentials.isNotEmpty) {
      AuthenticateMultipleServiceRequest(
        requests: storedCredentials,
      ).sendSignalToRust();
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
    String? encryptedData = await _settingsManager.getValue<String>(
      credentialsKey,
    );
    if (encryptedData == null) return [];

    String decryptedData = _decrypt(encryptedData);
    List<dynamic> decodedList = jsonDecode(decryptedData);
    return decodedList.map((item) => itemFromMap(item)).toList();
  }

  Future<void> retryLogin(String serviceId) async {
    // Send login request using the found credentials
    final stored = await _storedCredential(serviceId);
    final request = LoginRequestItem(
      serviceId: stored.serviceId,
      username: stored.username,
      secretInput: ScrobbleSecretInputSaved(
        credentialRef: stored.credentialRef!,
      ),
    );
    AuthenticateSingleServiceRequest(request: request).sendSignalToRust();

    // Wait for the response
    final rustSignal =
        await AuthenticateSingleServiceResponse.rustSignalStream.first;
    final response = rustSignal.message;

    // Handle the response
    final result = response.result;
    if (result is AuthenticateSingleServiceResultFailure) throw result;
  }

  Future<void> login(LoginRequestItem credentials) async {
    final storedCredentials = await _getStoredCredentials();
    final previous = storedCredentials.where(
      (item) => item.serviceId == credentials.serviceId,
    );
    final oldReference = previous.isEmpty ? null : previous.first.credentialRef;
    final loginRequest = LoginRequestItem(
      serviceId: credentials.serviceId,
      username: credentials.username,
      secretInput: switch (credentials.secretInput) {
        ScrobbleSecretInputPassword(:final password, :final apiCredentials) =>
          ScrobbleSecretInputPassword(
            password: password,
            apiCredentials: apiCredentials,
            replaces: oldReference,
          ),
        ScrobbleSecretInputSaved() => credentials.secretInput,
        _ => credentials.secretInput,
      },
    );
    AuthenticateSingleServiceRequest(request: loginRequest).sendSignalToRust();

    final rustSignal =
        await AuthenticateSingleServiceResponse.rustSignalStream.first;
    final response = rustSignal.message;

    final result = response.result;
    if (result is AuthenticateSingleServiceResultFailure) throw result;
    final reference =
        (result as AuthenticateSingleServiceResultSuccess).credentialRef;
    final replacement = LoginRequestItem(
      serviceId: credentials.serviceId,
      username: credentials.username,
      secretInput: ScrobbleSecretInputSaved(credentialRef: reference),
    );
    final existingIndex = storedCredentials.indexWhere(
      (item) => item.serviceId == credentials.serviceId,
    );
    if (existingIndex == -1) {
      storedCredentials.add(replacement);
    } else {
      storedCredentials[existingIndex] = replacement;
    }
    await _saveCredentials(storedCredentials);
  }

  Future<void> logout(String serviceId) async {
    // Send logout request to the backend
    List<LoginRequestItem> storedCredentials = await _getStoredCredentials();
    final matches = storedCredentials.where(
      (item) => item.serviceId == serviceId,
    );
    final stored = matches.isEmpty ? null : matches.first;
    final storedReference = stored?.credentialRef;
    if (storedReference == null) return;
    LogoutSingleServiceRequest(
      serviceId: serviceId,
      credentialRef: storedReference,
    ).sendSignalToRust();
    storedCredentials.removeWhere((item) => item.serviceId == serviceId);
    await _saveCredentials(storedCredentials);
  }

  Future<void> _handleStatusUpdate(
    RustSignalPack<ScrobbleServiceStatusUpdated> signal,
  ) async {
    List<LoginRequestItem> storedCredentials = await _getStoredCredentials();
    Set<String> credentialServiceIds = storedCredentials
        .where((item) => item.hasCredentials)
        .map((c) => c.serviceId)
        .toSet();

    _serviceStatuses = signal.message.services
        .map(
          (status) => ServiceStatus(
            serviceId: status.serviceId,
            isAvailable: status.isAvailable,
            hasCredentials: credentialServiceIds.contains(status.serviceId),
            error: status.error ?? "",
          ),
        )
        .toList();

    notifyListeners();
  }

  Future<LoginRequestItem> _storedCredential(String serviceId) async {
    final storedCredentials = await _getStoredCredentials();
    return storedCredentials.firstWhere(
      (item) => item.serviceId == serviceId && item.credentialRef != null,
      orElse: () =>
          throw Exception('No credentials found for serviceId: $serviceId'),
    );
  }

  Future<void> _saveCredentials(List<LoginRequestItem> credentials) async {
    final encryptedData = _encrypt(
      jsonEncode(credentials.map((item) => item.toMap()).toList()),
    );
    await _settingsManager.setValue(credentialsKey, encryptedData);
  }

  String _encrypt(String data) {
    final key = utf8.encode(_encryptionKeyValue.substring(0, 16));
    final iv = Uint8List(16);

    final pc.KeyParameter keyParam = pc.KeyParameter(Uint8List.fromList(key));
    final pc.ParametersWithIV<pc.KeyParameter> params = pc.ParametersWithIV(
      keyParam,
      iv,
    );

    final pc.PaddedBlockCipherParameters<
      pc.CipherParameters,
      pc.CipherParameters
    >
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
    final pc.ParametersWithIV<pc.KeyParameter> params = pc.ParametersWithIV(
      keyParam,
      iv,
    );

    final pc.PaddedBlockCipherParameters<
      pc.CipherParameters,
      pc.CipherParameters
    >
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
