import 'dart:async';
import 'dart:convert';

import 'package:fluent_ui/fluent_ui.dart';

import '../utils/settings_manager.dart';
import '../bindings/bindings.dart';

class LicenseProvider with ChangeNotifier {
  static const licenseKey = 'license';
  static const licenseValidationKey = 'licenseValidation';

  final SettingsManager _settingsManager = SettingsManager();
  bool _isStoreMode = false;
  bool _isPro = false;

  Completer<void> initialized = Completer();

  LicenseProvider() {
    _loadCachedLicense().then((_) => _verifyLicense());
  }

  bool get isStoreMode => _isStoreMode;
  bool get isPro => _isPro;

  Future<void> _loadCachedLicense() async {
    final cachedResult =
        await _settingsManager.getValue<String?>(licenseValidationKey);

    if (cachedResult != null) {
      final decodedResult = jsonDecode(cachedResult) as Map<String, dynamic>;
      _isPro = decodedResult['isPro'] ?? false;
      _isStoreMode = decodedResult['isStoreMode'] ?? false;
      notifyListeners();
    }

    initialized.complete();
  }

  Future<void> _verifyLicense() async {
    final licenseData = await _settingsManager.getValue<String?>(licenseKey);
    final newLicenseResult = await _fetchLicenseFromApi(licenseData);
    if (newLicenseResult != null) {
      _isPro = newLicenseResult.isPro;
      _isStoreMode = newLicenseResult.isStoreMode;
      final encodedResult = jsonEncode({
        'isPro': _isPro,
        'isStoreMode': _isStoreMode,
      });
      await _settingsManager.setValue(licenseValidationKey, encodedResult);
      notifyListeners();
    }
  }

  Future<void> revalidateLicense() async {
    await _verifyLicense();
  }

  Future<ValidateLicenseResponse?> _fetchLicenseFromApi(String? license) async {
    ValidateLicenseRequest(license: license).sendSignalToRust();
    return (await ValidateLicenseResponse.rustSignalStream.first).message;
  }
}
