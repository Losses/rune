import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../utils/settings_manager.dart';
import '../messages/all.dart';

const licenseKey = 'license';
const licenseValidationKey = 'licenseValidation';

class LicenseProvider with ChangeNotifier {
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
    final cachedResult = await _settingsManager
        .getValue<Map<String, bool>?>(licenseValidationKey);

    if (cachedResult != null) {
      _isPro = cachedResult['isPro'] ?? false;
      _isStoreMode = cachedResult['isStoreMode'] ?? false;
      notifyListeners();
    }

    initialized.complete();
  }

  Future<void> _verifyLicense() async {
    final licenseData = await _settingsManager.getValue<String?>(licenseKey);
    if (licenseData != null) {
      final newLicenseResult = await _fetchLicenseFromApi(licenseData);
      if (newLicenseResult != null) {
        _isPro = newLicenseResult.isPro;
        _isStoreMode = newLicenseResult.isStoreMode;
        await _settingsManager.setValue(licenseValidationKey, {
          'isPro': _isPro,
          'isStoreMode': _isStoreMode,
        });
        notifyListeners();
      }
    }
  }

  Future<void> revalidateLicense() async {
    await _verifyLicense();
  }

  Future<ValidateLibraryResponse?> _fetchLicenseFromApi(String license) async {
    ValidateLicenseRequest(license: license).sendSignalToRust();
    return (await ValidateLibraryResponse.rustSignalStream.first).message;
  }
}
